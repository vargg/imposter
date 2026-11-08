//! Request scripts end to end: a real database, a loopback server, and what History keeps.
#![cfg(test)]

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use imposter_app::executor::{ExecuteError, Executed, curl_command, execute, script_host};
use imposter_core::execution::Outcome;
use imposter_core::model::{BodyType, Request, RequestTemplate, Variables};
use imposter_core::variables::Scope;
use imposter_store::{Store, collections, environments, executions};
use imposter_testkit::{Received, Server, response};
use tokio_util::sync::CancellationToken;

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    (dir, store)
}

/// A saved POST with a JSON body, the global `secret` and an active `Dev` environment.
async fn saved(store: &Store, url: &str, before: &str, after: &str) -> Request {
    let template = RequestTemplate {
        name: "signed".into(),
        method: "POST".into(),
        url: format!("{url}/items"),
        body: br#"{"n":1}"#.to_vec(),
        body_type: BodyType::Json,
        scripts: imposter_core::model::Scripts {
            before: before.into(),
            after: after.into(),
        },
        ..RequestTemplate::default()
    };
    store
        .call(move |db| {
            let globals: Variables = [("secret".to_owned(), "s3cret".to_owned())]
                .into_iter()
                .collect();
            environments::set_globals(db, &globals)?;
            let dev = environments::create_environment(db, "Dev", &Variables::new())?;
            environments::set_active_environment(db, Some(dev.id))?;
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(db, collection.id, None, &template)
        })
        .await
        .unwrap()
}

async fn ok_server() -> Server {
    Server::start(|_, _| {
        response(
            "201 Created",
            &[("Content-Type", "application/json")],
            br#"{"id": 7}"#,
        )
    })
    .await
}

async fn send(store: &Store, request: Request) -> Result<Executed, ExecuteError> {
    execute(&store.handle(), request, CancellationToken::new()).await
}

fn received(server: &Server) -> Received {
    server.requests().into_iter().next().unwrap()
}

#[tokio::test]
async fn a_before_script_signs_and_changes_what_is_sent_and_recorded() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        r#"request.set_header("X-Signature", hex(hmac_sha256(vars.get("secret"), request.body)));
request.set_query("v", "2");"#,
        "",
    )
    .await;
    let executed = send(&store, request).await.unwrap();

    let arrived = received(&server);
    assert_eq!(
        arrived.header("x-signature"),
        Some("8f0119179f72a1ee1ddd6ea193d72c5430227f4ecb9255eaa54499b96559c4ad")
    );
    assert_eq!(arrived.target(), "/items?v=2");
    assert!(
        executed
            .sent
            .template
            .headers
            .iter()
            .any(|h| h.name == "X-Signature")
    );
    let record = store
        .call(move |db| executions::execution(db, executed.record.unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert!(
        record.resolved.template.url.ends_with("/items?v=2"),
        "History keeps the request as sent"
    );
}

#[tokio::test]
async fn a_failing_before_script_sends_and_records_nothing() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(&store, &server.url, "let a = 1;\nthrow \"no key\";", "").await;
    match send(&store, request).await {
        Err(ExecuteError::Script(message)) => {
            assert_eq!(message, "Before-sending script, line 2: no key");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(server.requests().len(), 0);
    let count = store.call(|db| executions::count(db)).await.unwrap();
    assert_eq!(count, 0, "a request that was not sent is not history");
}

#[tokio::test]
async fn cancel_stops_a_busy_script() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        "let i = 0; while i < 900000 { i += 1; }",
        "",
    )
    .await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = execute(&store.handle(), request, cancel).await;
    assert!(
        matches!(&result, Err(ExecuteError::Script(m)) if m.ends_with("Stopped: the send was cancelled")),
        "{result:?}"
    );
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn the_script_host_stops_at_its_deadline() {
    let host = script_host(CancellationToken::new(), Duration::ZERO);
    assert_eq!((host.stop)(), Some(imposter_app::script::Stop::TimeLimit));
    let host = script_host(CancellationToken::new(), Duration::from_secs(60));
    assert_eq!((host.stop)(), None);
}

#[tokio::test]
async fn after_script_tests_are_kept_but_its_console_is_not() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        r#"vars.set("stamp", "early");"#,
        r#"print("token " + vars.get("stamp"));
test("created", response.status == 201);
test("id", || { expect(response.path("$.id"), 8) });
vars.set("item_id", response.path("$.id"));
vars.set("secret", "rotated");"#,
    )
    .await;
    let executed = send(&store, request).await.unwrap();
    let report = executed.response.script.clone().unwrap();
    let tests: Vec<(&str, bool)> = report
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.passed))
        .collect();
    assert_eq!(tests, [("created", true), ("id", false)]);
    assert_eq!(report.tests[1].message, "expected 8, got 7");
    assert_eq!(executed.response.console, ["token early"]);

    let id = executed.record.unwrap();
    let record = store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.response.script, Some(report));
    assert_eq!(record.response.console, Vec::<String>::new());

    // Sets go where captures go: the defining scope, else the active environment.
    let writes: Vec<(Option<Scope>, &str, &str)> = executed
        .writes
        .iter()
        .map(|w| (w.scope, w.name.as_str(), w.value.as_str()))
        .collect();
    assert!(
        matches!(writes[0], (Some(Scope::Environment(_)), "stamp", "early")),
        "{writes:?}"
    );
    assert!(
        matches!(writes[1], (Some(Scope::Environment(_)), "item_id", "7")),
        "{writes:?}"
    );
    assert_eq!(writes[2], (Some(Scope::Global), "secret", "rotated"));
}

#[tokio::test]
async fn an_after_script_error_keeps_the_response() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        "",
        "test(\"a\", true);\nresponse.nope();",
    )
    .await;
    let executed = send(&store, request).await.unwrap();
    assert_eq!(executed.response.status, 201);
    let report = executed.response.script.unwrap();
    assert_eq!(report.tests.len(), 1);
    assert!(
        report.error.starts_with("After-response script, line 2:"),
        "{}",
        report.error
    );
    assert_eq!(report.failures(), 1);
}

#[tokio::test]
async fn no_after_script_runs_without_a_response() {
    let (_dir, store) = open();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let request = saved(&store, &url, "", "test(\"never\", true);").await;
    let executed = send(&store, request).await.unwrap();
    assert_eq!(executed.response.outcome, Outcome::TransportError);
    assert_eq!(executed.response.script, None);
}

#[tokio::test]
async fn copy_as_curl_includes_the_before_scripts_changes() {
    let (_dir, store) = open();
    let request = saved(
        &store,
        "http://example.test",
        r#"request.set_header("X-Signed", "yes");"#,
        "",
    )
    .await;
    let command = curl_command(&store.handle(), &request).await.unwrap();
    assert!(command.contains("X-Signed: yes"), "{command}");
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn script_output_never_reaches_the_logs() {
    let capture = Capture::default();
    let writer = capture.clone();
    tracing_subscriber::fmt()
        .with_env_filter("trace")
        .with_writer(move || writer.clone())
        .try_init()
        .unwrap();
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        r#"print("printed-secret-before"); debug("debugged-secret");"#,
        r#"print("printed-secret-after");"#,
    )
    .await;
    send(&store, request).await.unwrap();
    let mut logs = capture.0.lock().unwrap().clone();
    logs.flush().unwrap();
    let logs = String::from_utf8_lossy(&logs);
    for secret in [
        "printed-secret-before",
        "debugged-secret",
        "printed-secret-after",
    ] {
        assert!(!logs.contains(secret), "{secret} reached the logs");
    }
}

fn scripts(before: &str, after: &str) -> imposter_core::model::Scripts {
    imposter_core::model::Scripts {
        before: before.into(),
        after: after.into(),
    }
}

/// API (collection) > Outer > Inner > "nested" request, each level with `levels` scripts.
async fn nested(store: &Store, url: &str, levels: [imposter_core::model::Scripts; 4]) -> Request {
    let url = url.to_owned();
    store
        .call(move |db| {
            let [collection, outer, inner, own] = levels;
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::set_collection_scripts(db, api.id, &collection)?;
            let first = collections::create_folder(db, api.id, None, "Outer", "", 0)?;
            collections::set_folder_scripts(db, first.id, &outer)?;
            let second = collections::create_folder(db, api.id, Some(first.id), "Inner", "", 0)?;
            collections::set_folder_scripts(db, second.id, &inner)?;
            let template = RequestTemplate {
                name: "nested".into(),
                url: format!("{url}/items"),
                scripts: own,
                ..RequestTemplate::default()
            };
            collections::create_request(db, api.id, Some(second.id), &template)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn collection_and_folder_scripts_run_outer_first_around_the_requests_own() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = nested(
        &store,
        &server.url,
        [
            scripts(
                r#"print("collection"); request.set_header("X-Collection", "1"); vars.set("token", "t1");"#,
                r#"print("collection after"); test("from the collection", true);"#,
            ),
            scripts(
                r#"print("outer"); request.set_header("X-Outer", request.header("X-Collection") + "2");"#,
                "",
            ),
            scripts(
                r#"print("inner"); request.set_header("X-Inner", vars.get("token"));"#,
                r#"print("inner after");"#,
            ),
            scripts(
                r#"print("request");"#,
                r#"print("request after"); test("own", response.status == 201);"#,
            ),
        ],
    )
    .await;
    let executed = send(&store, request).await.unwrap();
    let arrived = received(&server);
    assert_eq!(arrived.header("x-collection"), Some("1"));
    assert_eq!(
        arrived.header("x-outer"),
        Some("12"),
        "inner levels see outer changes"
    );
    assert_eq!(arrived.header("x-inner"), Some("t1"), "and outer variables");
    assert_eq!(
        executed.response.console,
        [
            "collection",
            "outer",
            "inner",
            "request",
            "collection after",
            "inner after",
            "request after"
        ]
    );
    let tests: Vec<&str> = executed
        .response
        .script
        .as_ref()
        .unwrap()
        .tests
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(tests, ["from the collection", "own"], "pooled in run order");
}

#[tokio::test]
async fn a_failing_folder_script_is_named_and_nothing_is_sent() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = nested(
        &store,
        &server.url,
        [
            scripts("", ""),
            scripts("", ""),
            scripts("let a = 1;\nthrow \"no key\";", ""),
            scripts(r#"request.set_header("X", "1");"#, ""),
        ],
    )
    .await;
    match send(&store, request).await {
        Err(ExecuteError::Script(message)) => assert_eq!(
            message,
            "Folder \"Inner\" before-sending script, line 2: no key"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(server.requests().len(), 0);
}

#[tokio::test]
async fn a_failing_after_script_does_not_stop_the_others() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = nested(
        &store,
        &server.url,
        [
            scripts("", "throw \"collection says no\";"),
            scripts("", "response.nope();"),
            scripts("", ""),
            scripts("", r#"test("own", true);"#),
        ],
    )
    .await;
    let executed = send(&store, request).await.unwrap();
    let report = executed.response.script.unwrap();
    assert_eq!(report.tests.len(), 1, "the request's own test still ran");
    let errors: Vec<&str> = report.error.lines().collect();
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors[0],
        "Collection \"API\" after-response script, line 1: collection says no"
    );
    assert!(errors[1].starts_with("Folder \"Outer\" after-response script"));
    assert_eq!(report.failures(), 2);
}

#[tokio::test]
async fn copy_as_curl_runs_the_whole_chain() {
    let (_dir, store) = open();
    let request = nested(
        &store,
        "http://example.test",
        [
            scripts(r#"request.set_header("X-Collection", "1");"#, ""),
            scripts("", ""),
            scripts(r#"request.set_header("X-Inner", "2");"#, ""),
            scripts("", ""),
        ],
    )
    .await;
    let command = curl_command(&store.handle(), &request).await.unwrap();
    assert!(
        command.contains("X-Collection: 1") && command.contains("X-Inner: 2"),
        "{command}"
    );
}

#[tokio::test]
async fn a_request_whose_folder_is_gone_keeps_its_collections_scripts() {
    let (_dir, store) = open();
    let server = ok_server().await;
    let request = nested(
        &store,
        &server.url,
        [
            scripts(r#"request.set_header("X-Collection", "1");"#, ""),
            scripts(r#"request.set_header("X-Outer", "1");"#, ""),
            scripts("", ""),
            scripts("", ""),
        ],
    )
    .await;
    // As a History draft would be: the request and its folders deleted, the collection kept.
    let folder = request.folder_id.unwrap();
    store
        .call(move |db| {
            let outer = collections::folder(db, folder)?.unwrap().parent_id.unwrap();
            collections::delete_folder(db, outer)
        })
        .await
        .unwrap();
    let mut draft = request;
    draft.id = None;
    send(&store, draft).await.unwrap();
    let arrived = received(&server);
    assert_eq!(arrived.header("x-collection"), Some("1"));
    assert_eq!(
        arrived.header("x-outer"),
        None,
        "a deleted folder's scripts are gone"
    );
}

/// `secrets.get` reads a stored secret to sign with; `secrets.set` stores a token as a
/// secret of the active environment, never a variable; History holds neither value.
#[tokio::test]
async fn scripts_read_and_store_secrets_that_history_never_keeps() {
    use imposter_core::variables::SecretScope;
    use imposter_store::{scopes, secrets};

    let (_dir, store) = open();
    let server = ok_server().await;
    let request = saved(
        &store,
        &server.url,
        r#"request.set_header("x-signature", hex(hmac_sha256(secrets.get("signing_key"), request.body)));
           request.set_header("x-key", secrets.get("signing_key"));"#,
        r#"secrets.set("issued", "tok-" + response.path("$.id") + "-abcdef");
           test("seen at once", secrets.get("issued") == "tok-" + response.path("$.id") + "-abcdef");"#,
    )
    .await;
    let dev = store
        .call(|db| {
            let dev = environments::active_environment(db)?.unwrap();
            secrets::write(
                db,
                SecretScope::Global,
                &[("signing_key".to_owned(), "key-123456".to_owned())]
                    .into_iter()
                    .collect(),
            )?;
            Ok(dev)
        })
        .await
        .unwrap();
    let executed = send(&store, request).await.unwrap();
    let received = received(&server);
    assert_eq!(received.header("x-key"), Some("key-123456"));
    assert_eq!(received.header("x-signature").map(str::len), Some(64));
    let tests = &executed.response.script.as_ref().unwrap().tests;
    assert!(tests[0].passed, "{tests:?}");
    let issued = executed.writes.iter().find(|w| w.name == "issued").unwrap();
    assert!(issued.secret);
    imposter_app::executor::write_captures(&store.handle(), executed.writes.clone())
        .await
        .unwrap();
    let id = executed.record.unwrap();
    let (stored, variables, payload) = store
        .call(move |db| {
            Ok((
                secrets::read(db, SecretScope::Environment(dev))?,
                scopes::read(db, Scope::Environment(dev))?.0,
                db.query_row(
                    "SELECT payload FROM executions WHERE id = ?1",
                    [id.0],
                    |row| row.get::<_, String>(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        stored.get("issued").map(String::as_str),
        Some("tok-7-abcdef")
    );
    assert!(variables.is_empty(), "a secret set is not a variable");
    for secret in ["key-123456", "tok-7-abcdef"] {
        assert!(!payload.contains(secret), "{secret} reached History");
    }
}
