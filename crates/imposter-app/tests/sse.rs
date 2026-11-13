//! Event streams through Send: events reach the sink live, the request's listening limit
//! is a normal end its checks are decided on, and a stream cut at the capture limit leaves
//! its checks undecided, takes no captures and never passes. History keeps the stream.
#![cfg(test)]

use std::sync::{Arc, Mutex};

use imposter_app::executor::execute_streaming;
use imposter_core::check::{Check, Op, Verdict};
use imposter_core::model::{Options, Request, RequestTemplate, Variables};
use imposter_core::run_result::RunOutcome;
use imposter_core::select::Selector;
use imposter_core::sse::{Event, StreamEnd};
use imposter_store::{Store, collections, executions};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// A server answering every request with `stream` as an event stream, then holding the
/// connection open until the client goes.
async fn serve(stream: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).await.unwrap_or(0) == 1
                {
                    head.push(byte[0]);
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                    .await;
                let _ = socket.write_all(stream).await;
                // Hold until the client hangs up.
                let _ = socket.read(&mut byte).await;
            });
        }
    });
    url
}

async fn saved(store: &Store, url: String, options: Options, checks: Vec<Check>) -> Request {
    store
        .call(move |db| {
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(
                db,
                collection.id,
                None,
                &RequestTemplate {
                    name: "events".into(),
                    method: "GET".into(),
                    url,
                    options,
                    checks,
                    ..RequestTemplate::default()
                },
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_listening_limit_ends_the_stream_normally_and_its_checks_decide() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let url = serve(b"event: price\ndata: 1\n\nevent: price\ndata: 2\n\ndata: 3\n\n").await;
    let request = saved(
        &store,
        url,
        Options {
            stream_events: Some(2),
            ..Options::default()
        },
        vec![Check::new(Selector::Status, Op::Equals, "200")],
    )
    .await;
    let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
    let sink = seen.clone();

    let executed = execute_streaming(
        &store.handle(),
        request,
        CancellationToken::new(),
        Some(Arc::new(move |events| sink.lock().unwrap().extend(events))),
    )
    .await
    .unwrap();

    let stream = executed.response.stream.as_ref().unwrap();
    assert_eq!(stream.end, StreamEnd::Limit);
    let data: Vec<&str> = stream.events.iter().map(|e| e.data.as_str()).collect();
    assert_eq!(data, ["1", "2"]);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "the sink saw them as they came"
    );
    assert_eq!(RunOutcome::of(&executed.response), RunOutcome::Passed);

    let id = executed.record.unwrap();
    let recorded = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recorded.response.stream, executed.response.stream);
}

#[tokio::test]
async fn a_stream_cut_at_the_capture_limit_decides_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let long: &'static [u8] = Box::leak("data: x\n\n".repeat(300).into_bytes().into_boxed_slice());
    let url = serve(long).await;
    let request = saved(
        &store,
        url,
        Options {
            // One KiB, which the stream passes.
            download_limit: Some(1),
            ..Options::default()
        },
        vec![Check::new(Selector::Status, Op::Equals, "200")],
    )
    .await;

    let executed = execute_streaming(&store.handle(), request, CancellationToken::new(), None)
        .await
        .unwrap();

    let response = &executed.response;
    assert_eq!(response.stream.as_ref().unwrap().end, StreamEnd::Cut);
    assert_eq!(
        response.checks[0].verdict,
        Verdict::Unchecked("the stream was cut at the size limit".into())
    );
    assert_eq!(RunOutcome::of(response), RunOutcome::NotChecked);
}
