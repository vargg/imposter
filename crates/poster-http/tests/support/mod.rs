//! Loopback servers and test certificates. Every server binds an ephemeral port and serves a
//! fixed number of connections, returning the raw request bytes it received.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// Read one HTTP/1.1 request: the head plus a `Content-Length` or chunked body.
pub async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        if let Some(end) = find(&data, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let chunked = head
                .lines()
                .any(|line| line.starts_with("transfer-encoding:") && line.contains("chunked"));
            let body = &data[end + 4..];
            let complete = if chunked {
                body == b"0\r\n\r\n" || body.ends_with(b"\r\n0\r\n\r\n")
            } else {
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map_or(0, |value| value.trim().parse::<usize>().unwrap());
                body.len() >= length
            };
            if complete {
                return data;
            }
        }
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            return data;
        }
        data.extend_from_slice(&buffer[..read]);
    }
}

/// A received request split into its parts, with a chunked body decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    pub fn parse(raw: &[u8]) -> Self {
        let end = find(raw, b"\r\n\r\n").expect("a complete request head");
        let head = String::from_utf8_lossy(&raw[..end]).into_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap().to_owned();
        let headers: Vec<(String, String)> = lines
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_owned(), value.trim_start().to_owned())
            })
            .collect();
        let raw_body = &raw[end + 4..];
        let chunked = headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
        let body = if chunked {
            dechunk(raw_body)
        } else {
            raw_body.to_vec()
        };
        Self {
            request_line,
            headers,
            body,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn method(&self) -> &str {
        self.request_line.split(' ').next().unwrap()
    }

    pub fn target(&self) -> &str {
        self.request_line.split(' ').nth(1).unwrap()
    }
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line_end = find(data, b"\r\n").unwrap();
        let size =
            usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap().trim(), 16)
                .unwrap();
        data = &data[line_end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

/// A server answering every connection with `respond(index, request)`, recording requests.
pub struct Server {
    pub url: String,
    pub port: u16,
    requests: Arc<Mutex<Vec<Received>>>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn start(
        respond: impl Fn(usize, &Received) -> Vec<u8> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            let respond = Arc::new(respond);
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let raw = read_request(&mut stream).await;
                if raw.is_empty() {
                    continue;
                }
                let received = Received::parse(&raw);
                let index = {
                    let mut all = recorded.lock().unwrap();
                    all.push(received.clone());
                    all.len() - 1
                };
                let response = respond(index, &received);
                stream.write_all(&response).await.ok();
                stream.shutdown().await.ok();
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            port,
            requests,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Received> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A complete HTTP/1.1 response with `Content-Length` and `Connection: close`.
pub fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        let _ = write!(out, "{name}: {value}\r\n");
    }
    let _ = write!(
        out,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
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
