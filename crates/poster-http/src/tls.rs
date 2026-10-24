//! TLS client configuration: trust roots, verification policy and client certificates.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pkcs8::der::pem::PemLabel;
use pkcs8::{EncryptedPrivateKeyInfoRef, SecretDocument};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};

/// What the TLS layer needs from the effective settings. Empty paths mean "not set".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsSettings {
    pub verify: bool,
    pub ca_bundle: Option<PathBuf>,
    pub certificate: Option<PathBuf>,
    pub key: Option<PathBuf>,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("Cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} contains no PEM certificates")]
    NoCertificate { path: PathBuf },
    #[error("{path} contains no private key")]
    NoPrivateKey { path: PathBuf },
    #[error("{path} is not valid PEM: {detail}")]
    InvalidPem { path: PathBuf, detail: String },
    #[error("A private key requires a client certificate")]
    KeyWithoutCertificate,
    #[error("The private key is encrypted; enter its password")]
    PasswordRequired,
    #[error("Cannot decrypt the private key; check its password")]
    WrongPassword,
    #[error("The private key uses an unsupported encryption algorithm ({0})")]
    UnsupportedKeyEncryption(String),
    #[error(
        "Legacy encrypted PEM keys are not supported; convert the key with \
         `openssl pkcs8 -topk8 -in old.key -out new.key`"
    )]
    LegacyEncryptedKey,
    #[error("Invalid CA certificate in {path}: {source}")]
    InvalidCa {
        path: PathBuf,
        #[source]
        source: rustls::Error,
    },
    #[error("TLS configuration failed: {0}")]
    Config(#[from] rustls::Error),
}

/// The process-wide crypto provider. `ring` keeps the build free of cmake and system libraries.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Build a client configuration. Reads files, so call it off the UI thread.
///
/// A custom CA bundle replaces the platform roots (as Python's `create_default_context(cafile=…)`
/// does). Turning verification off skips both chain and hostname checks.
pub fn client_config(settings: &TlsSettings) -> Result<ClientConfig, TlsError> {
    let provider = provider();
    let verifier: Arc<dyn ServerCertVerifier> = if !settings.verify {
        Arc::new(NoVerification(provider.signature_verification_algorithms))
    } else if let Some(path) = &settings.ca_bundle {
        let mut roots = RootCertStore::empty();
        for cert in read_certificates(path)? {
            roots.add(cert).map_err(|source| TlsError::InvalidCa {
                path: path.clone(),
                source,
            })?;
        }
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|error| TlsError::Config(rustls::Error::General(error.to_string())))?
    } else {
        Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?)
    };
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let mut config = match (&settings.certificate, &settings.key) {
        (Some(certificate), key) => {
            let chain = read_certificates(certificate)?;
            let key = read_private_key(key.as_deref().unwrap_or(certificate), &settings.password)?;
            builder.with_client_auth_cert(chain, key)?
        }
        (None, Some(_)) => return Err(TlsError::KeyWithoutCertificate),
        (None, None) => builder.with_no_client_auth(),
    };
    // HTTP/1.1 only: the wire format (header order, framing) is what tests pin down.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|source| TlsError::Read {
        path: path.to_owned(),
        source,
    })
}

fn invalid_pem(path: &Path, error: impl std::fmt::Display) -> TlsError {
    TlsError::InvalidPem {
        path: path.to_owned(),
        detail: error.to_string(),
    }
}

fn read_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let data = read(path)?;
    let certs = CertificateDer::pem_slice_iter(&data)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| invalid_pem(path, error))?;
    if certs.is_empty() {
        return Err(TlsError::NoCertificate {
            path: path.to_owned(),
        });
    }
    Ok(certs)
}

/// Load the first private key in `path`: PKCS#8 (plain or encrypted), PKCS#1 or SEC1.
pub fn read_private_key(path: &Path, password: &str) -> Result<PrivateKeyDer<'static>, TlsError> {
    let data = read(path)?;
    parse_private_key(&data, password).map_err(|error| match error {
        KeyError::Missing => TlsError::NoPrivateKey {
            path: path.to_owned(),
        },
        KeyError::Pem(detail) => TlsError::InvalidPem {
            path: path.to_owned(),
            detail,
        },
        KeyError::Tls(error) => error,
    })
}

enum KeyError {
    Missing,
    Pem(String),
    Tls(TlsError),
}

fn parse_private_key(data: &[u8], password: &str) -> Result<PrivateKeyDer<'static>, KeyError> {
    let text = String::from_utf8_lossy(data);
    if text.contains("Proc-Type: 4,ENCRYPTED") {
        return Err(KeyError::Tls(TlsError::LegacyEncryptedKey));
    }
    if let Some(block) = pem_block(&text, EncryptedPrivateKeyInfoRef::PEM_LABEL) {
        if password.is_empty() {
            return Err(KeyError::Tls(TlsError::PasswordRequired));
        }
        let (_, der) =
            SecretDocument::from_pem(block).map_err(|error| KeyError::Pem(error.to_string()))?;
        let info = EncryptedPrivateKeyInfoRef::try_from(der.as_bytes())
            .map_err(|error| KeyError::Pem(error.to_string()))?;
        let plain = info.decrypt(password).map_err(|error| match error {
            pkcs8::Error::EncryptedPrivateKey(pkcs5::Error::DecryptFailed) => {
                KeyError::Tls(TlsError::WrongPassword)
            }
            pkcs8::Error::EncryptedPrivateKey(
                pkcs5::Error::UnsupportedAlgorithm { oid }
                | pkcs5::Error::AlgorithmParametersInvalid { oid },
            ) => KeyError::Tls(TlsError::UnsupportedKeyEncryption(oid.to_string())),
            pkcs8::Error::EncryptedPrivateKey(pkcs5::Error::NoPbes1CryptSupport) => {
                KeyError::Tls(TlsError::UnsupportedKeyEncryption("PBES1".to_owned()))
            }
            other => KeyError::Pem(other.to_string()),
        })?;
        return Ok(PrivatePkcs8KeyDer::from(plain.as_bytes().to_vec()).into());
    }
    match PrivateKeyDer::from_pem_slice(data) {
        Ok(key) => Ok(key),
        Err(rustls_pki_types::pem::Error::NoItemsFound) => Err(KeyError::Missing),
        Err(error) => Err(KeyError::Pem(error.to_string())),
    }
}

/// The text of the first `-----BEGIN {label}-----` … `-----END {label}-----` block.
fn pem_block<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = text.find(&begin)?;
    let stop = text[start..].find(&end)? + start + end.len();
    Some(&text[start..stop])
}

/// Accepts any server certificate. Used only when the user turns verification off.
#[derive(Debug)]
struct NoVerification(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}
