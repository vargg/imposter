//! `imposter send` with event streams: each way a stream ends has its outcome and exit
//! status, and the result carries the stream (result version 2).
#![cfg(test)]

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;

use imposter_cli::{Io, main};
use imposter_core::run_result::{RunOutcome, RunResult, STREAM_VERSION};
use imposter_core::sse::StreamEnd;
use serde_json::json;

/// A server on its own thread: `/closed` sends two events and closes, `/open` sends them
/// and holds the connection until the client goes, `/long` sends far more than a KiB.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let target = String::from_utf8_lossy(&head)
                    .split(' ')
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let _ = socket.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                );
                let events = if target == "/long" {
                    "data: x\n\n".repeat(300)
                } else {
                    "data: 1\n\ndata: 2\n\n".to_owned()
                };
                let _ = socket.write_all(events.as_bytes());
                if target != "/closed" {
                    let _ = socket.read(&mut byte);
                }
            });
        }
    });
    base
}

fn send(file: &str, name: &str) -> (u8, RunResult) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let args = ["send", "--file", file, name];
    let status = main(
        args.iter().map(OsString::from).collect(),
        &mut Io {
            stdout: &mut stdout,
            stderr: &mut stderr,
            var: &|_| None,
        },
    );
    (status, RunResult::read(&stdout).unwrap())
}

#[test]
fn each_way_a_stream_ends_has_its_outcome_and_exit_status() {
    let base = serve();
    let check = json!([{"selector": "status", "op": "equals", "expected": "200"}]);
    let request = |name: &str, path: &str, options: serde_json::Value| {
        json!({"name": name, "method": "GET", "url": format!("{base}{path}"), "body": "",
            "body_type": "json", "checks": check, "options": options})
    };
    let document = json!({
        "format": "imposter",
        "version": 5,
        "collection": {"name": "API", "variables": {}, "requests": [
            request("Closed", "/closed", json!({})),
            request("Limit", "/open", json!({"stream_events": 2})),
            request("Cut", "/long", json!({"download_limit": 1})),
            request("Silent", "/open", json!({"timeout": 0.2})),
        ]},
    });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("streams.imposter.json");
    std::fs::write(&file, serde_json::to_vec(&document).unwrap()).unwrap();
    let file = file.to_str().unwrap();

    for (name, end, outcome) in [
        ("Closed", StreamEnd::Closed, RunOutcome::Passed),
        ("Limit", StreamEnd::Limit, RunOutcome::Passed),
        ("Cut", StreamEnd::Cut, RunOutcome::NotChecked),
        ("Silent", StreamEnd::TimedOut, RunOutcome::TransportError),
    ] {
        let (status, result) = send(file, name);
        assert_eq!(result.outcome, outcome, "{name}: {}", result.summary);
        assert_eq!(status, outcome.exit_status(), "{name}");
        assert_eq!(result.version, STREAM_VERSION, "{name}");
        let stream = result.stream.unwrap();
        assert_eq!(stream.end, end, "{name}");
        assert!(stream.received >= 2, "{name}");
    }
}
