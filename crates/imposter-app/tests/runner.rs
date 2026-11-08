//! Running a folder or a collection as one run: the tree's order, values carried from one
//! request to the next, stopping at the first failure unless told to keep going, the same
//! steps from the profile and a file, and History for a profile run.
#![cfg(test)]

use imposter_app::document::{DocumentError, RequestPath, select_all};
use imposter_app::profile::{self, ProfileError};
use imposter_app::runner::{Options, SKIPPED_AFTER_FAILURE, Step, run};
use imposter_app::transfer::{import, read_preview};
use imposter_core::execution::Via;
use imposter_core::model::Variables;
use imposter_core::run_result::RunOutcome;
use imposter_store::{Store, collections, executions};
use imposter_testkit::{Received, Server, response};
use imposter_transfer::native::{self, NativeDocument};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn reply(_: usize, received: &Received) -> Vec<u8> {
    let json = [("Content-Type", "application/json")];
    match (received.method(), received.target()) {
        ("POST", "/items") => {
            response("201 Created", &json, br#"{"id": 42, "token": "run-token"}"#)
        }
        ("GET", "/items/42") => response("200 OK", &json, br#"{"id": 42}"#),
        ("GET", "/fail") => response("500 Internal Server Error", &[], b""),
        _ => response("200 OK", &[], b""),
    }
}

fn request(name: &str, method: &str, url: &str, position: i64, extra: Value) -> Value {
    let mut request = json!({
        "name": name, "method": method, "url": url, "position": position,
        "body": "", "body_type": "none",
    });
    if let (Some(map), Value::Object(extra)) = (request.as_object_mut(), extra) {
        map.extend(extra);
    }
    request
}

/// "Flow": Create (captures id and token), then the Nested folder's Read (uses the id),
/// then Echo (uses the token); "Broken": Fails, then After; and an empty folder.
fn document(base: &str) -> NativeDocument {
    native::decode_value(json!({
        "format": "imposter",
        "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": base},
            "folders": [
                {
                    "name": "Flow", "position": 0,
                    "requests": [
                        request("Create", "POST", "{{base}}/items", 0, json!({
                            "checks": [{"selector": "status", "op": "equals", "expected": "201"}],
                            "captures": [
                                {"variable": "id", "selector": {"json": "$.id"}},
                                {"variable": "token", "selector": {"json": "$.token"}}
                            ]
                        })),
                        request("Echo", "GET", "{{base}}/echo?token={{token}}", 2, json!({}))
                    ],
                    "folders": [{
                        "name": "Nested", "position": 1,
                        "requests": [request("Read", "GET", "{{base}}/items/{{id}}", 0, json!({
                            "checks": [{"selector": {"json": "$.id"}, "op": "equals", "expected": "{{id}}"}]
                        }))]
                    }]
                },
                {
                    "name": "Broken", "position": 1,
                    "requests": [
                        request("Fails", "GET", "{{base}}/fail", 0, json!({
                            "checks": [{"selector": "status", "op": "equals", "expected": "200"}]
                        })),
                        request("After", "GET", "{{base}}/after", 1, json!({}))
                    ]
                },
                {"name": "Empty", "position": 2}
            ]
        },
        "environments": [
            {"name": "Dev", "variables": {"token": "env-token"}},
            {"name": "Prod", "variables": {}, "color": "red"}
        ]
    }))
    .unwrap()
}

fn path(text: &str) -> RequestPath {
    RequestPath::parse(text).unwrap()
}

fn targets(server: &Server) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|r| format!("{} {}", r.method(), r.target()))
        .collect()
}

fn outcomes(steps: &[Step]) -> Vec<(String, Option<RunOutcome>)> {
    steps
        .iter()
        .map(|step| {
            let path = match step {
                Step::Ran { path, .. }
                | Step::NotSent { path, .. }
                | Step::Skipped { path, .. } => path.clone(),
            };
            (path, step.outcome())
        })
        .collect()
}

#[tokio::test]
async fn a_folder_runs_in_tree_order_and_carries_captured_values() {
    let server = Server::start(reply).await;
    let document = document(&server.url);
    let input = [("token".to_owned(), "input-token".to_owned())]
        .into_iter()
        .collect();
    let steps = select_all(&document, &path("Flow"), Some("Dev"), &input).unwrap();
    let mut seen = Vec::new();
    let ran = run(
        steps,
        Options::default(),
        CancellationToken::new(),
        |step| {
            seen.push(step.outcome());
        },
    )
    .await;
    assert_eq!(
        targets(&server),
        ["POST /items", "GET /items/42", "GET /echo?token=run-token"],
        "folders and requests mixed by position; the captured token beats the input and Dev"
    );
    assert_eq!(
        outcomes(&ran),
        [
            ("API/Flow/Create".into(), Some(RunOutcome::Passed)),
            ("API/Flow/Nested/Read".into(), Some(RunOutcome::Passed)),
            ("API/Flow/Echo".into(), Some(RunOutcome::NoChecks)),
        ]
    );
    assert_eq!(seen.len(), 3, "each step is reported as it ends");
}

#[tokio::test]
async fn a_failure_stops_the_run_unless_it_keeps_going() {
    let server = Server::start(reply).await;
    let document = document(&server.url);
    let none = Variables::new();
    let stopped = run(
        select_all(&document, &path("API / Broken"), None, &none).unwrap(),
        Options::default(),
        CancellationToken::new(),
        |_| {},
    )
    .await;
    assert_eq!(targets(&server), ["GET /fail"]);
    assert!(matches!(&stopped[1], Step::Skipped { reason, .. } if reason == SKIPPED_AFTER_FAILURE));

    let kept = run(
        select_all(&document, &path("Broken"), None, &none).unwrap(),
        Options {
            keep_going: true,
            record: None,
        },
        CancellationToken::new(),
        |_| {},
    )
    .await;
    assert_eq!(targets(&server), ["GET /fail", "GET /fail", "GET /after"]);
    assert_eq!(kept[1].outcome(), Some(RunOutcome::NoChecks));

    let unresolved = run(
        select_all(&document, &path("API/Flow/Nested/Read"), None, &none).unwrap(),
        Options::default(),
        CancellationToken::new(),
        |_| {},
    )
    .await;
    assert!(
        matches!(&unresolved[0], Step::NotSent { outcome: RunOutcome::NotSent, error, .. } if error.contains("id")),
        "a single request is a run of one; without Create its id is unresolved"
    );
    assert_eq!(
        select_all(&document, &path("Empty"), None, &none)
            .unwrap()
            .len(),
        0,
        "an empty folder selects nothing"
    );
    assert!(matches!(
        select_all(&document, &path("Nope"), None, &none),
        Err(DocumentError::NoTarget { .. })
    ));
}

#[tokio::test]
async fn a_cancelled_run_skips_the_rest() {
    let server = Server::start(reply).await;
    let document = document(&server.url);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let steps = run(
        select_all(&document, &path("Flow"), None, &Variables::new()).unwrap(),
        Options::default(),
        cancel,
        |_| {},
    )
    .await;
    assert!(steps.iter().all(|s| matches!(s, Step::Skipped { .. })));
    assert_eq!(targets(&server), Vec::<String>::new());
}

#[tokio::test]
async fn the_profile_runs_the_same_steps_and_records_them_frozen_at_the_start() {
    let server = Server::start(reply).await;
    let document = document(&server.url);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("api.imposter.json");
    std::fs::write(&file, native::encode(&document)).unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let preview = read_preview(&store.handle(), file, Vec::new())
        .await
        .unwrap();
    import(&store.handle(), preview, false).await.unwrap();
    let none = Variables::new();

    let from_profile =
        profile::select_all(&store.handle(), &path("API / Flow"), Some("Dev"), &none)
            .await
            .unwrap();
    let from_file = select_all(&document, &path("Flow"), Some("Dev"), &none).unwrap();
    let names = |steps: &[imposter_app::document::Selected]| -> Vec<String> {
        steps.iter().map(|s| s.path.to_string()).collect()
    };
    assert_eq!(names(&from_profile), names(&from_file));

    // An edit after the run read its requests changes nothing.
    store
        .call(|db| {
            for request in collections::requests(db, collections::collections(db)?[0].id)? {
                let mut template = request.template.clone();
                template.url = "http://127.0.0.1:1/edited".into();
                collections::update_request(db, request.id.unwrap(), &template)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let ran = run(
        from_profile,
        Options {
            keep_going: false,
            record: Some((store.handle(), Via::CommandLine)),
        },
        CancellationToken::new(),
        |_| {},
    )
    .await;
    assert_eq!(targets(&server).len(), 3, "the original URLs were used");
    assert!(
        ran.iter()
            .all(|s| s.outcome().is_some_and(RunOutcome::continues_run))
    );
    let history = store
        .call(|db| executions::page(db, None, 0, 10))
        .await
        .unwrap();
    assert_eq!(history.len(), 3);
    assert!(history.iter().all(|h| h.via == Via::CommandLine));

    assert!(matches!(
        profile::select_all(&store.handle(), &path("API"), Some("Prod"), &none).await,
        Err(ProfileError::Production { .. })
    ));
    assert!(matches!(
        profile::select_all(&store.handle(), &path("Nope"), None, &none).await,
        Err(ProfileError::Path(DocumentError::NoTarget { .. }))
    ));
    assert_eq!(
        profile::select_all(&store.handle(), &path("API"), None, &none)
            .await
            .unwrap()
            .len(),
        5,
        "the whole collection"
    );
}

/// The app names a run's target by id: the same requests as by path, and a run started in
/// the app is recorded as Send's runs are.
#[tokio::test]
async fn the_app_selects_by_id_and_records_its_runs_as_its_own() {
    let server = Server::start(reply).await;
    let document = document(&server.url);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("api.imposter.json");
    std::fs::write(&file, native::encode(&document)).unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let preview = read_preview(&store.handle(), file, Vec::new())
        .await
        .unwrap();
    import(&store.handle(), preview, false).await.unwrap();
    let (collection, flow) = store
        .call(|db| {
            let collection = collections::collections(db)?[0].id;
            let flow = collections::tree(db, collection)?
                .unwrap()
                .folders
                .into_iter()
                .find(|f| f.name == "Flow")
                .map(|f| f.id);
            Ok((collection, flow))
        })
        .await
        .unwrap();
    let by_id = profile::select_under(&store.handle(), collection, flow, None)
        .await
        .unwrap();
    let by_path = profile::select_all(&store.handle(), &path("API/Flow"), None, &Variables::new())
        .await
        .unwrap();
    let names = |steps: &[imposter_app::document::Selected]| -> Vec<String> {
        steps.iter().map(|s| s.path.to_string()).collect()
    };
    assert_eq!(names(&by_id), names(&by_path));
    let whole = profile::select_under(&store.handle(), collection, None, None)
        .await
        .unwrap();
    assert_eq!(whole.len(), 5);

    run(
        by_id,
        Options {
            keep_going: false,
            record: Some((store.handle(), Via::App)),
        },
        CancellationToken::new(),
        |_| {},
    )
    .await;
    let history = store
        .call(|db| executions::page(db, None, 0, 10))
        .await
        .unwrap();
    assert_eq!(history.len(), 3);
    assert!(history.iter().all(|h| h.via == Via::App));
}
