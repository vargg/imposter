//! TLS policy against loopback servers using generated certificates.
#![cfg(test)]

mod support;

use std::net::SocketAddr;
use std::path::Path;

use poster_http::tls::{TlsError, TlsSettings, client_config, read_private_key};
use support::{OK_RESPONSE, Pki, https_once};

fn client(settings: &TlsSettings, url: &str) -> reqwest::Client {
    let port = url.rsplit(':').next().unwrap().parse::<u16>().unwrap();
    reqwest::Client::builder()
        .use_preconfigured_tls(client_config(settings).unwrap())
        .resolve("localhost", SocketAddr::from(([127, 0, 0, 1], port)))
        .no_proxy()
        .build()
        .unwrap()
}

fn trusting(pki: &Pki) -> TlsSettings {
    TlsSettings {
        verify: true,
        ca_bundle: Some(pki.ca_pem.clone()),
        ..TlsSettings::default()
    }
}

#[tokio::test]
async fn custom_ca_verifies_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, server) = https_once(pki.server_config(false), OK_RESPONSE).await;

    let response = client(&trusting(&pki), &url)
        .get(&url)
        .send()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "ok");
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn custom_ca_replaces_platform_roots_so_unknown_servers_fail() {
    let dir = tempfile::tempdir().unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let other = Pki::generate(other_dir.path());
    let (url, _server) = https_once(pki.server_config(false), OK_RESPONSE).await;

    let error = client(&trusting(&other), &url)
        .get(&url)
        .send()
        .await
        .unwrap_err();

    assert!(error.is_connect(), "{error:?}");
}

#[tokio::test]
async fn disabled_verification_accepts_an_untrusted_server() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, server) = https_once(pki.server_config(false), OK_RESPONSE).await;
    let settings = TlsSettings {
        verify: false,
        ..TlsSettings::default()
    };

    let response = client(&settings, &url).get(&url).send().await.unwrap();

    assert_eq!(response.status(), 200);
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn encrypted_pkcs8_client_key_authenticates() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let key = dir.path().join("client-encrypted.key");
    pki.write_encrypted_client_key(&key, "s3cret");
    let (url, server) = https_once(pki.server_config(true), OK_RESPONSE).await;
    let settings = TlsSettings {
        certificate: Some(pki.client_cert_pem.clone()),
        key: Some(key),
        password: "s3cret".to_owned(),
        ..trusting(&pki)
    };

    let response = client(&settings, &url).get(&url).send().await.unwrap();

    assert_eq!(response.text().await.unwrap(), "ok");
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn server_requiring_a_client_certificate_rejects_anonymous_clients() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, server) = https_once(pki.server_config(true), OK_RESPONSE).await;

    let result = client(&trusting(&pki), &url).get(&url).send().await;

    assert!(result.is_err());
    assert!(server.await.unwrap().is_err());
}

#[test]
fn certificate_file_may_contain_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let combined = dir.path().join("combined.pem");
    let mut data = std::fs::read(&pki.client_cert_pem).unwrap();
    data.extend(std::fs::read(&pki.client_key_pem).unwrap());
    std::fs::write(&combined, data).unwrap();
    let settings = TlsSettings {
        certificate: Some(combined),
        ..trusting(&pki)
    };

    client_config(&settings).unwrap();
}

#[test]
fn encrypted_key_errors_are_specific() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let key = dir.path().join("client-encrypted.key");
    pki.write_encrypted_client_key(&key, "right");
    let settings = |password: &str| TlsSettings {
        certificate: Some(pki.client_cert_pem.clone()),
        key: Some(key.clone()),
        password: password.to_owned(),
        ..trusting(&pki)
    };

    assert!(matches!(
        client_config(&settings("")),
        Err(TlsError::PasswordRequired)
    ));
    assert!(matches!(
        client_config(&settings("wrong")),
        Err(TlsError::WrongPassword)
    ));
}

#[test]
fn legacy_encrypted_pem_is_rejected_with_conversion_advice() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let key = dir.path().join("legacy.key");
    std::fs::write(
        &key,
        "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00\n\nAAAA\n-----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();
    let settings = TlsSettings {
        certificate: Some(pki.client_cert_pem.clone()),
        key: Some(key),
        password: "x".to_owned(),
        ..trusting(&pki)
    };

    let error = client_config(&settings).unwrap_err();

    assert!(matches!(error, TlsError::LegacyEncryptedKey));
    assert!(error.to_string().contains("openssl pkcs8 -topk8"));
}

#[test]
fn key_without_certificate_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let settings = TlsSettings {
        key: Some(pki.client_key_pem.clone()),
        ..trusting(&pki)
    };

    assert!(matches!(
        client_config(&settings),
        Err(TlsError::KeyWithoutCertificate)
    ));
}

#[test]
fn missing_and_empty_files_are_reported_with_their_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.pem");
    let empty = dir.path().join("empty.pem");
    std::fs::write(&empty, "").unwrap();

    let read = client_config(&TlsSettings {
        verify: true,
        ca_bundle: Some(missing.clone()),
        ..TlsSettings::default()
    });
    let none = client_config(&TlsSettings {
        verify: true,
        ca_bundle: Some(empty.clone()),
        ..TlsSettings::default()
    });

    assert!(matches!(read, Err(TlsError::Read { path, .. }) if path == missing));
    assert!(matches!(none, Err(TlsError::NoCertificate { path }) if path == empty));
}

/// Fixtures made with real tools (`openssl pkcs8 -topk8 -v2 aes-256-cbc` from `LibreSSL` 3.3, and
/// OpenSSL 3 with `-v2prf hmacWithSHA256`), password `fixture-password`.
#[test]
fn openssl_encrypted_keys_decrypt_to_the_plain_key() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let plain = read_private_key(&fixtures.join("plain-pkcs8.key"), "").unwrap();
    for name in [
        "libressl-pbkdf2-sha1-aes256.key",
        "openssl3-pbkdf2-sha256-aes256.key",
    ] {
        let key = read_private_key(&fixtures.join(name), "fixture-password").unwrap();
        assert_eq!(key.secret_der(), plain.secret_der(), "{name}");
    }
}
