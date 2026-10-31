//! Checks: edited in the request's Tests tab, evaluated after each response, shown with the
//! response and in History, and added from a JSON value.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::check::{Check, Op};
use imposter_app::core::model::Request;
use imposter_app::core::select::Selector;
use imposter_app::store::collections;
use imposter_testkit::{Server, response};
use imposter_ui::app::{EditorTab, HistoryTab};
use support::{click, collection, request, settle, start_with, text_input};

struct Fixture {
    app: support::App,
    _server: Server,
    _runtime: tokio::runtime::Runtime,
}

/// The open request "Users", answered with 201 and a small JSON body.
fn opened(checks: Vec<Check>) -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "201 Created",
            &[("Content-Type", "application/json")],
            br#"{"id":7,"name":"Ann"}"#,
        )
    }));
    let url = format!("{}/users", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        let mut saved = request(store, api.id, "Users", &url);
        saved.template.checks = checks;
        let id = saved.id.unwrap();
        store
            .call_blocking(move |db| collections::update_request(db, id, &saved.template))
            .unwrap();
    });
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    Fixture {
        app,
        _server: server,
        _runtime: runtime,
    }
}

fn stored(app: &support::App) -> Request {
    let id = app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .id()
        .unwrap();
    app.harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, id))
        .unwrap()
        .unwrap()
}

fn send(app: &mut support::App) {
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
}

/// Replace the text of the field named `label`.
fn replace(app: &mut support::App, label: &str, text: &str) {
    text_input(&app.harness, label).click();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    text_input(&app.harness, label).type_text(text);
    app.harness.run_steps(2);
}

fn choose(app: &mut support::App, combo: &str, option: &str) {
    click(&mut app.harness, combo);
    app.harness.run_steps(1);
    click(&mut app.harness, option);
}

#[test]
fn checks_are_edited_saved_and_run_after_each_response() {
    let mut fixture = opened(Vec::new());
    let app = &mut fixture.app;
    click(&mut app.harness, "Tests");
    click(&mut app.harness, "Add check");
    click(&mut app.harness, "Add check");
    choose(app, "Check 2 source", "JSON path");
    replace(app, "Check 2 JSON path", "$.id");
    choose(app, "Check 2 operator", ">");
    replace(app, "Check 2 expected value", "5");
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert_eq!(
        stored(app).template.checks,
        [
            Check::new(Selector::Status, Op::Equals, "200"),
            Check::new(Selector::Json("$.id".into()), Op::Greater, "5"),
        ]
    );

    send(app);
    app.harness.get_by_label("1/2 passed");
    click(&mut app.harness, "Check results");
    app.harness.get_by_label("Failed");
    app.harness.get_by_label("status = 200");
    app.harness.get_by_label("got 201");
    app.harness.get_by_label("Passed");
    app.harness.get_by_label("$.id > 5");

    // Undecidable checks say why, and the results follow the next response.
    replace(app, "Check 2 JSON path", "id");
    send(app);
    app.harness.get_by_label("0/2 passed");
    app.harness.get_by_label("Not checked");
    app.harness.get_by_label_contains("Invalid JSONPath");
}

#[test]
fn a_json_value_becomes_a_check_on_the_open_request() {
    let mut fixture = opened(Vec::new());
    let app = &mut fixture.app;
    send(app);
    assert!(
        app.harness.query_by_label("Check results").is_none(),
        "no checks, no results tab"
    );
    app.harness
        .get_by_label_contains("\"name\": \"Ann\"")
        .click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Add check");
    settle(&mut app.harness);
    let pane = app.harness.state().state.editor.as_ref().unwrap();
    assert_eq!(pane.tab, EditorTab::Tests);
    assert_eq!(
        pane.editor.checks,
        [Check::new(
            Selector::Json("$.name".into()),
            Op::Equals,
            "Ann"
        )]
    );
    assert_eq!(
        text_input(&app.harness, "Check 1 JSON path")
            .value()
            .as_deref(),
        Some("$.name")
    );

    send(app);
    app.harness.get_by_label("1/1 passed");
}

#[test]
fn history_keeps_the_results_of_each_run() {
    let mut fixture = opened(vec![
        Check::new(Selector::Status, Op::Equals, "2xx"),
        Check {
            enabled: false,
            ..Check::new(Selector::Status, Op::Equals, "500")
        },
    ]);
    let app = &mut fixture.app;
    send(app);
    app.harness.get_by_label("1/1 passed");
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.state_mut().state.history.tab = HistoryTab::Response;
    app.harness.run_steps(2);
    app.harness.get_by_label("1/1 passed");
    click(&mut app.harness, "Check results");
    app.harness.get_by_label("status = 2xx");
    app.harness.get_by_label("got 201");
    assert!(
        app.harness.query_by_label("Add check").is_none(),
        "History offers no check editing"
    );
}
