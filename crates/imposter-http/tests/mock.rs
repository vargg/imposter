//! Static responses over real loopback sockets, including shutdown of incomplete requests.
#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use imposter_core::mock::{MockConfig, MockRoute};
use imposter_core::model::ParamItem;
use imposter_http::mock::{MockError, serve};
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

impl Server {
    async fn start(routes: Vec<MockRoute>) -> Self {
        let (tx, rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let task = tokio::spawn(serve(
            MockConfig { port: 0, routes },
            move |address| {
                tx.send(address).unwrap();
            },
            token,
        ));
        let address = tokio::time::timeout(DEADLINE, rx).await.unwrap().unwrap();
        Self {
            address,
            cancel,
            task,
        }
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
        |_| panic!("must not report listening"),
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
        |_| panic!("must validate before binding"),
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
