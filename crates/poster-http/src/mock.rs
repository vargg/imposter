//! A loopback-only HTTP/1 server with immutable routes for each run.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{CONNECTION, CONTENT_LENGTH, HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use poster_core::mock::{MockConfig, MockConfigError};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum MockError {
    #[error(transparent)]
    Config(#[from] MockConfigError),
    #[error("Mock server: {0}")]
    Io(#[from] std::io::Error),
}

fn response(config: &MockConfig, request: &Request<Incoming>) -> Response<Full<Bytes>> {
    let route = config.matching_route(request.method().as_str(), request.uri().path());
    let (status, body) = route.map_or((404, "Not found"), |r| (r.status, r.body.as_str()));
    let bodyless = matches!(status, 204 | 205 | 304);
    let bytes = if bodyless || request.method() == hyper::Method::HEAD {
        Bytes::new()
    } else {
        Bytes::copy_from_slice(body.as_bytes())
    };
    let mut response = Response::new(Full::new(bytes));
    *response.status_mut() =
        hyper::StatusCode::from_u16(status).expect("mock statuses are validated before binding");
    if let Some(route) = route {
        for header in route.headers.iter().filter(|header| header.is_enabled) {
            response.headers_mut().append(
                HeaderName::from_bytes(header.name.as_bytes())
                    .expect("mock header names are validated before binding"),
                HeaderValue::from_bytes(header.value.as_bytes())
                    .expect("mock header values are validated before binding"),
            );
        }
    }
    if !matches!(status, 204 | 304) {
        response.headers_mut().insert(
            CONTENT_LENGTH,
            HeaderValue::from(if bodyless { 0 } else { body.len() }),
        );
    }
    response
        .headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("close"));
    response
}

/// Report the bound address only after bind succeeds. Cancellation closes every connection.
pub async fn serve(
    config: MockConfig,
    listening: impl FnOnce(SocketAddr) + Send,
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
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        let reply = response(&config, &request);
                        async { Ok::<_, Infallible>(reply) }
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
