//! Execution end to end: a real database, a loopback server, and the recorded history.
#![cfg(test)]

use std::time::Duration;

use poster_app::executor::{ExecuteError, HISTORY_ERROR, execute, inputs};
use poster_core::execution::Outcome;
use poster_core::ids::CollectionId;
use poster_core::model::{BodyType, ParamItem, Request, RequestTemplate, Variables};
use poster_core::settings::AppSettings;
use poster_core::variables::Source;
use poster_store::{Store, collections, environments, executions, kv};
use poster_testkit::{Server, response};
use tokio_util::sync::CancellationToken;

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("poster.db")).unwrap();
    (dir, store)
}

/// A saved request in a collection with variables, plus globals and an active environment.
async fn setup(store: &Store, url: &str) -> Request {
    let url = url.to_owned();
    store
        .call(move |db| {
            environments::set_globals(
                db,
                &vars(&[
                    ("scope", "global"),
                    ("only_global", "g"),
                    ("token", "global-token"),
                ]),
            )?;
            let env =
                environments::create_environment(db, "Dev", &vars(&[("scope", "environment")]))?;
            environments::set_active_environment(db, Some(env.id))?;
            let collection = collections::create_collection(
                db,
                "API",
                "",
                &vars(&[("scope", "collection"), ("base", &url)]),
            )?;
            let template = RequestTemplate {
                name: "probe".into(),
                method: "POST".into(),
                url: "{{base}}/items".into(),
                headers: vec![
                    ParamItem::new("X-Scope", "{{scope}}"),
                    ParamItem::new("X-Global", "{{only_global}}"),
                ],
                body: b"{\"token\": \"{{token}}\"}".to_vec(),
                body_type: BodyType::Json,
                ..RequestTemplate::default()
            };
            collections::create_request(db, collection.id, None, &template)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn variables_resolve_by_precedence_and_history_records_both_forms() {
    let (_dir, store) = open();
    let server = Server::start(|_, _| response("200 OK", &[], b"created")).await;
    let request = setup(&store, &server.url).await;

    let executed = execute(&store.handle(), request.clone(), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(
        (executed.response.status, executed.response.body.as_slice()),
        (200, b"created".as_slice())
    );
    assert!(executed.response.elapsed > 0.0);
    let received = server.requests().remove(0);
    assert_eq!(received.request_line, "POST /items HTTP/1.1");
    assert_eq!(received.header("x-scope"), Some("environment"));
    assert_eq!(received.header("x-global"), Some("g"));
    assert_eq!(received.body, br#"{"token": "global-token"}"#);

    let id = executed.record.unwrap();
    let record = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.template.template, request.template,
        "the template keeps its placeholders"
    );
    assert_eq!(
        record.resolved.template.url,
        format!("{}/items", server.url)
    );
    assert_eq!(record.environment, "Dev");
    assert!(record.environment_id.is_some());
    assert_eq!(record.request_id, request.id);
    assert_eq!(
        record.template.collection_variables_snapshot["scope"],
        "collection"
    );
    assert_eq!(record.response, executed.response);
}

#[tokio::test]
async fn inherited_settings_resolve_against_the_same_variables() {
    let (_dir, store) = open();
    let proxy = Server::start(|_, _| response("200 OK", &[], b"via proxy")).await;
    let port = proxy.port.to_string();
    store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("proxy_port", &port)]))?;
            kv::set_app_settings(
                db,
                &AppSettings {
                    proxy_url: "http://127.0.0.1:{{proxy_port}}".into(),
                    ..AppSettings::default()
                },
            )
        })
        .await
        .unwrap();
    let request = Request {
        template: RequestTemplate {
            url: "http://target.invalid/x".into(),
            body_type: BodyType::None,
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let executed = execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(executed.response.body, b"via proxy");
    assert_eq!(
        proxy.requests().remove(0).request_line,
        "GET http://target.invalid/x HTTP/1.1"
    );
}

async fn history_count(store: &Store) -> u64 {
    store.call(|db| executions::count(db)).await.unwrap()
}

#[tokio::test]
async fn requests_that_cannot_be_sent_are_rejected_and_not_recorded() {
    let (_dir, store) = open();
    let unresolved = Request {
        template: RequestTemplate {
            url: "http://h/{{missing}}".into(),
            ..RequestTemplate::default()
        },
        ..Request::default()
    };
    let bad_url = Request {
        template: RequestTemplate {
            url: "ftp://h/".into(),
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let first = execute(&store.handle(), unresolved, CancellationToken::new()).await;
    let second = execute(&store.handle(), bad_url, CancellationToken::new()).await;

    assert!(
        matches!(first, Err(ExecuteError::Invalid(ref m)) if m == "Unresolved variables: missing")
    );
    // Unlike the Python app, which recorded URL errors as transport errors: nothing was sent.
    assert!(
        matches!(second, Err(ExecuteError::Invalid(ref m)) if m == "Enter an absolute HTTP or HTTPS URL")
    );
    assert_eq!(history_count(&store).await, 0);
}

#[tokio::test]
async fn transport_failures_are_responses_with_an_outcome_and_are_recorded() {
    let (_dir, store) = open();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let request = Request {
        template: RequestTemplate {
            url: format!("http://127.0.0.1:{port}/"),
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let executed = execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(executed.response.outcome, Outcome::TransportError);
    assert_eq!(executed.response.error_kind, "ConnectError");
    assert_eq!(history_count(&store).await, 1);
}

#[tokio::test]
async fn cancelled_executions_are_recorded_as_cancelled() {
    let (_dir, store) = open();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        accepted_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        accepted_rx.await.unwrap();
        trigger.cancel();
    });
    let request = Request {
        template: RequestTemplate {
            url,
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let executed = execute(&store.handle(), request, cancel).await.unwrap();

    assert_eq!(executed.response.outcome, Outcome::Cancelled);
    let summary = store
        .call(|db| executions::page(db, None, 0, 1))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(summary.outcome, Outcome::Cancelled);
}

#[tokio::test]
async fn a_history_failure_keeps_the_response() {
    let (_dir, store) = open();
    let body = vec![b'x'; 4096];
    let server = Server::start(move |_, _| response("200 OK", &[], &body)).await;
    store
        .call(|db| {
            kv::set_app_settings(
                db,
                &AppSettings {
                    history_max_bytes: 1024,
                    ..AppSettings::default()
                },
            )
        })
        .await
        .unwrap();
    let request = Request {
        template: RequestTemplate {
            url: server.url.clone(),
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let executed = execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(executed.response.body.len(), 4096);
    assert_eq!(executed.response.history_error, HISTORY_ERROR);
    assert_eq!(executed.record, None);
}

#[tokio::test]
async fn a_deleted_collection_falls_back_to_the_captured_variables() {
    let (_dir, store) = open();
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let request = Request {
        collection_id: Some(CollectionId(404)),
        collection_variables_snapshot: vars(&[("base", &server.url)]),
        template: RequestTemplate {
            url: "{{base}}/restored".into(),
            ..RequestTemplate::default()
        },
        ..Request::default()
    };

    let found = inputs(&store.handle(), &request).await.unwrap();
    let executed = execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(executed.response.status, 200);
    let source = &found.snapshot.effective("base").unwrap().source;
    assert_eq!(
        source,
        &Source::Collection {
            id: None,
            name: "Captured collection (deleted)".into()
        }
    );
}

#[tokio::test]
async fn switching_the_environment_mid_request_does_not_change_it() {
    let (_dir, store) = open();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let (mut stream, _) = listener.accept().await.unwrap();
        poster_testkit::read_request(&mut stream).await;
        arrived_tx.send(()).unwrap();
        release_rx.await.unwrap();
        stream
            .write_all(&response("200 OK", &[], b""))
            .await
            .unwrap();
    });
    let request = setup(&store, &url).await;
    let handle = store.handle();
    let running =
        tokio::spawn(async move { execute(&handle, request, CancellationToken::new()).await });

    tokio::time::timeout(Duration::from_secs(5), arrived_rx)
        .await
        .unwrap()
        .unwrap();
    store
        .call(|db| {
            let other = environments::create_environment(db, "Prod", &vars(&[("scope", "prod")]))?;
            environments::set_active_environment(db, Some(other.id))
        })
        .await
        .unwrap();
    release_tx.send(()).unwrap();
    let executed = running.await.unwrap().unwrap();

    let id = executed.record.unwrap();
    let record = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.environment, "Dev");
    assert!(
        record
            .resolved
            .template
            .headers
            .contains(&ParamItem::new("X-Scope", "environment"))
    );
}
