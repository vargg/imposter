//! The Scripts tab and script results: editing, compile errors, examples, the reference,
//! and what a send shows and keeps.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::store::{collections, executions};
use imposter_testkit::{Server, response};
use imposter_ui::app::{EditorTab, ResponseTab};
use imposter_ui::scripts_tab::{AFTER_EDITOR, BEFORE_EDITOR};
use support::{click, collection, request, settle, start_with};

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"id": 7}"#,
        )
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn open_scripts(app: &mut support::App) {
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Scripts;
    app.harness.run_steps(2);
}

fn set_scripts(app: &mut support::App, before: &str, after: &str) {
    let scripts = &mut app
        .harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .scripts;
    before.clone_into(&mut scripts.before);
    after.clone_into(&mut scripts.after);
    app.harness.run_steps(2);
}

fn send(app: &mut support::App) {
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
}

#[test]
fn scripts_are_typed_saved_and_marked_on_their_tab() {
    let mut f = fixture();
    open_scripts(&mut f.app);
    let tab = f.app.harness.get_by_role_and_label(Role::Button, "Scripts");
    assert_eq!(tab.accesskit_node().description(), None, "nothing set");

    support::type_into(
        &mut f.app.harness,
        BEFORE_EDITOR,
        "request.set_header(\"X\", \"1\");",
    );
    f.app.harness.get_by_label("Compiles");
    let tab = f.app.harness.get_by_role_and_label(Role::Button, "Scripts");
    assert_eq!(tab.accesskit_node().description().as_deref(), Some("Set"));

    click(&mut f.app.harness, "Save request");
    settle(&mut f.app.harness);
    let stored = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            let id = collections::collections(db)?[0].id;
            collections::requests(db, id)
        })
        .unwrap();
    assert_eq!(
        stored[0].template.scripts.before,
        "request.set_header(\"X\", \"1\");"
    );
}

#[test]
fn a_compile_error_is_shown_with_its_line() {
    let mut f = fixture();
    open_scripts(&mut f.app);
    set_scripts(&mut f.app, "", "let a = 1;\nlet b = ;");
    f.app.harness.get_by_label_contains("Line 2, column 9:");
}

#[test]
fn examples_go_in_at_the_cursor_and_the_reference_lists_the_api() {
    let mut f = fixture();
    open_scripts(&mut f.app);
    click(
        &mut f.app.harness,
        &format!("Insert an example into the {AFTER_EDITOR}"),
    );
    click(&mut f.app.harness, "Test the status");
    let after = f
        .app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .scripts
        .after
        .clone();
    assert_eq!(after, "test(\"status is 200\", response.status == 200);\n");

    click(&mut f.app.harness, "Script reference");
    f.app.harness.run_steps(3);
    assert!(matches!(
        f.app.harness.state().state.dialog,
        Some(imposter_ui::dialogs::Dialog::ScriptReference)
    ));
    for entry in imposter_app::script::API {
        assert!(
            f.app.harness.query_by_label(entry.name).is_some(),
            "the reference lists {}",
            entry.name
        );
    }
}

#[test]
fn a_send_shows_script_tests_and_console_and_history_keeps_only_the_tests() {
    let mut f = fixture();
    set_scripts(
        &mut f.app,
        r#"request.set_header("X-Signed", "yes");"#,
        r#"print("seen " + response.status);
test("status is 200", response.status == 200);
test("id", || { expect(response.path("$.id"), 8) });"#,
    );
    send(&mut f.app);
    assert_eq!(
        f.server.requests()[0].header("x-signed"),
        Some("yes"),
        "the before script's header was sent"
    );
    f.app.harness.state_mut().state.response.tab = ResponseTab::Tests;
    f.app.harness.run_steps(3);
    f.app.harness.get_by_label("status is 200");
    f.app.harness.get_by_label("expected 8, got 7");
    f.app
        .harness
        .get_by_label("Console output is not kept in History");
    f.app.harness.get_by_label("seen 200");
    let tests = f
        .app
        .harness
        .get_by_role_and_label(Role::Button, "Check results");
    assert_eq!(
        tests.accesskit_node().description().as_deref(),
        Some("1 failed")
    );

    let record = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            let summary = executions::page(db, None, 0, 1)?.remove(0);
            executions::execution(db, summary.id)
        })
        .unwrap()
        .unwrap();
    assert_eq!(record.response.script.unwrap().tests.len(), 2);
    assert_eq!(record.response.console, Vec::<String>::new());
}

#[test]
fn a_failing_before_script_is_not_sent_and_says_where() {
    let mut f = fixture();
    set_scripts(&mut f.app, "let a = 1;\nthrow \"no key\";", "");
    send(&mut f.app);
    assert_eq!(f.server.requests().len(), 0);
    f.app
        .harness
        .get_by_label_contains("Before-sending script, line 2: no key");
    let count = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap();
    assert_eq!(count, 0);
}
