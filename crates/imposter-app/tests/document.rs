//! Running a request straight from a collection file: the same engine as Send, with inputs
//! read from the file instead of the database.
#![cfg(test)]

use imposter_app::document::{DocumentError, RequestPath, request_paths, select};
use imposter_app::executor::{Writes, execute, run};
use imposter_app::transfer::{import, read_preview};
use imposter_core::capture::Taken;
use imposter_core::check::Verdict;
use imposter_core::model::Variables;
use imposter_store::{Store, collections, environments};
use imposter_testkit::{Received, Server, response};
use imposter_transfer::native::{self, NativeDocument};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn request(name: &str, url: &str) -> serde_json::Value {
    json!({
        "name": name,
        "method": "POST",
        "url": url,
        "headers": [{"name": "X-Scope", "value": "{{scope}}", "is_enabled": true}],
        "body": "",
        "body_type": "json",
    })
}

/// A collection with variables, scripts at every level, checks that pass, fail and cannot
/// be decided, a capture, globals and two environments.
fn document(base: &str) -> NativeDocument {
    let mut create = request("Create user", "{{base}}/users");
    create["checks"] = json!([
        {"selector": "status", "op": "equals", "expected": "201"},
        {"selector": {"json": "$.id"}, "op": "equals", "expected": "8"},
        {"selector": {"json": "nope"}, "op": "exists"},
    ]);
    create["captures"] = json!([{"variable": "token", "selector": {"json": "$.token"}}]);
    create["scripts"] = json!({
        "after": "test(\"created\", response.status == 201);\nvars.set(\"seen\", \"yes\");"
    });
    native::decode_value(json!({
        "format": "imposter",
        "version": 3,
        "collection": {
            "name": "API",
            "variables": {"base": base, "scope": "collection"},
            "requests": [request("Ping", "{{base}}/ping")],
            "folders": [
                {
                    "name": "Users",
                    "requests": [create],
                    "folders": [
                        {"name": "a/b", "requests": [request("Ping", "{{base}}/nested")]}
                    ],
                    "scripts": {"before": "request.set_header(\"X-Folder\", \"users\");"}
                },
                {"name": "Twins", "requests": [request("Same", "{{base}}/1"), request("Same", "{{base}}/2")]}
            ],
            "scripts": {"before": "request.set_header(\"X-Collection\", vars.get(\"secret\"));"}
        },
        "environments": [
            {"name": "Dev", "variables": {"scope": "environment"}},
            {"name": "Prod", "variables": {"scope": "production"}}
        ],
        "globals": {"scope": "global", "secret": "s3cret"}
    }))
    .unwrap()
}

fn path(text: &str) -> RequestPath {
    RequestPath::parse(text).unwrap()
}

#[test]
fn request_paths_name_folders_and_escape_slashes() {
    let document = document("http://127.0.0.1:1");
    assert_eq!(
        request_paths(&document),
        [
            "Ping",
            "Users/Create user",
            "Users/a\\/b/Ping",
            "Twins/Same",
            "Twins/Same"
        ]
    );
    assert_eq!(path("Users/a\\/b/Ping").to_string(), "Users/a\\/b/Ping");
    assert_eq!(path("a\\\\b").to_string(), "a\\\\b");
    for bad in ["", "/Ping", "Users/", "a//b", "a\\b", "a\\", "a/ /b"] {
        assert_eq!(
            RequestPath::parse(bad),
            Err(DocumentError::BadPath(bad.to_owned())),
            "{bad:?}"
        );
    }
}

#[test]
fn a_path_or_environment_that_names_nothing_or_several_is_refused() {
    let document = document("http://127.0.0.1:1");
    let none = Variables::new();
    let missing = select(&document, &path("Users/Delete"), None, &none).unwrap_err();
    assert_eq!(
        missing.to_string(),
        "No request at Users/Delete. Requests: Ping, Users/Create user, \
         Users/a\\/b/Ping, Twins/Same, Twins/Same"
    );
    assert_eq!(
        select(&document, &path("Twins/Same"), None, &none).unwrap_err(),
        DocumentError::Ambiguous {
            path: "Twins/Same".into(),
            count: 2
        }
    );
    assert_eq!(
        select(&document, &path("Ping"), Some("Staging"), &none)
            .unwrap_err()
            .to_string(),
        "No environment named \"Staging\". Environments: Dev, Prod"
    );
    let found = select(&document, &path("Users/a\\/b/Ping"), None, &none).unwrap();
    assert_eq!(found.request.template.url, "{{base}}/nested");
    assert_eq!(found.inputs.writes, Writes::Run);
    let copied = select(
        &document,
        &path(" API / Users / a\\/b / Ping "),
        None,
        &none,
    )
    .unwrap();
    assert_eq!(
        copied.request, found.request,
        "a copied breadcrumb, with the collection and spaces, names the same request"
    );
    assert_eq!(
        found
            .inputs
            .scripts
            .0
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["Collection \"API\"", "Folder \"Users\"", "Folder \"a/b\""]
    );
}

fn headers(received: &Received) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = received
        .headers
        .iter()
        .filter(|(name, _)| name != "host")
        .cloned()
        .collect();
    headers.sort();
    headers
}

/// The M1 contract: one engine. The same file imported and sent from the app, or run from
/// the file, sends the same request and reaches the same verdicts.
#[tokio::test]
async fn a_file_run_matches_send_of_the_imported_collection() {
    let server = Server::start(|_, _| {
        response(
            "201 Created",
            &[("Content-Type", "application/json")],
            br#"{"id": 7, "token": "t-1"}"#,
        )
    })
    .await;
    let document = document(&server.url);

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("api.imposter.json");
    std::fs::write(&file, native::encode(&document)).unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let preview = read_preview(&store.handle(), file, Vec::new())
        .await
        .unwrap();
    let collection = import(&store.handle(), preview, false).await.unwrap();
    let saved = store
        .call(move |db| {
            let dev = environments::environments(db)?
                .into_iter()
                .find(|e| e.name == "Dev")
                .map(|e| e.id);
            environments::set_active_environment(db, dev)?;
            collections::requests(db, collection)
        })
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.template.name == "Create user")
        .unwrap();
    let sent = execute(&store.handle(), saved, CancellationToken::new())
        .await
        .unwrap();

    let selected = select(
        &document,
        &path("Users/Create user"),
        Some("Dev"),
        &Variables::new(),
    )
    .unwrap();
    let ran = run(&selected.inputs, selected.request, CancellationToken::new())
        .await
        .unwrap();

    let received = server.requests();
    assert_eq!(received.len(), 2);
    assert_eq!(headers(&received[0]), headers(&received[1]));
    assert_eq!(received[0].request_line, received[1].request_line);
    assert_eq!(received[1].header("x-scope"), Some("environment"));
    assert_eq!(received[1].header("x-collection"), Some("s3cret"));
    assert_eq!(received[1].header("x-folder"), Some("users"));

    let from_file = &ran.record.response;
    let verdicts = |checks: &[imposter_core::check::CheckResult]| -> Vec<Verdict> {
        checks.iter().map(|c| c.verdict.clone()).collect()
    };
    assert_eq!(verdicts(&sent.response.checks), verdicts(&from_file.checks));
    assert!(matches!(
        verdicts(&from_file.checks).as_slice(),
        [Verdict::Passed, Verdict::Failed, Verdict::Unchecked(_)]
    ));
    assert_eq!(sent.response.script, from_file.script);
    assert_eq!(
        from_file.captures[0].taken,
        Taken::Value("t-1".into()),
        "captures take the same value"
    );
    assert_eq!(sent.response.captures[0].target, "Dev");
    assert_eq!(from_file.captures[0].target, "this run");

    let stored: Vec<_> = sent.writes.iter().map(|w| w.scope.is_some()).collect();
    assert_eq!(
        stored,
        [true, true],
        "Send writes the capture and the script's set"
    );
    assert!(
        ran.writes.iter().all(|w| w.scope.is_none()),
        "a file run keeps every value in the run"
    );
    assert_eq!(ran.record.environment, "Dev");
    assert_eq!(ran.record.environment_id, None);
}

#[tokio::test]
async fn inputs_override_the_file_and_an_environment_is_optional() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let document = document(&server.url);
    for (environment, values, expected) in [
        (None, Variables::new(), "collection"),
        (Some("Prod"), Variables::new(), "production"),
        (Some("Prod"), vars(&[("scope", "input")]), "input"),
    ] {
        let selected = select(&document, &path("Ping"), environment, &values).unwrap();
        let ran = run(&selected.inputs, selected.request, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(ran.record.response.status, 200);
        let last = server.requests().pop().unwrap();
        assert_eq!(last.header("x-scope"), Some(expected), "{environment:?}");
        assert_eq!(
            ran.record.environment,
            environment.unwrap_or("Global"),
            "no environment reads as Global, as in the app"
        );
    }
}
