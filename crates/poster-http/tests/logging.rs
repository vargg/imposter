//! Logs never contain credentials or bodies, even at trace level and including dependencies'
//! `log` records (the app's subscriber bridges them).
#![cfg(test)]

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poster_core::model::Credentials;
use poster_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use poster_http::tls::TlsSettings;
use poster_http::{Policy, send};
use support::{Server, response};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const SECRETS: [&str; 5] = [
    "bearer-secret-token",
    "body-secret-value",
    "url-secret-password",
    "digest-secret-pw",
    "header-secret",
];

#[tokio::test(flavor = "multi_thread")]
async fn nothing_secret_reaches_the_logs() {
    let capture = Capture::default();
    let writer = capture.clone();
    tracing_subscriber::fmt()
        .with_env_filter("trace")
        .with_writer(move || writer.clone())
        .try_init()
        .unwrap();

    let server = Server::start(|index, received| {
        if received.header("authorization").is_none()
            && index % 2 == 0
            && received.target() == "/digest"
        {
            response(
                "401 Unauthorized",
                &[(
                    "WWW-Authenticate",
                    r#"Digest realm="r", nonce="n", qop="auth""#,
                )],
                b"",
            )
        } else {
            response(
                "200 OK",
                &[("Set-Cookie", "session=header-secret")],
                b"response body-secret-value",
            )
        }
    })
    .await;
    let host = server.url.trim_start_matches("http://");
    let policy = Policy {
        timeout: Duration::from_secs(5),
        follow_redirects: true,
        proxy: None,
        tls: TlsSettings {
            verify: true,
            ..TlsSettings::default()
        },
        capture_limit: None,
    };
    let requests = [
        Prepared {
            method: "POST".into(),
            url: format!("http://user:url-secret-password@{host}/plain"),
            headers: vec![
                ("Authorization".into(), "Bearer bearer-secret-token".into()),
                ("X-Api-Key".into(), "header-secret".into()),
            ],
            body: PreparedBody::Bytes(b"body-secret-value".to_vec()),
            auth: PreparedAuth::None,
        },
        Prepared {
            method: "GET".into(),
            url: format!("{}/digest", server.url),
            headers: Vec::new(),
            body: PreparedBody::Bytes(Vec::new()),
            auth: PreparedAuth::Digest(Credentials {
                username: "u".into(),
                password: "digest-secret-pw".into(),
            }),
        },
    ];
    for prepared in &requests {
        send(prepared, &policy, &CancellationToken::new())
            .await
            .unwrap();
    }

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        !logs.is_empty(),
        "the subscriber captured nothing, so this test would prove nothing"
    );
    for secret in SECRETS {
        assert!(
            !logs.contains(secret),
            "{secret} leaked into the logs:\n{logs}"
        );
    }
}
