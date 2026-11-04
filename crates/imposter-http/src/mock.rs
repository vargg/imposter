//! A loopback-only HTTP/1 server with immutable routes for each run. Static routes answer
//! from their configuration; scripted ones ask the responder the app passes in, so this crate
//! needs no script engine. Every answered request goes to the log callback, without bodies.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, CONTENT_LENGTH, HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use imposter_core::mock::{
    MockConfig, MockConfigError, MockLogEntry, MockReply, MockRequest, Responded,
};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Request bodies a scripted route may read; larger ones are answered 413.
pub const MAX_MOCK_BODY: usize = 1024 * 1024;

/// Answers a scripted route (by its index in the configuration). It may run a script for
/// up to its own limit, so the server calls it on a blocking thread.
pub type Responder = Arc<dyn Fn(usize, MockRequest) -> Responded + Send + Sync>;

/// Told about every answered request.
pub type Log = Arc<dyn Fn(MockLogEntry) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum MockError {
    #[error(transparent)]
    Config(#[from] MockConfigError),
    #[error("Mock server: {0}")]
    Io(#[from] std::io::Error),
}

fn text_reply(status: u16, text: &str) -> MockReply {
    MockReply {
        status,
        headers: vec![(
            "Content-Type".to_owned(),
            "text/plain; charset=utf-8".to_owned(),
        )],
        body: text.as_bytes().to_vec(),
        delay: Duration::ZERO,
    }
}

/// The HTTP response for `reply`, which has been checked (`MockReply::problem`).
fn response(reply: &MockReply, head: bool) -> Response<Full<Bytes>> {
    let bodyless = matches!(reply.status, 204 | 205 | 304);
    let bytes = if bodyless || head {
        Bytes::new()
    } else {
        Bytes::copy_from_slice(&reply.body)
    };
    let mut response = Response::new(Full::new(bytes));
    *response.status_mut() = hyper::StatusCode::from_u16(reply.status)
        .expect("mock statuses are checked to be 200 to 599 before answering");
    for (name, value) in &reply.headers {
        response.headers_mut().append(
            HeaderName::from_bytes(name.as_bytes())
                .expect("mock header names are checked before answering"),
            HeaderValue::from_bytes(value.as_bytes())
                .expect("mock header values are checked before answering"),
        );
    }
    if !matches!(reply.status, 204 | 304) {
        response.headers_mut().insert(
            CONTENT_LENGTH,
            HeaderValue::from(if bodyless { 0 } else { reply.body.len() }),
        );
    }
    response
        .headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("close"));
    response
}

/// Ask the responder about a scripted route, reading the body first (up to the cap).
async fn scripted(responder: &Responder, index: usize, request: Request<Incoming>) -> Responded {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = match Limited::new(request.into_body(), MAX_MOCK_BODY)
        .collect()
        .await
    {
        Ok(collected) => collected.to_bytes().to_vec(),
        Err(error) if error.is::<http_body_util::LengthLimitError>() => {
            return Responded {
                reply: text_reply(413, "Mock request bodies are limited to 1 MiB"),
                ..Responded::default()
            };
        }
        Err(_) => {
            return Responded {
                reply: text_reply(400, "The request body could not be read"),
                ..Responded::default()
            };
        }
    };
    let request = MockRequest {
        method,
        path,
        query,
        headers,
        body,
    };
    let responder = Arc::clone(responder);
    tokio::task::spawn_blocking(move || responder(index, request))
        .await
        .unwrap_or_else(|_| {
            Responded::failed("Mock script failed unexpectedly".to_owned(), Vec::new())
        })
}

async fn answer(
    config: &MockConfig,
    responder: Option<&Responder>,
    log: &Log,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let started = Instant::now();
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let head = request.method() == hyper::Method::HEAD;
    let index = config.matching_index(&method, &path);
    let mut outcome = match (index, responder) {
        (Some(index), Some(responder)) if config.routes[index].is_scripted() => {
            scripted(responder, index, request).await
        }
        (Some(index), _) => Responded {
            reply: MockReply::from_route(&config.routes[index]),
            ..Responded::default()
        },
        (None, _) => Responded {
            reply: MockReply {
                status: 404,
                body: b"Not found".to_vec(),
                ..MockReply::default()
            },
            ..Responded::default()
        },
    };
    // A responder is any code: what it answers is checked here, so nothing it sends can
    // break the response.
    if let Some(problem) = outcome.reply.problem() {
        outcome = Responded::failed(problem, outcome.console);
    }
    if !outcome.reply.delay.is_zero() {
        tokio::time::sleep(outcome.reply.delay).await;
    }
    log(MockLogEntry {
        method,
        path,
        status: outcome.reply.status,
        elapsed: started.elapsed(),
        error: outcome.error,
        console: outcome.console,
    });
    response(&outcome.reply, head)
}

/// Report the bound address only after bind succeeds. Cancellation closes every connection.
pub async fn serve(
    config: MockConfig,
    responder: Option<Responder>,
    listening: impl FnOnce(SocketAddr) + Send,
    log: Log,
    cancel: CancellationToken,
) -> Result<(), MockError> {
    config.validate()?;
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], config.port))).await?;
    listening(listener.local_addr()?);
    let config = Arc::new(config);
    let mut connections = JoinSet::new();
    let result = loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = match accepted { Ok(value) => value, Err(error) => break Err(error.into()) };
                // A local client must not grow the task set without bound.
                if connections.len() >= 128 { continue; }
                let config = Arc::clone(&config);
                let responder = responder.clone();
                let log = Arc::clone(&log);
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        let config = Arc::clone(&config);
                        let responder = responder.clone();
                        let log = Arc::clone(&log);
                        async move {
                            Ok::<_, Infallible>(answer(&config, responder.as_ref(), &log, request).await)
                        }
                    });
                    // Clients may leave headers or bodies incomplete; close bounded sessions.
                    let connection = http1::Builder::new().keep_alive(false).serve_connection(TokioIo::new(stream), service);
                    let _ = tokio::time::timeout(Duration::from_secs(30), connection).await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    };
    drop(listener);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    result
}
