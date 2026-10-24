//! The request workflow through the real UI and a real database.
#![cfg(test)]

mod support;

use egui_kittest::kittest::Queryable;
use poster_app::core::settings::AppSettings;
use poster_app::store::{collections, executions, kv};
use poster_testkit::{Server, response};
use poster_ui::app::CloseState;
use support::{answer_name, click, collection, request, settle, start, start_with, type_into};

#[test]
fn an_empty_database_offers_to_create_a_collection() {
    let mut app = start();

    app.harness
        .get_by_label("No collections yet. Create one with New.");
    click(&mut app.harness, "New");
    click(&mut app.harness, "New Collection");
    answer_name(&mut app.harness, "My API");

    app.harness
        .get_by_label("This collection is empty. Add a request with New.");
    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::collections(db))
        .unwrap();
    assert_eq!(
        stored.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["My API"]
    );
}

#[test]
fn new_requests_open_in_the_editor_and_autosave_edits() {
    let mut app = start_with(|store| {
        collection(store, "API");
    });

    click(&mut app.harness, "New");
    click(&mut app.harness, "New Request");
    answer_name(&mut app.harness, "Health");
    app.harness.get_by_label("API / Health");

    type_into(
        &mut app.harness,
        "Request URL",
        "https://example.com/health?verbose=1",
    );
    app.harness.get_by_label("Unsaved request changes");
    for _ in 0..4 {
        app.harness.step();
    }
    settle(&mut app.harness);

    app.harness.get_by_label("Saved automatically");
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
    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, id))
        .unwrap()
        .unwrap();
    assert_eq!(stored.template.url, "https://example.com/health?verbose=1");
    assert_eq!(
        stored.template.queries.len(),
        1,
        "typing the URL fills the parameter rows"
    );
    assert_eq!(stored.template.queries[0].name, "verbose");
}

#[test]
fn sending_shows_the_response_and_records_a_run() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"ok":true}"#,
        )
    }));
    let url = format!("{}/ping", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Ping", &url);
    });
    click(&mut app.harness, "GET Ping");
    settle(&mut app.harness);

    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    app.harness.get_by_label("Status: 200");
    app.harness.get_by_label_contains("\"ok\": true");
    app.harness.get_by_label("Recorded runs");
    assert_eq!(server.requests().len(), 1);
    let runs = app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap();
    assert_eq!(runs, 1);
}

#[test]
fn a_failed_send_explains_itself_without_recording() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Broken", "not a url");
    });
    click(&mut app.harness, "GET Broken");
    settle(&mut app.harness);

    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    app.harness
        .get_by_label("Request failed: Enter an absolute HTTP or HTTPS URL");
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| executions::count(db))
            .unwrap(),
        0
    );
}

fn manual_save(store: &poster_app::store::Store) {
    store
        .call_blocking(|db| {
            kv::set_app_settings(
                db,
                &AppSettings {
                    autosave: false,
                    ..AppSettings::default()
                },
            )
        })
        .unwrap();
}

#[test]
fn switching_requests_with_unsaved_changes_asks_and_discard_restores_the_saved_state() {
    let mut app = start_with(|store| {
        manual_save(store);
        let api = collection(store, "API");
        request(store, api.id, "First", "http://first");
        request(store, api.id, "Second", "http://second");
    });
    click(&mut app.harness, "GET First");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/changed");

    click(&mut app.harness, "GET Second");
    app.harness
        .get_by_label("Save changes to \"First\" before continuing?");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);

    app.harness.get_by_label("API / Second");
    let first = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::collections(db))
        .unwrap()[0]
        .id;
    let tree = app
        .harness
        .state()
        .store()
        .call_blocking(move |db| collections::tree(db, first))
        .unwrap()
        .unwrap();
    let first_id = tree.requests.iter().find(|r| r.name == "First").unwrap().id;
    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, first_id))
        .unwrap()
        .unwrap();
    assert_eq!(stored.template.url, "http://first");
}

#[test]
fn saving_from_the_guard_persists_then_navigates() {
    let mut app = start_with(|store| {
        manual_save(store);
        let api = collection(store, "API");
        request(store, api.id, "First", "http://first");
        request(store, api.id, "Second", "http://second");
    });
    click(&mut app.harness, "GET First");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/saved");
    app.harness.get_by_label("Unsaved request changes");

    click(&mut app.harness, "GET Second");
    click(&mut app.harness, "Save");
    settle(&mut app.harness);

    app.harness.get_by_label("API / Second");
    let stored = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        let tree = collections::tree(db, api)?.unwrap();
        let first = tree.requests.iter().find(|r| r.name == "First").unwrap().id;
        collections::request(db, first)
    });
    assert_eq!(stored.unwrap().unwrap().template.url, "http://first/saved");
}

#[test]
fn cancel_keeps_the_edits_and_the_current_request() {
    let mut app = start_with(|store| {
        manual_save(store);
        let api = collection(store, "API");
        request(store, api.id, "First", "http://first");
        request(store, api.id, "Second", "http://second");
    });
    click(&mut app.harness, "GET First");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/kept");

    click(&mut app.harness, "GET Second");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);

    app.harness.get_by_label("API / First");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .url,
        "http://first/kept"
    );
}

#[test]
fn closing_saves_pending_edits_and_ui_state_then_closes() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Only", "http://only");
    });
    click(&mut app.harness, "GET Only");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/edited");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);

    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    let store = app.harness.state().store().clone();
    let saved = store
        .call_blocking(|db| {
            let api = collections::collections(db)?[0].id;
            let tree = collections::tree(db, api)?.unwrap();
            collections::request(db, tree.requests[0].id)
        })
        .unwrap()
        .unwrap();
    assert_eq!(saved.template.url, "http://only/edited");
    let last: Option<serde_json::Value> = store
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "last"))
        .unwrap();
    assert_eq!(
        last.unwrap()["request_id"],
        serde_json::json!(saved.id.unwrap().0)
    );
}

#[test]
fn the_last_request_reopens_on_the_next_start() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Remembered", "http://remembered");
    });
    click(&mut app.harness, "GET Remembered");
    settle(&mut app.harness);
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    let dir = app.dir;
    drop(app.harness);

    let mut reopened = support::harness_at(dir.path(), egui::vec2(1400.0, 900.0));
    settle(&mut reopened);

    reopened.get_by_label("API / Remembered");
}

#[test]
fn deleting_the_open_request_clears_the_editor() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Doomed", "http://doomed");
    });
    click(&mut app.harness, "GET Doomed");
    settle(&mut app.harness);
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

    app.harness.state_mut().state.dialog = Some(poster_ui::dialogs::Dialog::Confirm {
        title: "Delete".into(),
        message: "Delete \"Doomed\"?".into(),
        confirm: "Delete".into(),
        purpose: poster_ui::dialogs::ConfirmPurpose::Delete(poster_ui::tree::NodeId::Request(id)),
    });
    app.harness.run_steps(2);
    support::click_button(&mut app.harness, "Delete");
    settle(&mut app.harness);

    app.harness.get_by_label("Select a request");
    assert!(
        app.harness
            .state()
            .store()
            .call_blocking(move |db| collections::request(db, id))
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_workspace_stacks_on_narrow_windows_and_sits_side_by_side_on_wide_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut narrow = support::harness_at(dir.path(), egui::vec2(1000.0, 650.0));
    narrow.run_steps(2);
    assert_eq!(
        narrow.state().state.orientation,
        Some(poster_ui::layout::Orientation::Stacked)
    );
    drop(narrow);

    let mut wide = support::harness_at(dir.path(), egui::vec2(1600.0, 900.0));
    wide.run_steps(2);
    assert_eq!(
        wide.state().state.orientation,
        Some(poster_ui::layout::Orientation::SideBySide)
    );
}
