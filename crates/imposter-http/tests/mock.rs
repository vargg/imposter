//! Static responses over real loopback sockets, including shutdown of incomplete requests.
#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use imposter_core::mock::{MockConfig, MockLogEntry, MockReply, MockRoute, Responded};
use imposter_core::model::ParamItem;
use imposter_http::mock::{MockError, Responder, serve};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(5);

struct Server {
    address: SocketAddr,
    cancel: CancellationToken,
    task: JoinHandle<Result<(), MockError>>,
}

type Logged = Arc<Mutex<Vec<MockLogEntry>>>;

impl Server {
    async fn start(routes: Vec<MockRoute>) -> Self {
        Self::scripted(routes, None).await.0
    }

    /// A server whose scripted routes ask `responder`, and the log it writes.
    async fn scripted(routes: Vec<MockRoute>, responder: Option<Responder>) -> (Self, Logged) {
        let (tx, rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let logged = Logged::default();
        let log = Arc::clone(&logged);
        let task = tokio::spawn(serve(
            MockConfig { port: 0, routes },
            responder,
            move |address| {
                tx.send(address).unwrap();
            },
            Arc::new(move |entry| log.lock().unwrap().push(entry)),
            token,
        ));
        let address = tokio::time::timeout(DEADLINE, rx).await.unwrap().unwrap();
        (
            Self {
                address,
                cancel,
                task,
            },
            logged,
        )
    }

    async fn exchange(&self, request: &str) -> String {
        tokio::time::timeout(DEADLINE, async {
            let mut stream = TcpStream::connect(self.address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut result = String::new();
            stream.read_to_string(&mut result).await.unwrap();
            result
        })
        .await
        .unwrap()
    }

    async fn stop(&mut self) {
        self.cancel.cancel();
        tokio::time::timeout(DEADLINE, &mut self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

#[tokio::test]
async fn exact_routes_ignore_queries_and_bodies_and_keep_duplicate_header_values() {
    let mut server = Server::start(vec![
        MockRoute {
            method: "POST".into(),
            path: "/a%20b".into(),
            body: "answer".into(),
            headers: vec![
                ParamItem::new("X-Order", "first"),
                ParamItem::new("X-Other", "middle"),
                ParamItem::new("X-Order", "second"),
            ],
            ..Default::default()
        },
        MockRoute {
            path: "/disabled".into(),
            enabled: false,
            ..Default::default()
        },
    ])
    .await;
    let response = server.exchange("POST /a%20b?query=ignored HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\n\r\ndata").await;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("\r\n\r\nanswer"));
    let response = response.to_ascii_lowercase();
    assert!(response.find("x-order: first").unwrap() < response.find("x-order: second").unwrap());
    assert!(response.contains("content-length: 6\r\n"));
    for target in ["GET /a%20b", "POST /a%20B", "GET /disabled", "GET /missing"] {
        let response = server
            .exchange(&format!("{target} HTTP/1.1\r\nHost: localhost\r\n\r\n"))
            .await;
        assert!(response.starts_with("HTTP/1.1 404"), "{target}: {response}");
    }
    server.stop().await;
}

#[tokio::test]
async fn head_and_bodyless_statuses_obey_response_framing_rules() {
    let mut routes = vec![MockRoute {
        method: "HEAD".into(),
        body: "hello".into(),
        ..Default::default()
    }];
    for status in [204, 205, 304] {
        routes.push(MockRoute {
            path: format!("/{status}"),
            status,
            body: "suppressed".into(),
            ..Default::default()
        });
    }
    let mut server = Server::start(routes).await;
    let head = server
        .exchange("HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await;
    assert!(head.to_ascii_lowercase().contains("content-length: 5\r\n"));
    assert_eq!(head.split_once("\r\n\r\n").unwrap().1, "");
    for status in [204, 205, 304] {
        let response = server
            .exchange(&format!(
                "GET /{status} HTTP/1.1\r\nHost: localhost\r\n\r\n"
            ))
            .await;
        assert!(response.starts_with(&format!("HTTP/1.1 {status}")));
        assert_eq!(response.split_once("\r\n\r\n").unwrap().1, "");
        assert!(!response.to_ascii_lowercase().contains("transfer-encoding"));
    }
    server.stop().await;
}

#[tokio::test]
async fn bind_errors_validation_and_stop_release_all_resources() {
    let mut server = Server::start(Vec::new()).await;
    let mut partial = TcpStream::connect(server.address).await.unwrap();
    partial.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
    let occupied = serve(
        MockConfig {
            port: server.address.port(),
            routes: Vec::new(),
        },
        None,
        |_| panic!("must not report listening"),
        Arc::new(|_| {}),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(occupied, Err(MockError::Io(_))));
    let invalid = serve(
        MockConfig {
            port: 0,
            routes: vec![MockRoute {
                status: 99,
                ..Default::default()
            }],
        },
        None,
        |_| panic!("must validate before binding"),
        Arc::new(|_| {}),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(invalid, Err(MockError::Config(_))));
    server.stop().await;
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(DEADLINE, partial.read_to_end(&mut bytes))
        .await
        .unwrap();
    assert!(result.is_ok() || result.unwrap_err().kind() == std::io::ErrorKind::ConnectionReset);
    let rebound = tokio::net::TcpListener::bind(server.address).await.unwrap();
    assert_eq!(rebound.local_addr().unwrap(), server.address);
}

fn scripted_route(path: &str) -> MockRoute {
    MockRoute {
        method: "POST".into(),
        path: path.into(),
        body: "static".into(),
        script: "(answered by the test's responder)".into(),
        ..MockRoute::default()
    }
}

fn post(path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn scripted_routes_get_the_request_and_their_state_lasts() {
    let calls = Arc::new(Mutex::new(0_u32));
    let seen = Arc::clone(&calls);
    let responder: Responder = Arc::new(move |index, request| {
        let mut calls = seen.lock().unwrap();
        *calls += 1;
        assert_eq!(index, 1);
        let status = if *calls == 1 { 500 } else { 200 };
        Responded {
            reply: MockReply {
                status,
                headers: vec![("X-Query".into(), request.query.clone())],
                body: request.body,
                ..MockReply::default()
            },
            console: vec![format!("call {calls}")],
            error: None,
        }
    });
    let static_route = MockRoute {
        path: "/static".into(),
        body: "fixed".into(),
        ..MockRoute::default()
    };
    let (mut server, logged) =
        Server::scripted(vec![static_route, scripted_route("/echo")], Some(responder)).await;

    let first = server.exchange(&post("/echo?a=1", "hello")).await;
    assert!(first.starts_with("HTTP/1.1 500"), "{first}");
    let second = server.exchange(&post("/echo?a=1", "hello")).await;
    assert!(second.starts_with("HTTP/1.1 200"), "{second}");
    assert!(second.contains("x-query: a=1"), "{second}");
    assert!(second.ends_with("hello"), "echoed: {second}");
    let fixed = server
        .exchange("GET /static HTTP/1.1\r\nHost: x\r\n\r\n")
        .await;
    assert!(fixed.ends_with("fixed"));
    assert_eq!(
        *calls.lock().unwrap(),
        2,
        "a static route never asks the responder"
    );

    let log = logged.lock().unwrap().clone();
    let entries: Vec<(&str, &str, u16)> = log
        .iter()
        .map(|e| (e.method.as_str(), e.path.as_str(), e.status))
        .collect();
    assert_eq!(
        entries,
        [
            ("POST", "/echo", 500),
            ("POST", "/echo", 200),
            ("GET", "/static", 200)
        ]
    );
    assert_eq!(log[1].console, ["call 2"]);
    server.stop().await;
}

#[tokio::test]
async fn a_delay_holds_the_answer_and_a_bad_reply_becomes_a_500() {
    let responder: Responder = Arc::new(|_, request| {
        if request.path == "/slow" {
            Responded {
                reply: MockReply {
                    status: 200,
                    delay: Duration::from_millis(150),
                    ..MockReply::default()
                },
                ..Responded::default()
            }
        } else {
            Responded {
                reply: MockReply {
                    status: 200,
                    headers: vec![("Content-Length".into(), "1".into())],
                    ..MockReply::default()
                },
                ..Responded::default()
            }
        }
    });
    let (mut server, logged) = Server::scripted(
        vec![scripted_route("/slow"), scripted_route("/bad")],
        Some(responder),
    )
    .await;
    let started = Instant::now();
    let slow = server.exchange(&post("/slow", "")).await;
    assert!(slow.starts_with("HTTP/1.1 200"));
    assert!(started.elapsed() >= Duration::from_millis(150));
    let bad = server.exchange(&post("/bad", "")).await;
    assert!(bad.starts_with("HTTP/1.1 500"), "{bad}");
    assert!(bad.contains("The server sets this header itself"), "{bad}");
    assert!(logged.lock().unwrap()[1].error.is_some());
    server.stop().await;
}

#[tokio::test]
async fn scripted_routes_refuse_bodies_over_one_mebibyte() {
    let responder: Responder = Arc::new(|_, _| panic!("never asked about a refused body"));
    let (mut server, _) = Server::scripted(vec![scripted_route("/big")], Some(responder)).await;
    let body = "x".repeat(imposter_http::mock::MAX_MOCK_BODY + 1);
    let answer = server.exchange(&post("/big", &body)).await;
    assert!(answer.starts_with("HTTP/1.1 413"), "{}", &answer[..40]);
    server.stop().await;
}
