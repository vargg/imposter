//! The experimental MCP server, driven as a client would over its message stream: the
//! handshake, the two tools, cancellation, and secrets kept out of every message.
#![cfg(test)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use imposter_cli::SendArgs;
use imposter_cli::mcp::serve;
use imposter_testkit::{Received, Server, response};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const SECRET: &str = "tok-SECRET-42";

fn write_document(dir: &Path, base: &str) -> PathBuf {
    let request = |name: &str, path: &str| {
        json!({
            "name": name, "method": "GET", "url": format!("{{{{base}}}}{path}"),
            "body": "", "body_type": "json",
            "auth": {"type": "bearer_token", "bearer_token": {"token": "{{token}}"}},
            "checks": [{"selector": "status", "op": "equals", "expected": "200"}],
        })
    };
    let document = json!({
        "format": "imposter",
        "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": base, "token": "none"},
            "folders": [{"name": "Users", "requests": [request("List", "/users"), request("Hold", "/hold")]}],
        },
    });
    let path = dir.join("api.imposter.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

fn echo(_: usize, received: &Received) -> Vec<u8> {
    let auth = received
        .header("authorization")
        .unwrap_or_default()
        .to_owned();
    response("200 OK", &[("X-Echo", &auth)], auth.as_bytes())
}

/// The server's stdout, delivered line by line as it writes.
struct Lines(UnboundedSender<Vec<u8>>);

impl std::io::Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(buf.to_vec());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A session: lines go in through `send`, answers come out of `answers`, and `finish`
/// closes the input, as a client that leaves does.
struct Session {
    input: UnboundedSender<String>,
    output: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pending: String,
    task: tokio::task::JoinHandle<()>,
}

impl Session {
    fn start(file: PathBuf, secret: bool) -> Self {
        Self::start_with(Some(file), secret, None)
    }

    /// A server over `file`, or over the profile in `data` without one.
    fn start_with(file: Option<PathBuf>, secret: bool, data: Option<String>) -> Self {
        let (input, lines) = unbounded_channel();
        let (written, output) = unbounded_channel();
        let args = SendArgs {
            file,
            inputs: if secret {
                vec![imposter_cli::InputArg {
                    variable: "token".into(),
                    from: "API_TOKEN".into(),
                    secret: true,
                }]
            } else {
                Vec::new()
            },
            ..SendArgs::default()
        };
        let task = tokio::task::spawn_blocking(move || {
            let mut env: HashMap<String, String> =
                [("API_TOKEN".to_owned(), SECRET.to_owned())].into();
            if let Some(data) = data {
                env.insert("IMPOSTER_DATA_DIR".to_owned(), data);
            }
            let var = move |name: &str| env.get(name).cloned();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(serve(lines, &mut Lines(written), &args, &var))
                .unwrap();
        });
        Self {
            input,
            output,
            pending: String::new(),
            task,
        }
    }

    /// The next `count` messages the server writes.
    async fn answers(&mut self, count: usize) -> Vec<Value> {
        let mut messages = Vec::new();
        while messages.len() < count {
            if let Some(end) = self.pending.find('\n') {
                let line: String = self.pending.drain(..=end).collect();
                messages.push(serde_json::from_str(&line).unwrap());
                continue;
            }
            let chunk = self.output.recv().await.expect("the server stopped early");
            self.pending.push_str(&String::from_utf8(chunk).unwrap());
        }
        messages
    }

    fn send(&self, message: &Value) {
        self.input.send(message.to_string()).unwrap();
    }

    async fn finish(mut self) -> Vec<Value> {
        drop(self.input);
        self.task.await.unwrap();
        let mut rest = std::mem::take(&mut self.pending);
        while let Ok(chunk) = self.output.try_recv() {
            rest.push_str(&String::from_utf8(chunk).unwrap());
        }
        rest.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    let mut message =
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name}});
    message["params"]["arguments"] = arguments;
    message
}

fn by_id(messages: &[Value], id: u64) -> &Value {
    messages
        .iter()
        .find(|m| m["id"] == json!(id))
        .unwrap_or_else(|| panic!("no answer to {id}: {messages:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_lists_and_sends_requests_with_secrets_kept_out() {
    let server = Server::start(echo).await;
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::start(write_document(dir.path(), &server.url), true);
    session.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    session.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    session.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    session.send(&call(3, "list_requests", json!({})));
    session.send(&call(4, "send_request", json!({"path": "Users/List"})));
    session.send(&call(5, "send_request", json!({"path": "Users/Nope"})));
    session.send(&call(6, "send_request", json!({})));
    session.send(&call(7, "fetch", json!({"url": "http://example.com"})));
    session.send(&json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"}));
    session.send(&json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}));
    session.input.send("not json".into()).unwrap();
    let messages = session.answers(10).await;
    assert_eq!(session.finish().await, Vec::<Value>::new());

    let init = &by_id(&messages, 1)["result"];
    assert_eq!(
        init["protocolVersion"], "2025-06-18",
        "a known version is kept"
    );
    assert_eq!(init["serverInfo"]["name"], "imposter");
    let tools: Vec<&str> = by_id(&messages, 2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["list_requests", "send_request"]);
    assert_eq!(
        by_id(&messages, 3)["result"]["structuredContent"]["requests"],
        json!(["Users/List", "Users/Hold"])
    );
    let sent = &by_id(&messages, 4)["result"];
    assert_eq!(sent["isError"], false);
    assert_eq!(sent["structuredContent"]["outcome"], "passed");
    let text: Value = serde_json::from_str(sent["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        text, sent["structuredContent"],
        "the text repeats the structured result"
    );
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert_eq!(
        by_id(&messages, 5)["result"]["structuredContent"]["outcome"],
        "not_sent"
    );
    assert_eq!(by_id(&messages, 6)["error"]["code"], -32602);
    assert_eq!(
        by_id(&messages, 7)["error"]["message"],
        "Unknown tool \"fetch\""
    );
    assert_eq!(by_id(&messages, 8)["error"]["code"], -32601);
    assert_eq!(by_id(&messages, 9)["result"], json!({}));
    assert!(messages.iter().any(|m| m["error"]["code"] == -32700));
    assert_eq!(
        server.requests().len(),
        1,
        "only the request that exists was sent"
    );
    for message in &messages {
        assert!(!message.to_string().contains(SECRET), "{message}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_stops_the_send_it_names() {
    let (arrived, mut wait) = unbounded_channel();
    let (_release, hold) = std::sync::mpsc::channel::<()>();
    let hold = std::sync::Mutex::new(hold);
    let server = Server::start(move |_, _| {
        let _ = arrived.send(());
        // Answer only once the test has gone: the run must end without a response.
        let _ = hold.lock().unwrap().recv();
        response("200 OK", &[], b"")
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::start(write_document(dir.path(), &server.url), false);
    session.send(&call(1, "send_request", json!({"path": "Users/Hold"})));
    wait.recv().await.unwrap();
    session.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
    session.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 99}}),
    );
    session.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
    );
    let messages = session.answers(2).await;
    assert_eq!(messages[0]["id"], 1, "the send answers first");
    session.finish().await;
    assert_eq!(
        by_id(&messages, 1)["result"]["structuredContent"]["outcome"],
        "cancelled"
    );
    assert_eq!(
        by_id(&messages, 2)["result"],
        json!({}),
        "a message during a send waits its turn"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_leaves_mid_send_stops_it() {
    let (arrived, mut wait) = unbounded_channel();
    let (_release, hold) = std::sync::mpsc::channel::<()>();
    let hold = std::sync::Mutex::new(hold);
    let server = Server::start(move |_, _| {
        let _ = arrived.send(());
        let _ = hold.lock().unwrap().recv();
        response("200 OK", &[], b"")
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let session = Session::start(write_document(dir.path(), &server.url), false);
    session.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}));
    session.send(&call(2, "send_request", json!({"path": "Users/Hold"})));
    wait.recv().await.unwrap();
    let messages = session.finish().await;
    assert_eq!(
        by_id(&messages, 1)["result"]["protocolVersion"],
        imposter_cli::mcp::PROTOCOL_VERSIONS[0],
        "an unknown version gets the newest"
    );
    assert_eq!(
        by_id(&messages, 2)["result"]["structuredContent"]["outcome"],
        "cancelled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_file_it_serves_the_saved_requests() {
    use imposter_app::core::model::{RequestTemplate, Variables};
    use imposter_app::store::{Store, collections};
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(&data.path().join("imposter.db")).unwrap();
    let url = format!("{}/ping", server.url);
    store
        .call(move |db| {
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(
                db,
                api.id,
                None,
                &RequestTemplate {
                    name: "Ping".into(),
                    method: "GET".into(),
                    url,
                    ..RequestTemplate::default()
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    let mut session =
        Session::start_with(None, false, Some(data.path().to_str().unwrap().to_owned()));
    session.send(&call(1, "list_requests", json!({})));
    session.send(&call(2, "send_request", json!({"path": "API / Ping"})));
    let messages = session.answers(2).await;
    session.finish().await;
    assert_eq!(
        by_id(&messages, 1)["result"]["structuredContent"]["requests"],
        json!(["API/Ping"])
    );
    let sent = &by_id(&messages, 2)["result"]["structuredContent"];
    assert_eq!(sent["outcome"], "no_checks");
    assert_eq!(sent["definition"]["source"], "profile");
    assert_eq!(server.requests().len(), 1);
}
