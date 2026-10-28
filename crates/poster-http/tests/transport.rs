//! The transport against loopback servers, checking the bytes that actually arrive.
#![cfg(test)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use md5::{Digest as _, Md5};
use poster_core::model::{Credentials, FormItem};
use poster_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use poster_http::tls::TlsSettings;
use poster_http::transport::collect;
use poster_http::upload::multipart_stream;
use poster_http::{ErrorKind, Policy, SendError, send};
use poster_testkit::{Pki, Received, Server, response};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
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

fn request(method: &str, url: &str) -> Prepared {
    Prepared {
        method: method.to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: PreparedBody::Bytes(Vec::new()),
        auth: PreparedAuth::None,
    }
}

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

fn uploads() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/uploads")
        .canonicalize()
        .unwrap()
}

async fn ok(prepared: &Prepared, policy: &Policy) -> poster_core::execution::Response {
    send(prepared, policy, &CancellationToken::new())
        .await
        .unwrap()
}

async fn failure(prepared: &Prepared, policy: &Policy) -> (ErrorKind, String) {
    match send(prepared, policy, &CancellationToken::new()).await {
        Err(SendError::Failed { kind, message }) => (kind, message),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[tokio::test]
async fn the_wire_carries_method_target_ordered_headers_and_body() {
    let server = Server::start(|_, _| response("200 OK", &[], b"ok")).await;
    let mut prepared = request("PROPFIND", &format!("{}/p?a=1&a=2", server.url));
    prepared.headers = headers(&[
        ("X-A", "1"),
        ("X-B", "2"),
        ("X-A", "3"),
        ("Empty", ""),
        ("Content-Type", "application/json"),
    ]);
    prepared.body = PreparedBody::Bytes(br#"{"k": 1}"#.to_vec());

    ok(&prepared, &policy()).await;

    let received = server.requests().remove(0);
    assert_eq!(received.request_line, "PROPFIND /p?a=1&a=2 HTTP/1.1");
    let ours: Vec<_> = received
        .headers
        .iter()
        .filter(|(n, _)| n.starts_with("x-") || n == "empty")
        .cloned()
        .collect();
    assert_eq!(
        ours,
        headers(&[("x-a", "1"), ("x-a", "3"), ("x-b", "2"), ("empty", "")])
    );
    assert_eq!(received.header("content-type"), Some("application/json"));
    assert_eq!(
        received.header("user-agent"),
        Some(concat!("Poster/", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(received.body, br#"{"k": 1}"#);
}

#[tokio::test]
async fn a_user_agent_header_replaces_the_default() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let mut prepared = request("GET", &server.url);
    prepared.headers = headers(&[("User-Agent", "custom/1")]);

    ok(&prepared, &policy()).await;

    let received = server.requests().remove(0);
    let agents: Vec<_> = received
        .headers
        .iter()
        .filter(|(n, _)| n == "user-agent")
        .collect();
    assert_eq!(agents, [&("user-agent".to_owned(), "custom/1".to_owned())]);
}

#[tokio::test]
async fn response_status_headers_and_body_are_captured() {
    let server = Server::start(|_, _| {
        b"HTTP/1.1 201 Created\r\nSet-Cookie: a=1\r\nX-Other: x\r\nSet-Cookie: b=2\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    })
    .await;

    let captured = ok(&request("GET", &server.url), &policy()).await;

    assert_eq!(
        (captured.status, captured.reason.as_str()),
        (201, "Created")
    );
    let cookies: Vec<_> = captured
        .headers
        .iter()
        .filter(|(n, _)| n == "set-cookie")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(cookies, ["a=1", "b=2"]);
    assert_eq!(captured.body, b"hello");
    assert!(!captured.truncated);
}

#[tokio::test]
async fn the_capture_limit_truncates_exactly_and_flags_it() {
    let body = vec![b'x'; 10 * 1024];
    let server = Server::start(move |_, _| response("200 OK", &[], &body)).await;
    let url = server.url.clone();

    let limited = ok(
        &request("GET", &url),
        &Policy {
            capture_limit: Some(4096),
            ..policy()
        },
    )
    .await;
    let exact = ok(
        &request("GET", &url),
        &Policy {
            capture_limit: Some(10 * 1024),
            ..policy()
        },
    )
    .await;
    let unlimited = ok(&request("GET", &url), &policy()).await;

    assert_eq!((limited.body.len(), limited.truncated), (4096, true));
    assert_eq!((exact.body.len(), exact.truncated), (10 * 1024, false));
    assert_eq!(
        (unlimited.body.len(), unlimited.truncated),
        (10 * 1024, false)
    );
}

#[tokio::test]
async fn compressed_responses_are_decoded() {
    const GZIP: &str = "1f8b08000000000002ff4bcecf2d284a2d2e4e4d51c848cdc9c90700b601994610000000";
    let bytes: Vec<u8> = (0..GZIP.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&GZIP[i..i + 2], 16).unwrap())
        .collect();
    let server =
        Server::start(move |_, _| response("200 OK", &[("Content-Encoding", "gzip")], &bytes))
            .await;

    let captured = ok(&request("GET", &server.url), &policy()).await;

    assert_eq!(captured.body, b"compressed hello");
}

#[tokio::test]
async fn multipart_bodies_match_the_python_app_byte_for_byte() {
    let cases: Vec<Value> = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/parity/multipart.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for case in cases.iter().filter(|case| case.get("fields").is_some()) {
        let name = case["name"].as_str().unwrap();
        let fields: Vec<FormItem> = serde_json::from_value::<Vec<FormItem>>(case["fields"].clone())
            .unwrap()
            .into_iter()
            .map(|field| match field.kind {
                poster_core::model::FormKind::File => FormItem {
                    value: uploads().join(&field.value).display().to_string(),
                    ..field
                },
                poster_core::model::FormKind::Text => field,
            })
            .collect();
        let expected = poster_core::base64_bytes::decode(case["body"].as_str().unwrap()).unwrap();

        let body = collect(multipart_stream(&fields, "BOUNDARY").unwrap())
            .await
            .unwrap();

        assert_eq!(
            String::from_utf8_lossy(&body),
            String::from_utf8_lossy(&expected),
            "{name}"
        );
        assert_eq!(body, expected, "{name}");
    }
}

#[tokio::test]
async fn multipart_uploads_stream_with_a_matching_boundary() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let mut prepared = request("POST", &server.url);
    prepared.headers = headers(&[("content-type", "text/plain"), ("X-Keep", "1")]);
    prepared.body = PreparedBody::Multipart(vec![
        FormItem::text("a", "1"),
        FormItem::file("blob", uploads().join("data.bin").display().to_string()),
    ]);

    ok(&prepared, &policy()).await;

    let received = server.requests().remove(0);
    let content_type = received.header("content-type").unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    assert_eq!(boundary.len(), 48);
    assert_eq!(
        received
            .headers
            .iter()
            .filter(|(n, _)| n == "content-type")
            .count(),
        1
    );
    let PreparedBody::Multipart(fields) = &prepared.body else {
        unreachable!()
    };
    assert_eq!(
        received.body,
        collect(multipart_stream(fields, boundary).unwrap())
            .await
            .unwrap()
    );
    assert_eq!(received.header("transfer-encoding"), Some("chunked"));
}

#[tokio::test]
async fn binary_bodies_stream_the_file_without_adding_a_content_type() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let mut prepared = request("PUT", &server.url);
    prepared.body = PreparedBody::File(uploads().join("data.bin").display().to_string());

    ok(&prepared, &policy()).await;

    let received = server.requests().remove(0);
    assert_eq!(
        received.body,
        std::fs::read(uploads().join("data.bin")).unwrap()
    );
    assert_eq!(received.header("content-type"), None);
}

#[tokio::test]
async fn missing_upload_files_fail_before_anything_is_sent() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let mut prepared = request("POST", &server.url);
    prepared.body = PreparedBody::Multipart(vec![FormItem::file("f", "/definitely/missing/file")]);

    let (kind, message) = failure(&prepared, &policy()).await;

    assert_eq!(kind, ErrorKind::Upload);
    assert!(message.contains("/definitely/missing/file"), "{message}");
    let mut directory = request("POST", &server.url);
    directory.body = PreparedBody::File(uploads().display().to_string());
    assert_eq!(failure(&directory, &policy()).await.0, ErrorKind::Upload);
    assert_eq!(server.requests(), []);
}

#[tokio::test]
async fn post_redirected_by_302_becomes_a_bodiless_get_without_cookies() {
    let server = Server::start(|index, _| match index {
        0 => response("302 Found", &[("Location", "/next?x=1")], b""),
        _ => response("200 OK", &[], b"done"),
    })
    .await;
    let mut prepared = request("POST", &format!("{}/start", server.url));
    prepared.headers = headers(&[("Cookie", "a=1"), ("Authorization", "Bearer t")]);
    prepared.body = PreparedBody::Bytes(b"payload".to_vec());

    let captured = ok(&prepared, &policy()).await;

    assert_eq!(captured.body, b"done");
    let [first, second] = server.requests().try_into().unwrap();
    assert_eq!(
        (first.method(), first.body.as_slice()),
        ("POST", b"payload".as_slice())
    );
    assert_eq!((second.method(), second.target()), ("GET", "/next?x=1"));
    assert_eq!(second.body, b"");
    assert_eq!(second.header("cookie"), None);
    assert_eq!(
        second.header("authorization"),
        Some("Bearer t"),
        "same origin keeps credentials"
    );
}

#[tokio::test]
async fn redirect_307_replays_a_streamed_upload_with_the_same_method() {
    let server = Server::start(|index, _| match index {
        0 => response("307 Temporary Redirect", &[("Location", "/again")], b""),
        _ => response("200 OK", &[], b""),
    })
    .await;
    let mut prepared = request("POST", &server.url);
    prepared.body = PreparedBody::Multipart(vec![FormItem::file(
        "blob",
        uploads().join("data.bin").display().to_string(),
    )]);

    ok(&prepared, &policy()).await;

    let [first, second] = server.requests().try_into().unwrap();
    assert_eq!(second.method(), "POST");
    assert_eq!(first.body, second.body);
    assert!(first.body.len() > 76_800);
}

#[tokio::test]
async fn cross_origin_redirects_drop_authorization() {
    let target = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let location = format!("http://localhost:{}/landing", target.port);
    let origin = Server::start(move |_, _| {
        response("301 Moved Permanently", &[("Location", &location)], b"")
    })
    .await;
    let mut prepared = request("GET", &origin.url);
    prepared.headers = headers(&[("Authorization", "Bearer secret"), ("X-Keep", "1")]);

    ok(&prepared, &policy()).await;

    let landed = target.requests().remove(0);
    assert_eq!(landed.header("authorization"), None);
    assert_eq!(landed.header("x-keep"), Some("1"));
}

#[tokio::test]
async fn redirects_are_returned_as_is_when_not_followed() {
    let server =
        Server::start(|_, _| response("302 Found", &[("Location", "/elsewhere")], b"")).await;

    let captured = ok(
        &request("GET", &server.url),
        &Policy {
            follow_redirects: false,
            ..policy()
        },
    )
    .await;

    assert_eq!(captured.status, 302);
    assert!(
        captured
            .headers
            .contains(&("location".to_owned(), "/elsewhere".to_owned()))
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn endless_redirects_stop_after_twenty() {
    let server = Server::start(|_, _| response("302 Found", &[("Location", "/loop")], b"")).await;

    let (kind, _) = failure(&request("GET", &server.url), &policy()).await;

    assert_eq!(kind, ErrorKind::TooManyRedirects);
    assert_eq!(server.requests().len(), 21);
}

fn md5_hex(text: &str) -> String {
    Md5::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Checks the client's Digest answer with an independent computation.
fn digest_server_response(index: usize, received: &Received) -> Vec<u8> {
    if index == 0 {
        return response(
            "401 Unauthorized",
            &[(
                "WWW-Authenticate",
                r#"Digest realm="poster", nonce="abc123", qop="auth", opaque="op""#,
            )],
            b"",
        );
    }
    let header = received.header("authorization").unwrap_or_default();
    let field = |name: &str| {
        let start = header.find(&format!("{name}=")).unwrap() + name.len() + 1;
        let rest = &header[start..];
        if let Some(quoted) = rest.strip_prefix('"') {
            quoted[..quoted.find('"').unwrap()].to_owned()
        } else {
            rest.split(',').next().unwrap().to_owned()
        }
    };
    let ha1 = md5_hex("alice:poster:wonderland");
    let ha2 = md5_hex(&format!("{}:{}", received.method(), received.target()));
    let expected = md5_hex(&format!(
        "{ha1}:abc123:{}:{}:auth:{ha2}",
        field("nc"),
        field("cnonce")
    ));
    let valid = field("response") == expected
        && field("uri") == received.target()
        && field("opaque") == "op";
    if valid {
        response("200 OK", &[], b"welcome")
    } else {
        response("403 Forbidden", &[], b"bad digest")
    }
}

#[tokio::test]
async fn digest_answers_the_challenge_and_replays_the_upload() {
    let server = Server::start(digest_server_response).await;
    let mut prepared = request("POST", &format!("{}/secure?q=1", server.url));
    prepared.auth = PreparedAuth::Digest(Credentials {
        username: "alice".into(),
        password: "wonderland".into(),
    });
    prepared.body = PreparedBody::File(uploads().join("hello.txt").display().to_string());

    let captured = ok(&prepared, &policy()).await;

    assert_eq!(
        (captured.status, captured.body.as_slice()),
        (200, b"welcome".as_slice())
    );
    let [first, second] = server.requests().try_into().unwrap();
    assert_eq!(first.header("authorization"), None);
    assert_eq!(first.body, second.body);
    assert_eq!(second.body, b"hello\nworld\n");
}

#[tokio::test]
async fn a_401_without_a_digest_challenge_is_returned() {
    let server = Server::start(|_, _| {
        response(
            "401 Unauthorized",
            &[("WWW-Authenticate", r#"Basic realm="x""#)],
            b"no",
        )
    })
    .await;
    let mut prepared = request("GET", &server.url);
    prepared.auth = PreparedAuth::Digest(Credentials {
        username: "u".into(),
        password: "p".into(),
    });

    let captured = ok(&prepared, &policy()).await;

    assert_eq!(captured.status, 401);
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn https_requests_use_the_tls_policy() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, server) =
        poster_testkit::https_once(pki.server_config(false), poster_testkit::OK_RESPONSE).await;
    let url = url.replace("localhost", "127.0.0.1");
    let tls = TlsSettings {
        verify: false,
        ..TlsSettings::default()
    };

    let captured = ok(&request("GET", &url), &Policy { tls, ..policy() }).await;

    assert_eq!(captured.body, b"ok");
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn untrusted_certificates_are_tls_errors() {
    let dir = tempfile::tempdir().unwrap();
    let pki = Pki::generate(dir.path());
    let (url, _server) =
        poster_testkit::https_once(pki.server_config(false), poster_testkit::OK_RESPONSE).await;

    let (kind, message) = failure(
        &request("GET", &url.replace("localhost", "127.0.0.1")),
        &policy(),
    )
    .await;

    assert_eq!(kind, ErrorKind::Tls, "{message}");
}

#[tokio::test]
async fn certificate_problems_are_reported_before_connecting() {
    let tls = TlsSettings {
        verify: true,
        ca_bundle: Some("/missing/ca.pem".into()),
        ..TlsSettings::default()
    };
    let (kind, message) = failure(
        &request("GET", "https://127.0.0.1:1/"),
        &Policy { tls, ..policy() },
    )
    .await;
    assert_eq!(kind, ErrorKind::Tls);
    assert!(message.contains("/missing/ca.pem"), "{message}");
}

#[tokio::test]
async fn refused_connections_are_connect_errors() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (kind, _) = failure(
        &request("GET", &format!("http://127.0.0.1:{port}/")),
        &policy(),
    )
    .await;
    assert_eq!(kind, ErrorKind::Connect);
}

#[tokio::test]
async fn a_silent_server_times_out() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    let _silent = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let started = Instant::now();

    let (kind, _) = failure(
        &request("GET", &url),
        &Policy {
            timeout: Duration::from_millis(300),
            ..policy()
        },
    )
    .await;

    assert_eq!(kind, ErrorKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_server_that_hangs_up_mid_response_is_an_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        poster_testkit::read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
            .await
            .unwrap();
    });

    let (kind, _) = failure(&request("GET", &url), &policy()).await;

    assert_eq!(kind, ErrorKind::Body);
}

#[tokio::test]
async fn cancelling_mid_stream_returns_at_once_and_closes_the_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    let (streaming_tx, streaming_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        poster_testkit::read_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\nfirst bytes")
            .await
            .unwrap();
        streaming_tx.send(()).unwrap();
        let mut buffer = [0_u8; 16];
        // EOF or a reset: either way the client went away.
        tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .unwrap()
            .unwrap_or(0)
    });
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        streaming_rx.await.unwrap();
        trigger.cancel();
    });
    let started = Instant::now();

    let result = send(&request("GET", &url), &policy(), &cancel).await;

    assert!(matches!(result, Err(SendError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(server.await.unwrap(), 0);
}

#[tokio::test]
async fn an_explicit_proxy_receives_the_absolute_form_request() {
    let proxy = Server::start(|_, _| response("200 OK", &[], b"via proxy")).await;
    let proxied = Policy {
        proxy: Some(proxy.url.clone()),
        ..policy()
    };

    let captured = ok(&request("GET", "http://target.invalid/path?q=1"), &proxied).await;

    assert_eq!(captured.body, b"via proxy");
    assert_eq!(
        proxy.requests().remove(0).request_line,
        "GET http://target.invalid/path?q=1 HTTP/1.1"
    );
}

#[tokio::test]
async fn invalid_proxies_urls_and_methods_are_reported() {
    let bad_proxy = Policy {
        proxy: Some("::not a url::".into()),
        ..policy()
    };
    assert_eq!(
        failure(&request("GET", "http://127.0.0.1:1/"), &bad_proxy)
            .await
            .0,
        ErrorKind::InvalidProxy
    );
    assert_eq!(
        failure(&request("GET", "http://[::1/"), &policy()).await.0,
        ErrorKind::InvalidUrl
    );
    assert_eq!(
        failure(&request("GE T", "http://127.0.0.1:1/"), &policy())
            .await
            .0,
        ErrorKind::Protocol
    );
}

#[tokio::test]
async fn error_messages_never_include_url_credentials() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (_, message) = failure(
        &request("GET", &format!("http://user:hunter2@127.0.0.1:{port}/")),
        &policy(),
    )
    .await;
    assert!(!message.contains("hunter2"), "{message}");
}
