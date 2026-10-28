//! curl in and out through the real UI: paste a command as a draft, copy a request as curl.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::Variables;
use imposter_app::store::{collections, environments};
use imposter_testkit::{Server, response};
use support::{click, click_button, collection, request, settle, start_with, text_input};

/// Run frames until something is copied, returning it; `None` after the work is done without.
fn copied(app: &mut support::App) -> Option<String> {
    for _ in 0..50 {
        assert!(
            app.harness
                .state()
                .bridge()
                .wait_idle(Duration::from_secs(10))
        );
        app.harness.step();
        let text = app
            .harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        if text.is_some() {
            return text;
        }
    }
    None
}

fn globals(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open_curl_dialog(app: &mut support::App) {
    click(&mut app.harness, "New");
    click(&mut app.harness, "Import from curl…");
    settle(&mut app.harness);
}

#[test]
fn a_pasted_command_opens_as_an_unsaved_draft() {
    let mut app = start_with(|store| {
        collection(store, "API");
    });
    open_curl_dialog(&mut app);
    text_input(&app.harness, "curl command:").type_text(
        "curl 'https://api.example.com/items' -H 'X-Trace: on' --data-raw '{\"a\":1}' --http2",
    );
    app.harness.run_steps(2);
    app.harness
        .get_by_label("--http2 is not supported and was ignored");

    click_button(&mut app.harness, "Open as draft");
    settle(&mut app.harness);

    let pane = app.harness.state().state.editor.as_ref().unwrap();
    assert!(pane.editor.is_draft());
    assert_eq!(pane.editor.url, "https://api.example.com/items");
    assert_eq!(pane.editor.method, "POST");
    assert_eq!(pane.editor.headers[0].name, "X-Trace");
    let saved = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::requests(db, imposter_app::core::ids::CollectionId(1)))
        .unwrap();
    assert_eq!(saved, [], "nothing is saved until Save");
}

#[test]
fn a_command_that_does_not_parse_says_why() {
    let mut app = start_with(|store| {
        collection(store, "API");
    });
    open_curl_dialog(&mut app);
    text_input(&app.harness, "curl command:").type_text("wget http://h");
    app.harness.run_steps(2);

    app.harness
        .get_by_label("Paste a curl command: it starts with \"curl\"");
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Open as draft")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn copy_as_curl_resolves_variables_and_copies() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &globals(&[("host", "example.com")])))
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://{{host}}/x");
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);

    app.harness.get_by_label("Copy as curl").click();
    let command = copied(&mut app).expect("a command is copied");

    assert_eq!(command, "curl 'http://example.com/x' \\\n  -L");
    app.harness.get_by_label("Copied as curl");
}

#[test]
fn an_unresolved_variable_is_reported_and_nothing_is_copied() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://{{missing}}/x");
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);

    app.harness.get_by_label("Copy as curl").click();
    assert_eq!(copied(&mut app), None);
    settle(&mut app.harness);
    app.harness.get_by_label("Copy as curl failed");
}

#[test]
fn history_copies_the_request_as_it_was_sent() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let url = format!("{}/sent", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", &url);
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);

    // History copies in the frame of the click, so no frames may run before looking.
    app.harness
        .get_by_role_and_label(Role::Button, "Copy as curl")
        .click();
    let command = copied(&mut app).expect("a command is copied");
    assert!(
        command.starts_with(&format!("curl '{}/sent'", server.url)),
        "{command}"
    );
}
