//! Keyboard shortcuts: Send, Save, New and Find, and that dialogs block them.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::{collections, kv};
use imposter_testkit::{Server, response};
use imposter_ui::app::Screen;
use support::{click, collection, request, settle, start, start_with, type_into};

fn press(app: &mut support::App, key: Key) {
    app.harness.key_press_modifiers(Modifiers::COMMAND, key);
    app.harness.run_steps(2);
}

fn manual_save(store: &imposter_app::store::Store) {
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
fn send_shortcuts_send_the_open_request() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let url = server.url.clone();
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", &url);
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);

    press(&mut app, Key::Enter);
    settle(&mut app.harness);
    press(&mut app, Key::R);
    settle(&mut app.harness);

    assert_eq!(server.requests().len(), 2);
}

#[test]
fn save_shortcut_saves_the_request() {
    let mut app = start_with(|store| {
        manual_save(store);
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h");
    });
    app.harness.state_mut().state.settings.autosave = false;
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/x");

    press(&mut app, Key::S);
    settle(&mut app.harness);

    let saved = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::requests(db, imposter_app::core::ids::CollectionId(1)))
        .unwrap();
    assert_eq!(saved[0].template.url, "http://h/x");
}

#[test]
fn new_shortcut_creates_a_request_or_first_a_collection() {
    let mut app = start();
    press(&mut app, Key::N);
    app.harness.get_by_label("New Collection");
    click(&mut app.harness, "Cancel");

    let mut app = start_with(|store| {
        collection(store, "API");
    });
    press(&mut app, Key::N);
    app.harness.get_by_label("New Request");
}

#[test]
fn shortcuts_do_nothing_while_a_dialog_is_open_or_off_their_screen() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let url = server.url.clone();
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", &url);
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);

    press(&mut app, Key::N);
    press(&mut app, Key::Enter);
    settle(&mut app.harness);
    assert_eq!(server.requests().len(), 0, "the dialog blocks Send");
    click(&mut app.harness, "Cancel");

    click(&mut app.harness, "History");
    settle(&mut app.harness);
    press(&mut app, Key::Enter);
    press(&mut app, Key::N);
    settle(&mut app.harness);
    assert_eq!(
        server.requests().len(),
        0,
        "Send belongs to the Requests screen"
    );
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(app.harness.state().state.screen, Screen::History);
}

#[test]
fn find_opens_in_the_viewer_under_the_pointer() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h");
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    app.harness
        .state_mut()
        .state
        .response
        .show(imposter_app::core::execution::Response {
            status: 200,
            body: b"alpha".to_vec(),
            ..Default::default()
        });
    app.harness.run_steps(2);
    let body = app.harness.get_by_label("Copy body").rect();
    app.harness.hover_at(body.center() + egui::vec2(0.0, 80.0));
    app.harness.run_steps(1);

    press(&mut app, Key::F);

    app.harness.get_by_label("Find in response body");
}

#[test]
fn the_save_icon_saves_and_the_status_is_one_word() {
    use egui_kittest::kittest::NodeT;
    let mut app = start_with(|store| {
        manual_save(store);
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h");
    });
    app.harness.state_mut().state.settings.autosave = false;
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    let disabled = |app: &support::App| {
        app.harness
            .get_by_label("Save request")
            .accesskit_node()
            .is_disabled()
    };
    app.harness.get_by_label("Saved");
    assert!(disabled(&app), "nothing to save");

    type_into(&mut app.harness, "Request URL", "/x");
    app.harness.get_by_label("Unsaved");
    assert!(!disabled(&app));
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);

    app.harness.get_by_label("Saved");
    let saved = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::requests(db, imposter_app::core::ids::CollectionId(1)))
        .unwrap();
    assert_eq!(saved[0].template.url, "http://h/x");
}

fn with_open_request() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h/old");
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    app
}

#[test]
fn select_url_shortcut_lets_typing_replace_the_url() {
    let mut app = with_open_request();
    press(&mut app, Key::L);
    app.harness.run_steps(2);
    app.harness.key_press(Key::Backspace);
    app.harness.run_steps(1);
    support::text_input(&app.harness, "Request URL").type_text("http://h/new");
    app.harness.run_steps(2);

    let pane = app.harness.state().state.editor.as_ref().unwrap();
    assert_eq!(pane.editor.url, "http://h/new");
}

#[test]
fn duplicate_and_rename_shortcuts_act_on_the_open_request() {
    let mut app = with_open_request();
    press(&mut app, Key::D);
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .state
        .tree
        .as_ref()
        .unwrap()
        .source
        .requests
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");

    app.harness.key_press(Key::F2);
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Rename");
    click(&mut app.harness, "Cancel");
    app.harness
        .key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::R);
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Rename");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);
    let sent = app
        .harness
        .state()
        .store()
        .call_blocking(|db| imposter_app::store::executions::count(db))
        .unwrap();
    assert_eq!(sent, 0, "Ctrl+Shift+R renames; it does not send");
}

#[test]
fn sidebar_settings_and_the_shortcut_sheet() {
    let mut app = with_open_request();
    assert!(app.harness.state().state.layout.sidebar_visible);
    press(&mut app, Key::B);
    assert!(!app.harness.state().state.layout.sidebar_visible);
    press(&mut app, Key::B);
    assert!(app.harness.state().state.layout.sidebar_visible);

    app.harness
        .key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Slash);
    app.harness.run_steps(2);
    app.harness.get_by_label("Keyboard shortcuts");
    app.harness
        .get_by_label("Show or hide the collections sidebar");
    press(&mut app, Key::B);
    assert!(
        app.harness.state().state.layout.sidebar_visible,
        "shortcuts are inert while the sheet is open"
    );
    click(&mut app.harness, "Close");

    press(&mut app, Key::Comma);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Settings);
}
