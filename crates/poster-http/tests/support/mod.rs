//! Loopback servers and test certificates. Every server binds an ephemeral port and serves a
//! fixed number of connections, returning the raw request bytes it received.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub const OK_RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

/// A CA plus a server certificate for `localhost` and a client certificate, written to `dir`.
pub struct Pki {
    pub ca_pem: PathBuf,
    pub server_chain: Vec<CertificateDer<'static>>,
    pub server_key: PrivateKeyDer<'static>,
    pub client_ca: RootCertStore,
    pub client_cert_pem: PathBuf,
    /// Unencrypted PKCS#8 client key.
    pub client_key_pem: PathBuf,
    pub client_key_der: Vec<u8>,
}

impl Pki {
    pub fn generate(dir: &Path) -> Self {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();

        let server_key = KeyPair::generate().unwrap();
        let server = CertificateParams::new(vec!["localhost".to_owned()])
            .unwrap()
            .signed_by(&server_key, &ca)
            .unwrap();

        let client_key = KeyPair::generate().unwrap();
        let client = CertificateParams::new(vec!["client".to_owned()])
            .unwrap()
            .signed_by(&client_key, &ca)
            .unwrap();

        let ca_pem = dir.join("ca.pem");
        std::fs::write(&ca_pem, ca.pem()).unwrap();
        let client_cert_pem = dir.join("client.pem");
        std::fs::write(&client_cert_pem, client.pem()).unwrap();
        let client_key_pem = dir.join("client.key");
        std::fs::write(&client_key_pem, client_key.serialize_pem()).unwrap();

        let mut client_ca = RootCertStore::empty();
        client_ca.add(ca.der().clone()).unwrap();
        Self {
            ca_pem,
            server_chain: vec![server.der().clone(), ca.der().clone()],
            server_key: PrivateKeyDer::from_pem_slice(server_key.serialize_pem().as_bytes())
                .unwrap(),
            client_ca,
            client_cert_pem,
            client_key_pem,
            client_key_der: client_key.serialize_der(),
        }
    }

    /// Write the client key as an encrypted PKCS#8 PEM file.
    pub fn write_encrypted_client_key(&self, path: &Path, password: &str) {
        let info = pkcs8::PrivateKeyInfoRef::try_from(self.client_key_der.as_slice()).unwrap();
        let encrypted = info.encrypt(password).unwrap();
        let pem = encrypted
            .to_pem("ENCRYPTED PRIVATE KEY", pkcs8::der::pem::LineEnding::LF)
            .unwrap();
        std::fs::write(path, pem.as_bytes()).unwrap();
    }

    /// TLS server config that requires a client certificate signed by the test CA.
    pub fn server_config(&self, require_client_cert: bool) -> Arc<ServerConfig> {
        let provider = poster_http::tls::provider();
        let builder = ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap();
        let builder = if require_client_cert {
            let verifier = WebPkiClientVerifier::builder_with_provider(
                Arc::new(self.client_ca.clone()),
                provider,
            )
            .build()
            .unwrap();
            builder.with_client_cert_verifier(verifier)
        } else {
            builder.with_no_client_auth()
        };
        Arc::new(
            builder
                .with_single_cert(self.server_chain.clone(), self.server_key.clone_key())
                .unwrap(),
        )
    }
}

/// Read one HTTP/1.1 request: the head plus a `Content-Length` body.
pub async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        if let Some(end) = find(&data, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map_or(0, |value| value.trim().parse::<usize>().unwrap());
            if data.len() >= end + 4 + length {
                return data;
            }
        }
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            return data;
        }
        data.extend_from_slice(&buffer[..read]);
    }
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn answer(mut stream: impl AsyncRead + AsyncWrite + Unpin, response: &[u8]) -> Vec<u8> {
    let request = read_request(&mut stream).await;
    stream.write_all(response).await.unwrap();
    stream.shutdown().await.ok();
    request
}

/// Plain HTTP server answering one request with `response`.
pub async fn http_once(response: &'static [u8]) -> (String, JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        answer(stream, response).await
    });
    (url, task)
}

/// HTTPS server for `localhost` answering one request; the handshake result is returned too.
pub async fn https_once(
    config: Arc<ServerConfig>,
    response: &'static [u8],
) -> (String, JoinHandle<Result<Vec<u8>, std::io::Error>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await?;
        Ok(answer(stream, response).await)
    });
    (url, task)
}
