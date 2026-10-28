//! Host identities are selected again at each real TLS boundary.
#![cfg(test)]

use imposter_core::model::{Options, RequestTemplate};
use imposter_core::prepare::prepare;
use imposter_core::settings::{AppSettings, HostIdentity};
use imposter_http::{Policy, send};
use imposter_testkit::{OK_RESPONSE, Pki, read_request};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

async fn server(
    pki: &Pki,
    response: Vec<u8>,
) -> (String, tokio::task::JoinHandle<std::io::Result<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(pki.server_config(true));
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut stream = acceptor.accept(stream).await?;
        read_request(&mut stream).await;
        stream.write_all(&response).await?;
        stream.shutdown().await
    });
    (url, task)
}

fn identity(pki: &Pki, url: &str) -> HostIdentity {
    HostIdentity {
        host: "127.0.0.1".into(),
        port: reqwest::Url::parse(url).unwrap().port().unwrap(),
        certificate_path: pki.client_cert_pem.display().to_string(),
        key_file: pki.client_key_pem.display().to_string(),
        password: String::new(),
    }
}

#[tokio::test]
async fn redirects_switch_identities_and_resolve_encrypted_keys_from_the_snapshot() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let first = Pki::generate(a.path());
    let second = Pki::generate(b.path());
    let encrypted = b.path().join("encrypted.pem");
    second.write_encrypted_client_key(&encrypted, "secret");
    let (next, second_task) = server(&second, OK_RESPONSE.to_vec()).await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {next}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (start, first_task) = server(&first, redirect.into_bytes()).await;
    let mut second_identity = identity(&second, &next);
    second_identity.key_file = "{{key}}".into();
    second_identity.password = "{{password}}".into();
    let settings = AppSettings {
        verify_ssl: false,
        host_identities: vec![identity(&first, &start), second_identity],
        ..Default::default()
    };
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing.as_mut().unwrap().variables = [
        ("key".into(), encrypted.display().to_string()),
        ("password".into(), "secret".into()),
    ]
    .into_iter()
    .collect();
    let prepared = prepare(
        &RequestTemplate {
            url: start,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    let response = send(&prepared, &policy, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.body, b"ok");
    first_task.await.unwrap().unwrap();
    second_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn redirect_to_an_unmatched_port_does_not_inherit_the_previous_identity() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (next, second_task) = server(&pki, OK_RESPONSE.to_vec()).await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {next}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (start, first_task) = server(&pki, redirect.into_bytes()).await;
    let settings = AppSettings {
        verify_ssl: false,
        host_identities: vec![identity(&pki, &start)],
        ..Default::default()
    };
    let prepared = prepare(
        &RequestTemplate {
            url: start,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert!(
        send(
            &prepared,
            &Policy::from_settings(&settings),
            &CancellationToken::new()
        )
        .await
        .is_err()
    );
    first_task.await.unwrap().unwrap();
    assert!(second_task.await.unwrap().is_err());
}

#[tokio::test]
async fn explicit_request_credentials_override_a_host_entry() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, task) = server(&pki, OK_RESPONSE.to_vec()).await;
    let mut wrong = identity(&pki, &url);
    wrong.certificate_path = "missing-certificate".into();
    let settings = AppSettings {
        verify_ssl: false,
        host_identities: vec![wrong],
        ..Default::default()
    };
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing.as_mut().unwrap().options = Options {
        certificate_path: Some(pki.client_cert_pem.display().to_string()),
        ..Default::default()
    };
    let prepared = prepare(
        &RequestTemplate {
            url,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert_eq!(
        send(&prepared, &policy, &CancellationToken::new())
            .await
            .unwrap()
            .status,
        200
    );
    task.await.unwrap().unwrap();
}
