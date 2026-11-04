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
use support::{click, click_button, collection, request, settle, start_with};

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
        // A name can be listed under two headings (request and mock scripts).
        assert!(
            f.app
                .harness
                .query_all_by_label(entry.name)
                .next()
                .is_some(),
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

/// API > Users > "Items", opened.
fn in_folder() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"{}")));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        store
            .call_blocking(move |db| {
                let users = collections::create_folder(db, api.id, None, "Users", "", 0)?;
                let template = imposter_app::core::model::RequestTemplate {
                    name: "Items".into(),
                    url,
                    ..Default::default()
                };
                collections::create_request(db, api.id, Some(users.id), &template).map(drop)
            })
            .unwrap();
    });
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn stored_scripts(
    app: &support::App,
) -> (
    imposter_app::core::model::Scripts,
    imposter_app::core::model::Scripts,
) {
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            let collection = collections::collections(db)?.remove(0);
            let folder = collections::folders(db, collection.id)?.remove(0);
            Ok((collection.scripts, folder.scripts))
        })
        .unwrap()
}

#[test]
fn a_folders_scripts_are_edited_in_a_dialog_and_run_around_its_requests() {
    let mut f = in_folder();
    f.app.harness.get_by_label("Folder Users").click_secondary();
    f.app.harness.run_steps(2);
    click(&mut f.app.harness, "Scripts…");
    f.app.harness.run_steps(3);
    support::type_into(
        &mut f.app.harness,
        "Folder \"Users\" before-sending script",
        "request.set_header(\"X-Folder\", \"users\");",
    );
    click_button(&mut f.app.harness, "Save");
    settle(&mut f.app.harness);
    let (_, folder) = stored_scripts(&f.app);
    assert_eq!(
        folder.before,
        "request.set_header(\"X-Folder\", \"users\");"
    );
    let row = f.app.harness.get_by_label("Folder Users");
    assert_eq!(
        row.accesskit_node().description().as_deref(),
        Some("Has scripts")
    );

    open_scripts(&mut f.app);
    f.app
        .harness
        .get_by_label("Open the scripts of Folder \"Users\"");
    send(&mut f.app);
    assert_eq!(f.server.requests()[0].header("x-folder"), Some("users"));
}

#[test]
fn the_collection_menu_edits_its_scripts_and_cancel_keeps_nothing() {
    let mut f = in_folder();
    click(&mut f.app.harness, "Collection actions");
    click(&mut f.app.harness, "Collection Scripts…");
    f.app.harness.run_steps(3);
    support::type_into(
        &mut f.app.harness,
        "Collection \"API\" after-response script",
        "test(\"ok\", true);",
    );
    click_button(&mut f.app.harness, "Cancel");
    settle(&mut f.app.harness);
    assert!(stored_scripts(&f.app).0.is_empty(), "Cancel keeps nothing");

    click(&mut f.app.harness, "Collection actions");
    click(&mut f.app.harness, "Collection Scripts…");
    f.app.harness.run_steps(3);
    support::type_into(
        &mut f.app.harness,
        "Collection \"API\" after-response script",
        "test(\"from the collection\", true);",
    );
    click_button(&mut f.app.harness, "Save");
    settle(&mut f.app.harness);
    assert_eq!(
        stored_scripts(&f.app).0.after,
        "test(\"from the collection\", true);"
    );
    open_scripts(&mut f.app);
    click(&mut f.app.harness, "Open the scripts of Collection \"API\"");
    f.app.harness.run_steps(3);
    assert!(matches!(
        f.app.harness.state().state.dialog,
        Some(imposter_ui::dialogs::Dialog::Scripts { .. })
    ));
}
