//! The Run screen: a folder run from its menu with values carried forward, stopping at a
//! failure or keeping going, the environment it uses and the production question, Stop,
//! and History holding what was sent while variables stay as they were.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::{Duration, Instant};

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::execution::Via;
use imposter_app::store::{collections, environments, executions};
use imposter_testkit::{LoadReply, LoadServer, Received, Server, response};
use imposter_ui::app::Screen;
use serde_json::{Value, json};
use support::{click, click_button, settle, start_with};

struct Fixture {
    app: support::App,
    server: Server,
    hang: LoadServer,
    _runtime: tokio::runtime::Runtime,
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    let json = [("Content-Type", "application/json")];
    match (received.method(), received.target()) {
        ("POST", "/items") => response("201 Created", &json, br#"{"id": 7}"#),
        ("GET", "/items/7") => response("200 OK", &json, br#"{"id": 7}"#),
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

/// "API": Flow (Create captures the id, Read uses it), Broken (Fails, then After), Env (one
/// request naming `{{who}}`) and Slow (one that never answers, then After). Dev is active;
/// Other and Prod (red) are not.
fn fixture() -> Fixture {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime.block_on(Server::start(reply));
    let hang = runtime.block_on(LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_secs(60),
        ..LoadReply::ok(b"late")
    }));
    let status = |code: &str| json!([{"selector": "status", "op": "equals", "expected": code}]);
    let document = json!({
        "format": "imposter", "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": server.url, "slow": hang.url},
            "folders": [
                {"name": "Flow", "position": 0, "requests": [
                    request("Create", "POST", "{{base}}/items", 0, json!({
                        "checks": status("201"),
                        "captures": [{"variable": "id", "selector": {"json": "$.id"}}]
                    })),
                    request("Read", "GET", "{{base}}/items/{{id}}", 1, json!({"checks": status("200")}))
                ]},
                {"name": "Broken", "position": 1, "requests": [
                    request("Fails", "GET", "{{base}}/fail", 0, json!({"checks": status("200")})),
                    request("After", "GET", "{{base}}/after", 1, json!({"checks": status("200")}))
                ]},
                {"name": "Env", "position": 2, "requests": [
                    request("Who", "GET", "{{base}}/env/{{who}}", 0, json!({"checks": status("200")}))
                ]},
                {"name": "Slow", "position": 3, "requests": [
                    request("Hang", "GET", "{{slow}}/hang", 0, json!({})),
                    request("After", "GET", "{{base}}/after", 1, json!({}))
                ]}
            ]
        },
        "environments": [
            {"name": "Dev", "variables": {"who": "dev"}},
            {"name": "Other", "variables": {"who": "other"}},
            {"name": "Prod", "variables": {"who": "prod"}, "color": "red"}
        ]
    });
    let app = start_with(move |store| {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("api.imposter.json");
        std::fs::write(&file, serde_json::to_vec(&document).unwrap()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let preview = imposter_app::transfer::read_preview(&store.handle(), file, Vec::new())
                .await
                .unwrap();
            imposter_app::transfer::import(&store.handle(), preview, false)
                .await
                .unwrap();
        });
        store
            .call_blocking(|db| {
                let dev = environments::environments(db)?
                    .into_iter()
                    .find(|e| e.name == "Dev")
                    .map(|e| e.id);
                environments::set_active_environment(db, dev)
            })
            .unwrap();
    });
    Fixture {
        app,
        server,
        hang,
        _runtime: runtime,
    }
}

fn targets(server: &Server) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|r| format!("{} {}", r.method(), r.target()))
        .collect()
}

/// Open the folder's context menu in the sidebar and choose "Run…".
fn run_folder(app: &mut support::App, folder: &str) {
    app.harness
        .get_by_label(&format!("Folder {folder}"))
        .click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Run…");
    settle(&mut app.harness);
}

fn start(app: &mut support::App) {
    click(&mut app.harness, "Start the run");
    settle(&mut app.harness);
}

#[test]
fn a_folder_runs_from_its_menu_carrying_values_into_history_but_not_variables() {
    let mut f = fixture();
    run_folder(&mut f.app, "Flow");
    assert_eq!(f.app.harness.state().state.screen, Screen::Run);
    let target = f
        .app
        .harness
        .get_by_label("What to run")
        .accesskit_node()
        .value();
    assert_eq!(target.as_deref(), Some("API / Flow"));
    start(&mut f.app);

    assert_eq!(targets(&f.server), ["POST /items", "GET /items/7"]);
    f.app
        .harness
        .get_by_label("Run step Passed API/Flow/Create");
    f.app.harness.get_by_label("Run step Passed API/Flow/Read");
    f.app
        .harness
        .get_by_label("Passed · 2 requests ran · 2 of 2 checks passed");

    click(&mut f.app.harness, "Run step Passed API/Flow/Read");
    let shown = f.app.harness.state().state.run.pane.response.clone();
    assert_eq!(shown.unwrap().status, 200);
    f.app.harness.get_by_label_contains("\"id\"");

    let store = f.app.harness.state().store();
    let history = store
        .call_blocking(|db| executions::page(db, None, 0, 10))
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|h| h.via == Via::App));
    let collection = store
        .call_blocking(|db| Ok(collections::collections(db)?[0].clone()))
        .unwrap();
    assert!(
        !collection.variables.contains_key("id"),
        "the captured id stayed in the run"
    );
    let globals = store.call_blocking(|db| environments::globals(db)).unwrap();
    assert!(!globals.contains_key("id"));
}

#[test]
fn a_failure_stops_the_run_unless_it_keeps_going() {
    let mut f = fixture();
    support::open_screen(&mut f.app.harness, "Run");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "What to run");
    click(&mut f.app.harness, "API / Broken");
    start(&mut f.app);
    assert_eq!(targets(&f.server), ["GET /fail"]);
    f.app
        .harness
        .get_by_label("Run step Failed API/Broken/Fails");
    f.app
        .harness
        .get_by_label("Run step Skipped API/Broken/After");
    f.app
        .harness
        .get_by_label("Failed · 1 request ran · 0 of 1 checks passed · 1 skipped");
    f.app
        .harness
        .get_by_label("Run finished · Failed · 1 request ran · 0 of 1 checks passed · 1 skipped");
    click(&mut f.app.harness, "Run step Skipped API/Broken/After");
    f.app
        .harness
        .get_by_label("Skipped: an earlier request did not pass.");

    click(&mut f.app.harness, "Keep going after a failure");
    start(&mut f.app);
    assert_eq!(targets(&f.server), ["GET /fail", "GET /fail", "GET /after"]);
    f.app
        .harness
        .get_by_label("Run step Passed API/Broken/After");
    click(&mut f.app.harness, "Earlier runs");
    assert_eq!(
        f.app
            .harness
            .get_all_by_label_contains("API / Broken · ")
            .count(),
        3
    );
}

#[test]
fn the_run_uses_the_environment_chosen_and_asks_before_production() {
    let mut f = fixture();
    run_folder(&mut f.app, "Env");
    start(&mut f.app);
    assert_eq!(
        targets(&f.server),
        ["GET /env/dev"],
        "the active environment"
    );

    click(&mut f.app.harness, "Environment for the run");
    click(&mut f.app.harness, "Other");
    start(&mut f.app);
    assert_eq!(targets(&f.server)[1], "GET /env/other");

    click(&mut f.app.harness, "Environment for the run");
    click(&mut f.app.harness, "Prod");
    click(&mut f.app.harness, "Start the run");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("Run against Prod?");
    assert_eq!(
        targets(&f.server).len(),
        2,
        "nothing sent before the answer"
    );
    click_button(&mut f.app.harness, "Start run");
    settle(&mut f.app.harness);
    assert_eq!(targets(&f.server)[2], "GET /env/prod");
}

#[test]
fn stop_cancels_the_request_in_flight_and_skips_the_rest() {
    let mut f = fixture();
    run_folder(&mut f.app, "Slow");
    click(&mut f.app.harness, "Start the run");
    let deadline = Instant::now() + Duration::from_secs(10);
    while f.hang.requests().is_empty() {
        assert!(Instant::now() < deadline, "the run never sent");
        f.app.harness.run_steps(1);
        std::thread::yield_now();
    }
    f.app.harness.run_steps(2);
    f.app
        .harness
        .get_by_label_contains("Running API / Slow with Dev");
    click(&mut f.app.harness, "Requests");
    click(&mut f.app.harness, "Run running");
    assert_eq!(f.app.harness.state().state.screen, Screen::Run);
    click(&mut f.app.harness, "Tools");
    let description = f
        .app
        .harness
        .get_all_by_label("Run")
        .find_map(|node| node.accesskit_node().description());
    assert_eq!(description.as_deref(), Some("Running"));
    f.app.harness.key_press(egui::Key::Escape);
    f.app.harness.run_steps(2);

    click(&mut f.app.harness, "Stop the run");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label("Run step Cancelled API/Slow/Hang");
    f.app
        .harness
        .get_by_label("Run step Skipped API/Slow/After");
    assert_eq!(targets(&f.server), Vec::<String>::new());
    assert!(f.app.harness.query_by_label("Run running").is_none());
}
