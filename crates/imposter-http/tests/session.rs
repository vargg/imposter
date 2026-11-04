//! Sessions: a single send keeps no connection; a shared session reuses its connections.
#![cfg(test)]

use std::time::Duration;

use imposter_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use imposter_http::tls::TlsSettings;
use imposter_http::{Policy, Session, send};
use imposter_testkit::{LoadReply, LoadServer};
use tokio_util::sync::CancellationToken;

fn policy() -> Policy {
    Policy {
        timeout: Duration::from_secs(5),
        follow_redirects: true,
        proxy: None,
        tls: TlsSettings {
            verify: true,
            ..TlsSettings::default()
        },
        capture_limit: None,
        identity_routing: None,
    }
}

fn get(url: &str) -> Prepared {
    Prepared {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: PreparedBody::Bytes(Vec::new()),
        auth: PreparedAuth::None,
    }
}

#[tokio::test]
async fn single_sends_open_a_connection_each() {
    let server = LoadServer::start(|_, _| LoadReply::ok(b"ok")).await;
    for _ in 0..3 {
        let response = send(&get(&server.url), &policy(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(response.body, b"ok");
    }
    assert_eq!(server.connections(), 3);
}

#[tokio::test]
async fn a_session_with_idle_connections_reuses_one() {
    let server = LoadServer::start(|_, _| LoadReply::ok(b"ok")).await;
    let session = Session::new(policy(), 4);
    for _ in 0..3 {
        let response = session
            .send(&get(&server.url), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            (response.status, response.body.as_slice()),
            (200, &b"ok"[..])
        );
    }
    assert_eq!(server.connections(), 1);
    assert_eq!(server.requests().len(), 3);
}
