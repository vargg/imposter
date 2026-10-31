//! Execution end to end: a real database, a loopback server, and the recorded history.
#![cfg(test)]

use std::time::Duration;

use imposter_app::executor::{ExecuteError, HISTORY_ERROR, execute, inputs};
use imposter_core::execution::Outcome;
use imposter_core::ids::CollectionId;
use imposter_core::model::{BodyType, ParamItem, Request, RequestTemplate, Variables};
use imposter_core::settings::AppSettings;
use imposter_core::variables::Source;
use imposter_store::{Store, collections, environments, executions, kv};
use imposter_testkit::{Server, response};
use tokio_util::sync::CancellationToken;

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
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
async fn checks_run_against_the_response_with_the_same_variables_and_are_recorded() {
    use imposter_core::check::{Check, Op, Verdict};
    use imposter_core::select::Selector;
    let (_dir, store) = open();
    let server = Server::start(|_, _| {
        response(
            "201 Created",
            &[("Content-Type", "application/json")],
            br#"{"scope":"environment","id":7}"#,
        )
    })
    .await;
    let mut request = setup(&store, &server.url).await;
    request.template.checks = vec![
        Check::new(Selector::Status, Op::Equals, "2xx"),
        Check::new(Selector::Json("$.scope".into()), Op::Equals, "{{scope}}"),
        Check::new(Selector::Json("$.id".into()), Op::Greater, "10"),
        Check::new(
            Selector::Header("X-Missing".into()),
            Op::Equals,
            "{{undefined}}",
        ),
    ];

    let executed = execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();
    let verdicts: Vec<&Verdict> = executed
        .response
        .checks
        .iter()
        .map(|r| &r.verdict)
        .collect();
    assert_eq!(
        verdicts,
        [
            &Verdict::Passed,
            &Verdict::Passed,
            &Verdict::Failed,
            &Verdict::Unchecked("Unresolved variables: undefined".into()),
        ]
    );
    assert_eq!(executed.response.checks[1].check.expected, "environment");
    let id = executed.record.unwrap();
    let record = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.response.checks, executed.response.checks);
}

#[tokio::test]
async fn captured_values_feed_the_next_request() {
    use imposter_app::executor::write_captures;
    use imposter_core::capture::{Capture, Taken};
    use imposter_core::select::Selector;
    let (_dir, store) = open();
    let server = Server::start(|index, _| {
        if index == 0 {
            response(
                "200 OK",
                &[("Content-Type", "application/json")],
                br#"{"token":"fresh","id":42}"#,
            )
        } else {
            response("200 OK", &[], b"ok")
        }
    })
    .await;
    let mut login = setup(&store, &server.url).await;
    login.template.captures = vec![
        Capture::new("token", Selector::Json("$.token".into())),
        Capture::new("scope", Selector::Json("$.id".into())),
        Capture::new("new_one", Selector::Status),
    ];
    let executed = execute(&store.handle(), login.clone(), CancellationToken::new())
        .await
        .unwrap();
    let targets: Vec<(&str, &Taken, &str)> = executed
        .response
        .captures
        .iter()
        .map(|c| (c.variable.as_str(), &c.taken, c.target.as_str()))
        .collect();
    assert_eq!(
        targets,
        [
            ("token", &Taken::Value("fresh".into()), "Global"),
            ("scope", &Taken::Value("42".into()), "Dev"),
            ("new_one", &Taken::Value("200".into()), "Dev"),
        ],
        "each name goes where its value came from; new names to the active environment"
    );
    write_captures(&store.handle(), executed.writes)
        .await
        .unwrap();

    let values = inputs(&store.handle(), &login)
        .await
        .unwrap()
        .snapshot
        .values();
    assert_eq!(values["token"], "fresh");
    assert_eq!(values["scope"], "42");
    assert_eq!(values["new_one"], "200");
    let mut next = login;
    next.template.captures.clear();
    execute(&store.handle(), next, CancellationToken::new())
        .await
        .unwrap();
    let sent = server.requests().remove(1);
    assert_eq!(sent.body, br#"{"token": "fresh"}"#);
    assert_eq!(sent.header("x-scope"), Some("42"));
}

#[tokio::test]
async fn generated_values_are_the_same_wherever_one_send_uses_them() {
    use imposter_core::check::{Check, Op, Verdict};
    use imposter_core::select::Selector;
    let (_dir, store) = open();
    let server = Server::start(|_, received| {
        // Echo the id the request carried in its header.
        let id = received.header("x-id").unwrap_or_default().to_owned();
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            format!("{{\"id\":\"{id}\"}}").as_bytes(),
        )
    })
    .await;
    let mut request = setup(&store, &server.url).await;
    request.template.url = "{{base}}/items/{{$uuid}}?n={{$randomInt 1 9}}&other={{$uuid b}}".into();
    request.template.headers = vec![ParamItem::new("X-Id", "{{$uuid}}")];
    request.template.body = b"{\"id\": \"{{$uuid}}\", \"at\": {{$timestamp}}}".to_vec();
    request.template.checks = vec![Check::new(
        Selector::Json("$.id".into()),
        Op::Equals,
        "{{$uuid}}",
    )];

    let executed = execute(&store.handle(), request.clone(), CancellationToken::new())
        .await
        .unwrap();
    let received = server.requests().remove(0);
    let id = received.header("x-id").unwrap().to_owned();
    assert_eq!(id.len(), 36);
    let (path, query) = received.request_line.split_once('?').unwrap();
    assert!(path.ends_with(&format!("/items/{id}")), "{path}");
    assert!(
        !query.contains(&id),
        "a labelled expression is another value: {query}"
    );
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    assert_eq!(body["id"], id.as_str());
    assert!(body["at"].as_i64().unwrap() > 1_700_000_000);
    assert_eq!(
        executed.response.checks[0].verdict,
        Verdict::Passed,
        "checks see the sent value"
    );

    // Another send generates anew.
    execute(&store.handle(), request.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_ne!(server.requests().remove(1).header("x-id").unwrap(), id);

    // A name the user defined wins; an unknown one stops the send.
    store
        .call(|db| environments::set_globals(db, &vars(&[("$uuid", "mine")])))
        .await
        .unwrap();
    let with_url = |url: &str| Request {
        template: RequestTemplate {
            url: url.into(),
            ..request.template.clone()
        },
        ..request.clone()
    };
    execute(
        &store.handle(),
        with_url("{{base}}/{{$uuid}}"),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let received = server.requests().remove(2);
    assert!(
        received.request_line.starts_with("POST /mine "),
        "{}",
        received.request_line
    );
    assert_eq!(received.header("x-id"), Some("mine"));
    let error = execute(
        &store.handle(),
        with_url("{{base}}/{{$randomFirstName}}"),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Unresolved variables: $randomFirstName");
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

    assert!(matches!(first, Err(ExecuteError::Unresolved(ref names)) if names == &["missing"]));
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
        imposter_testkit::read_request(&mut stream).await;
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

#[tokio::test]
async fn copy_as_curl_resolves_what_send_would_send_without_sending() {
    let (_dir, store) = open();
    let request = setup(&store, "http://h").await;

    let command = imposter_app::executor::curl_command(&store.handle(), &request)
        .await
        .unwrap();

    assert!(command.starts_with("curl 'http://h/items'"), "{command}");
    assert!(command.contains("-H 'X-Scope: environment'"), "{command}");
    assert!(
        command.contains(r#"--data-raw '{"token": "global-token"}'"#),
        "{command}"
    );
    assert_eq!(
        history_count(&store).await,
        0,
        "nothing is sent or recorded"
    );

    let mut broken = request.clone();
    broken.template.url = "{{missing}}/x".into();
    let error = imposter_app::executor::curl_command(&store.handle(), &broken).await;
    assert_eq!(
        error.unwrap_err().to_string(),
        "Unresolved variables: missing",
        "the message stays the same"
    );
}

#[tokio::test]
async fn host_identity_uses_the_resolved_url_and_is_preserved_in_history_and_curl() {
    use imposter_core::settings::HostIdentity;
    let (dir, store) = open();
    let pki = imposter_testkit::Pki::generate(dir.path());
    let (url, server) =
        imposter_testkit::https_once(pki.server_config(true), imposter_testkit::OK_RESPONSE).await;
    let url = url.replace("localhost", "127.0.0.1");
    let port = url.rsplit(':').next().unwrap().parse().unwrap();
    let certificate = pki.client_cert_pem.display().to_string();
    let key = pki.client_key_pem.display().to_string();
    let request = setup(&store, &url).await;
    let expected = certificate.clone();
    store
        .call(move |db| {
            environments::set_globals(
                db,
                &vars(&[
                    ("certificate", &certificate),
                    ("key", &key),
                    ("only_global", "g"),
                    ("token", "token"),
                ]),
            )?;
            kv::set_app_settings(
                db,
                &AppSettings {
                    verify_ssl: false,
                    certificate_path: "{{unused_global}}".into(),
                    host_identities: vec![HostIdentity {
                        host: "127.0.0.1".into(),
                        port,
                        certificate_path: "{{certificate}}".into(),
                        key_file: "{{key}}".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    let captured = inputs(&store.handle(), &request).await.unwrap();
    let (_, effective) = imposter_app::executor::resolve(&request, &captured).unwrap();
    assert_eq!(effective.certificate_path, expected);
    let (command, notice) =
        imposter_app::executor::curl_with_identity_notice(&store.handle(), &request)
            .await
            .unwrap();
    assert!(notice);
    assert_eq!(
        imposter_core::curl::parse(&command)
            .unwrap()
            .template
            .options
            .certificate_path,
        Some(expected.clone())
    );
    let result = execute(&store.handle(), request.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result.response.status, 200);
    server.await.unwrap().unwrap();
    let id = result.record.unwrap();
    let record = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.settings.certificate_path, expected);
    assert_eq!(
        imposter_core::curl::parse(&imposter_app::executor::recorded_curl(&record).unwrap())
            .unwrap()
            .template
            .options
            .certificate_path,
        Some(expected.clone())
    );
    store
        .call(|db| environments::set_globals(db, &Variables::new()))
        .await
        .unwrap();
    assert_eq!(
        imposter_app::executor::resolve(&request, &captured)
            .unwrap()
            .1
            .certificate_path,
        expected
    );
}

#[tokio::test]
async fn a_chosen_saved_value_is_what_the_next_send_uses_in_its_own_environment() {
    use imposter_core::model::SavedValues;
    use imposter_core::variables::Scope;
    use imposter_store::scopes;

    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let (_dir, store) = open();
    let base = server.url.clone();
    let (dev, prod, request) = store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("base", &base)]))?;
            let dev = environments::create_environment(db, "Dev", &vars(&[("user", "42")]))?.id;
            let prod = environments::create_environment(db, "Prod", &vars(&[("user", "1")]))?.id;
            let mut saved = SavedValues::default();
            saved.set("user", ["42".to_owned(), "77".to_owned()]);
            scopes::write(
                db,
                Scope::Environment(dev),
                &vars(&[("user", "42")]),
                &saved,
            )?;
            environments::set_active_environment(db, Some(dev))?;
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            let template = RequestTemplate {
                name: "User".into(),
                url: "{{base}}/users/{{user}}".into(),
                body_type: BodyType::None,
                ..RequestTemplate::default()
            };
            let request = collections::create_request(db, api.id, None, &template)?;
            Ok((dev, prod, request))
        })
        .await
        .unwrap();

    let effective = inputs(&store.handle(), &request).await.unwrap().snapshot;
    assert_eq!(effective.effective("user").unwrap().saved, ["42", "77"]);
    store
        .call(move |db| scopes::choose_value(db, Scope::Environment(dev), "user", "77"))
        .await
        .unwrap();
    execute(&store.handle(), request.clone(), CancellationToken::new())
        .await
        .unwrap();
    store
        .call(move |db| environments::set_active_environment(db, Some(prod)))
        .await
        .unwrap();
    let in_prod = inputs(&store.handle(), &request).await.unwrap().snapshot;
    assert_eq!(
        in_prod.effective("user").unwrap().saved,
        Vec::<String>::new(),
        "Dev's values are not offered in Prod"
    );
    execute(&store.handle(), request, CancellationToken::new())
        .await
        .unwrap();

    let targets: Vec<String> = server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets, ["/users/77", "/users/1"]);
}
