//! Every action that removes saved data asks first and says what goes: a stray click, or
//! Escape on the question, loses nothing.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{RequestTemplate, Variables};
use imposter_app::store::{collections, environments, executions};
use imposter_testkit::{Server, response};
use support::{click, click_button, collection, request, settle, start_with, type_into};

struct Fixture {
    app: support::App,
    _server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// API holds Health and a folder Auth with Login; Dev has two variables; history one run.
fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let url = format!("{}/health", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Health", &url);
        store
            .call_blocking(move |db| {
                let auth = collections::create_folder(db, api.id, None, "Auth", "", 1)?;
                let login = RequestTemplate {
                    name: "Login".into(),
                    url: "http://x/login".into(),
                    ..Default::default()
                };
                collections::create_request(db, api.id, Some(auth.id), &login)?;
                environments::create_environment(db, "Dev", &vars(&[("a", "1"), ("b", "2")]))
                    .map(drop)
            })
            .unwrap();
    });
    click(&mut app.harness, "GET Health");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    Fixture {
        app,
        _server: server,
        _runtime: runtime,
    }
}

/// The question is open and says `says`; Escape answers it with no.
fn asked_and_escaped(app: &mut support::App, says: &str) {
    app.harness.get_by_label_contains(says);
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none(), "{says}");
}

fn requests(app: &support::App) -> usize {
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            let id = collections::collections(db)?[0].id;
            Ok(collections::tree(db, id)?.map_or(0, |t| t.requests.len()))
        })
        .unwrap()
}

#[test]
fn deleting_from_the_tree_asks_and_names_what_goes() {
    let mut fixture = fixture();
    let app = &mut fixture.app;

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Delete Collection…");
    asked_and_escaped(
        app,
        "Delete \"API\" with 2 requests in 1 folder, including unsaved edits?",
    );

    app.harness.get_by_label("Folder Auth").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Delete…");
    asked_and_escaped(app, "Delete the folder \"Auth\" with 1 request,");

    app.harness.get_by_label("GET Health").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Delete…");
    asked_and_escaped(
        app,
        "Delete the request \"Health\", including unsaved edits?",
    );

    assert_eq!(requests(app), 2, "nothing was deleted");
}

#[test]
fn variables_ask_before_they_are_cleared_reverted_or_deleted() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "Variable scope");
    click(&mut app.harness, "Environment · Dev");
    settle(&mut app.harness);

    click(&mut app.harness, "Variable actions");
    click(&mut app.harness, "Delete environment…");
    asked_and_escaped(app, "Delete the environment \"Dev\" and its 2 variables?");

    click(&mut app.harness, "Variable actions");
    click(&mut app.harness, "Clear all variables…");
    asked_and_escaped(app, "Remove all 2 variables from the Dev environment?");
    assert_eq!(app.harness.state().state.variables.rows.len(), 2);

    type_into(&mut app.harness, "Variable 1 value", "-edited");
    click(&mut app.harness, "Revert unsaved changes");
    asked_and_escaped(app, "Discard the unsaved changes to the Dev environment");
    assert!(
        app.harness.state().state.variables.dirty(),
        "the edit is still there"
    );
    click(&mut app.harness, "Revert unsaved changes");
    click_button(&mut app.harness, "Discard changes");
    assert!(!app.harness.state().state.variables.dirty());

    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(|db| environments::environments(db))
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].variables, vars(&[("a", "1"), ("b", "2")]));
}

#[test]
fn clearing_history_asks_and_counts_the_runs() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    click_button(&mut app.harness, "History");
    settle(&mut app.harness);
    click(&mut app.harness, "History actions");
    click(&mut app.harness, "Clear all history…");
    asked_and_escaped(
        app,
        "Delete 1 execution from history? This cannot be undone.",
    );
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| executions::count(db))
            .unwrap(),
        1
    );
}
