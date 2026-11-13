//! Event streams on the wire, from a hand-written loopback server that writes in parts and
//! waits for the test between them: events reach the sink while the stream is open, and
//! the stream ends as it closes, at either limit, at the capture limit, on cancel or when
//! nothing arrives in time, keeping what arrived every time.
#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use imposter_core::prepare::{Prepared, PreparedAuth, PreparedBody};
use imposter_core::sse::{Event, StreamEnd};
use imposter_http::tls::TlsSettings;
use imposter_http::{Listening, Policy, send_streaming};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// What the server does after the head, in order.
enum Step {
    Write(&'static [u8]),
    /// Wait until the test says go on.
    Wait,
}

/// A server answering one request with an event stream; `go` moves it past each `Wait`.
async fn serve(steps: Vec<Step>) -> (String, Arc<Notify>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/events", listener.local_addr().unwrap());
    let go = Arc::new(Notify::new());
    let wait = go.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).await.unwrap_or(0) == 1 {
            head.push(byte[0]);
        }
        let _ = stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await;
        for step in steps {
            match step {
                Step::Write(bytes) => {
                    if stream.write_all(bytes).await.is_err() {
                        return;
                    }
                    let _ = stream.flush().await;
                }
                Step::Wait => wait.notified().await,
            }
        }
    });
    (url, go)
}

fn policy(timeout: Duration, capture_limit: Option<u64>) -> Policy {
    Policy {
        timeout,
        follow_redirects: true,
        proxy: None,
        tls: TlsSettings {
            verify: true,
            ..TlsSettings::default()
        },
        identity_routing: None,
        capture_limit,
        jar: None,
        send_jar: true,
    }
}

fn get(url: &str) -> Prepared {
    Prepared {
        method: "GET".into(),
        url: url.into(),
        headers: Vec::new(),
        body: PreparedBody::Bytes(Vec::new()),
        auth: PreparedAuth::None,
    }
}

/// A sink that keeps what it is given and calls `then` with the count so far.
fn sink(then: impl Fn(usize) + Send + Sync + 'static) -> (Arc<Mutex<Vec<Event>>>, Listening) {
    let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
    let kept = seen.clone();
    let listening = Listening {
        sink: Some(Arc::new(move |events| {
            let mut kept = kept.lock().unwrap();
            kept.extend(events);
            then(kept.len());
        })),
        ..Listening::default()
    };
    (seen, listening)
}

fn data(events: &[Event]) -> Vec<&str> {
    events.iter().map(|e| e.data.as_str()).collect()
}

const LONG: Duration = Duration::from_secs(10);

#[tokio::test]
async fn events_arrive_while_the_stream_is_open_and_it_ends_when_closed() {
    let (url, go) = serve(vec![
        Step::Write(b"event: tick\ndata: one\n\ndata: tw"),
        Step::Wait,
        Step::Write(b"o\n\n"),
    ])
    .await;
    let (seen, listening) = sink(move |count| {
        // The first event arrived with the stream still open: let it finish.
        if count == 1 {
            go.notify_one();
        }
    });

    let response = send_streaming(
        &get(&url),
        &policy(LONG, None),
        &CancellationToken::new(),
        &listening,
    )
    .await
    .unwrap();

    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::Closed);
    assert_eq!(data(&stream.events), ["one", "two"]);
    assert_eq!(stream.events[0].event, "tick");
    assert_eq!(*seen.lock().unwrap(), stream.events);
    assert_eq!(response.body, b"event: tick\ndata: one\n\ndata: two\n\n");
}

#[tokio::test]
async fn an_events_limit_is_a_normal_end() {
    let (url, _go) = serve(vec![
        Step::Write(b"data: 1\n\ndata: 2\n\ndata: 3\n\ndata: 4\n\n"),
        Step::Wait,
    ])
    .await;
    let listening = Listening {
        events: Some(3),
        ..Listening::default()
    };
    let response = send_streaming(
        &get(&url),
        &policy(LONG, None),
        &CancellationToken::new(),
        &listening,
    )
    .await
    .unwrap();
    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::Limit);
    assert_eq!(data(&stream.events), ["1", "2", "3"]);
}

#[tokio::test]
async fn a_time_limit_is_a_normal_end() {
    let (url, _go) = serve(vec![Step::Write(b"data: only\n\n"), Step::Wait]).await;
    let listening = Listening {
        duration: Some(Duration::from_millis(100)),
        ..Listening::default()
    };
    let response = send_streaming(
        &get(&url),
        &policy(LONG, None),
        &CancellationToken::new(),
        &listening,
    )
    .await
    .unwrap();
    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::Limit);
    assert_eq!(data(&stream.events), ["only"]);
}

#[tokio::test]
async fn the_capture_limit_cuts_the_stream_and_keeps_what_fit() {
    let (url, _go) = serve(vec![
        Step::Write(b"data: first\n\ndata: second-is-longer\n\n"),
        Step::Wait,
    ])
    .await;
    let response = send_streaming(
        &get(&url),
        &policy(LONG, Some(20)),
        &CancellationToken::new(),
        &Listening::default(),
    )
    .await
    .unwrap();
    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::Cut);
    assert!(response.truncated);
    assert_eq!(response.body.len(), 20);
    assert_eq!(data(&stream.events), ["first"]);
}

#[tokio::test]
async fn cancel_stops_listening_and_keeps_the_events() {
    let (url, _go) = serve(vec![Step::Write(b"data: before\n\n"), Step::Wait]).await;
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let (_, listening) = sink(move |_| stop.cancel());
    let response = send_streaming(&get(&url), &policy(LONG, None), &cancel, &listening)
        .await
        .unwrap();
    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::Cancelled);
    assert_eq!(data(&stream.events), ["before"]);
}

#[tokio::test]
async fn silence_past_the_timeout_ends_the_stream() {
    let (url, _go) = serve(vec![Step::Write(b"data: last\n\n"), Step::Wait]).await;
    let response = send_streaming(
        &get(&url),
        &policy(Duration::from_millis(100), None),
        &CancellationToken::new(),
        &Listening::default(),
    )
    .await
    .unwrap();
    let stream = response.stream.unwrap();
    assert_eq!(stream.end, StreamEnd::TimedOut);
    assert_eq!(data(&stream.events), ["last"]);
}
