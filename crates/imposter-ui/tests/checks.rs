//! Checks and captures: edited in the request's Tests tab, evaluated after each response,
//! shown with the response and in History; checks can be added from a JSON value, captures
//! write variables unless that would lose unsaved edits.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::capture::Capture;
use imposter_app::core::check::{Check, Op};
use imposter_app::core::model::Request;
use imposter_app::core::select::Selector;
use imposter_app::core::variables::Scope;
use imposter_app::store::{collections, scopes};
use imposter_testkit::{Server, response};
use imposter_ui::app::{EditorTab, HistoryTab};
use support::{click, collection, open_tab, request, settle, start_with, text_input, type_into};

struct Fixture {
    app: support::App,
    _server: Server,
    _runtime: tokio::runtime::Runtime,
}

/// The open request "Users", answered with 201 and a small JSON body.
fn opened(checks: Vec<Check>) -> Fixture {
    opened_with(checks, Vec::new())
}

fn opened_with(checks: Vec<Check>, captures: Vec<Capture>) -> Fixture {
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
        saved.template.captures = captures;
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
    open_tab(&mut app.harness, "Tests");
    assert!(
        app.harness.query_by_label("Add capture").is_none(),
        "captures have their own tab"
    );
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
    // The request's tab counts its checks; the response's says how many failed, since a
    // red total read as that many failures.
    let description = |app: &support::App, label: &str| {
        app.harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .accesskit_node()
            .description()
    };
    assert_eq!(description(app, "Tests").as_deref(), Some("2 in use"));
    assert_eq!(
        description(app, "Check results").as_deref(),
        Some("1 failed")
    );
    // The marks in the status line open the results.
    click(&mut app.harness, "Checks: 1 passed, 1 failed");
    app.harness.get_by_label("got 201");
    let failed = app.harness.get_by_label("status = 200").rect();
    let passed = app.harness.get_by_label("$.id > 5").rect();
    assert!(failed.top() < passed.top(), "failures come first");
    app.harness.get_by_label("Failed");
    app.harness.get_by_label("Passed");

    // Undecidable checks say why, and the results follow the next response.
    replace(app, "Check 2 JSON path", "id");
    send(app);
    app.harness.get_by_label("Checks: 1 failed, 1 not checked");
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
    app.harness.get_by_label("Checks: 1 passed");
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
    app.harness.get_by_label("Checks: 1 passed");
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.state_mut().state.history.tab = HistoryTab::Response;
    app.harness.run_steps(2);
    app.harness.get_by_label("Checks: 1 passed");
    click(&mut app.harness, "Check results");
    app.harness.get_by_label("status = 2xx");
    app.harness.get_by_label("got 201");
    assert!(
        app.harness.query_by_label("Add check").is_none(),
        "History offers no check editing"
    );
}

fn globals(app: &support::App) -> imposter_app::core::model::Variables {
    app.harness
        .state()
        .store()
        .call_blocking(|db| scopes::read(db, Scope::Global))
        .unwrap()
        .0
}

#[test]
fn captures_set_variables_that_the_next_send_uses() {
    let mut fixture = opened(Vec::new());
    let app = &mut fixture.app;
    open_tab(&mut app.harness, "Captures");
    click(&mut app.harness, "Add capture");
    type_into(&mut app.harness, "Capture 1 variable", "user_id");
    replace(app, "Capture 1 JSON path", "$.id");
    click(&mut app.harness, "Add capture");
    type_into(&mut app.harness, "Capture 2 variable", "missing");
    replace(app, "Capture 2 JSON path", "$.nope");
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert_eq!(
        stored(app).template.captures,
        [
            Capture::new("user_id", Selector::Json("$.id".into())),
            Capture::new("missing", Selector::Json("$.nope".into())),
        ]
    );
    // Each tab counts its own rows.
    let description = |app: &support::App, label: &str| {
        app.harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .accesskit_node()
            .description()
    };
    assert_eq!(description(app, "Captures").as_deref(), Some("2 in use"));
    // Tests may wait under "More" beside the selected Captures tab; its badge is none.
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .badge(EditorTab::Tests),
        None
    );

    send(app);
    assert_eq!(globals(app).get("user_id").map(String::as_str), Some("7"));
    assert!(
        !globals(app).contains_key("missing"),
        "nothing there, nothing set"
    );
    app.harness
        .get_by_label_contains("201 Created · ")
        .value()
        .filter(|text| text.ends_with(" · set user_id"))
        .expect("the result and the capture share the status bar");
    click(&mut app.harness, "Check results");
    app.harness.get_by_label("in Global: 7");
    app.harness
        .get_by_label("nothing there; the variable keeps its value");
    assert!(
        app.harness
            .state()
            .state
            .snapshot
            .as_ref()
            .unwrap()
            .values()["user_id"]
            == "7",
        "the editor's variables follow"
    );
}

#[test]
fn captures_wait_for_unsaved_variables_and_refresh_a_clean_screen() {
    let mut fixture = opened_with(
        Vec::new(),
        vec![Capture::new("user_id", Selector::Json("$.id".into()))],
    );
    let app = &mut fixture.app;
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            scopes::write(
                db,
                Scope::Global,
                &[("user_id".to_owned(), "1".to_owned())]
                    .into_iter()
                    .collect(),
                &imposter_app::core::model::SavedValues::default(),
            )
        })
        .unwrap();

    // Unsaved edits to the globals: the captured value is not written.
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Variable 1 value", "0");
    click(&mut app.harness, "Requests");
    send(app);
    assert_eq!(globals(app)["user_id"], "1");
    app.harness
        .get_by_label("Not set: user_id — the Variables screen has unsaved changes to that scope");

    // Once saved, the next capture is written and the clean screen shows it.
    click(&mut app.harness, "Variables");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    click(&mut app.harness, "Requests");
    send(app);
    assert_eq!(globals(app)["user_id"], "7");
    let screen = &app.harness.state().state.variables;
    assert!(!screen.dirty());
    click(&mut app.harness, "Variables");
    assert_eq!(
        text_input(&app.harness, "Variable 1 value")
            .value()
            .as_deref(),
        Some("7")
    );
}

#[test]
fn a_json_value_becomes_a_variable_named_after_it() {
    let mut fixture = opened(Vec::new());
    let app = &mut fixture.app;
    send(app);
    app.harness
        .get_by_label_contains("\"id\": 7")
        .click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Set variable");
    settle(&mut app.harness);
    let pane = app.harness.state().state.editor.as_ref().unwrap();
    assert_eq!(pane.tab, EditorTab::Captures);
    assert_eq!(
        pane.editor.captures,
        [Capture::new("id", Selector::Json("$.id".into()))]
    );
    send(app);
    assert_eq!(
        globals(app)["id"],
        "7",
        "the unsaved capture runs with the next send"
    );
}

fn tests_tab(app: &mut support::App) {
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Tests;
    app.harness.run_steps(2);
}

fn captures_tab(app: &mut support::App) {
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Captures;
    app.harness.run_steps(2);
}

fn rect(app: &support::App, label: &str) -> egui::Rect {
    app.harness.get_by_label(label).rect()
}

#[test]
fn checks_keep_their_columns_whatever_they_read() {
    let mut fixture = opened(vec![
        Check::new(Selector::Status, Op::Equals, "200"),
        Check::new(Selector::Json("$.name".into()), Op::Equals, "Ann"),
        Check::new(Selector::Duration, Op::Less, "500"),
    ]);
    let app = &mut fixture.app;
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    tests_tab(app);
    // One line each: the operator and expected value sit in one column whether or not the
    // source has a path.
    let ops: Vec<egui::Rect> = (1..=3)
        .map(|n| rect(app, &format!("Check {n} operator")))
        .collect();
    let expected: Vec<egui::Rect> = (1..=3)
        .map(|n| rect(app, &format!("Check {n} expected value")))
        .collect();
    for rects in [&ops, &expected] {
        assert!(
            rects
                .iter()
                .all(|r| (r.left() - rects[0].left()).abs() < 0.5),
            "{rects:?}"
        );
    }
    let source = rect(app, "Check 2 source");
    assert!(
        (ops[1].center().y - source.center().y).abs() < 1.0,
        "one line"
    );
}

#[test]
fn checks_stack_in_a_narrow_pane() {
    let mut fixture = opened(vec![
        Check::new(Selector::Status, Op::Equals, "200"),
        Check::new(Selector::Json("$.name".into()), Op::Equals, "Ann"),
    ]);
    let app = &mut fixture.app;
    app.harness.set_size(imposter_ui::layout::MIN_WINDOW.into());
    tests_tab(app);
    // What to read, its path, the operator and the expected value, each on its own line,
    // all ending at the remove button's right edge.
    let source = rect(app, "Check 2 source");
    let path = rect(app, "Check 2 JSON path");
    let op = rect(app, "Check 2 operator");
    let expected = rect(app, "Check 2 expected value");
    let remove = rect(app, "Remove check 2");
    assert!(
        source.bottom() <= path.top() && path.bottom() <= op.top(),
        "{source:?} {path:?} {op:?}"
    );
    assert!(op.bottom() <= expected.top(), "{op:?} {expected:?}");
    for (what, r) in [("path", path), ("expected", expected)] {
        assert!(
            (r.right() - remove.right()).abs() < 1.5,
            "{what} {r:?} ends with {remove:?}"
        );
    }
    // A status check has no path line.
    let first_op = rect(app, "Check 1 operator");
    let first_source = rect(app, "Check 1 source");
    assert!(
        first_op.top() - first_source.bottom() < 12.0,
        "{first_source:?} {first_op:?}"
    );
}

/// The capture rows' targets, top to bottom.
fn targets(app: &support::App) -> Vec<String> {
    let mut found: Vec<(f32, String)> = app
        .harness
        .query_all_by_label_contains("→ ")
        .map(|node| {
            let text = node.accesskit_node().value().unwrap_or_default();
            (node.rect().top(), text)
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, text)| text).collect()
}

#[test]
fn a_capture_says_which_scope_it_writes_to() {
    let mut fixture = opened_with(
        Vec::new(),
        vec![
            Capture::new("defined", Selector::Json("$.id".into())),
            Capture::new("fresh", Selector::Json("$.name".into())),
        ],
    );
    let app = &mut fixture.app;
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            imposter_app::store::environments::set_globals(
                db,
                &[("defined".to_owned(), "1".to_owned())]
                    .into_iter()
                    .collect(),
            )
        })
        .unwrap();
    let dev = app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            imposter_app::store::environments::create_environment(
                db,
                "Dev",
                &imposter_app::core::model::Variables::default(),
            )
        })
        .unwrap();
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Global (no environment)");
    settle(&mut app.harness);
    captures_tab(app);
    assert_eq!(targets(app), ["→ Global", "→ Global"]);

    app.harness
        .state_mut()
        .state
        .actions
        .push(imposter_ui::app::Action::SetEnvironment(Some(dev.id)));
    app.harness.run_steps(1);
    settle(&mut app.harness);
    // Defined in Global: replaced there. Defined nowhere: the active environment.
    assert_eq!(targets(app), ["→ Global", "→ Dev"]);
    app.harness.get_by_label("Add capture");
    assert!(app.harness.query_by_label("Add variable").is_none());
}
