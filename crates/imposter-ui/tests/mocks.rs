//! The real mock screen, database, listener, and close guards.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // tests drive the app outside its frame loop

mod support;

use std::io::{Read as _, Write as _};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use imposter_app::core::mock::{MockConfig, MockRoute};
use imposter_app::store::kv::{self, Table};
use imposter_ui::app::CloseState;
use support::{click, settle, start_with, type_into};

fn opened(port: u16) -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(move |db| {
                kv::set(
                    db,
                    Table::Settings,
                    "mock_server",
                    &MockConfig {
                        port,
                        routes: vec![MockRoute {
                            body: "first".into(),
                            ..Default::default()
                        }],
                    },
                )
            })
            .unwrap();
    });
    click(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    app
}

fn wait_for(app: &mut support::App, ready: impl Fn(&imposter_ui::app::ImposterApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready(app.harness.state()) {
        assert!(
            Instant::now() < deadline,
            "mock service did not reach the expected state"
        );
        app.harness.run_steps(2);
        std::thread::yield_now();
    }
    app.harness.run_steps(2);
}

fn address(app: &support::App) -> String {
    app.harness
        .state()
        .state
        .mocks
        .run
        .as_ref()
        .unwrap()
        .address
        .clone()
        .unwrap()
}

fn response(url: &str) -> String {
    let mut stream = std::net::TcpStream::connect(url.trim_start_matches("http://")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET /?ignored=yes HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn start(app: &mut support::App) {
    click(&mut app.harness, "Start mock server");
    wait_for(app, |app| {
        app.state
            .mocks
            .run
            .as_ref()
            .is_some_and(|run| run.address.is_some())
    });
}

#[test]
fn saved_edits_apply_only_after_restart_and_close_stops_the_listener() {
    let mut app = opened(0);
    assert!(
        app.harness.state().state.mocks.run.is_none(),
        "never starts automatically"
    );
    start(&mut app);
    let first = address(&app);
    assert!(response(&first).ends_with("\r\n\r\nfirst"));
    click(&mut app.harness, "Mock route 1");
    type_into(&mut app.harness, "Mock response body", "-edited");
    click(&mut app.harness, "Save mocks");
    wait_for(&mut app, |app| !app.state.mocks.dirty());
    app.harness.get_by_label("Restart to apply");
    assert!(response(&first).ends_with("\r\n\r\nfirst"));
    click(&mut app.harness, "Stop mock server");
    settle(&mut app.harness);
    start(&mut app);
    let second = address(&app);
    assert!(response(&second).ends_with("\r\n\r\nfirst-edited"));
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    assert!(std::net::TcpStream::connect(second.trim_start_matches("http://")).is_err());
    let rebound = std::net::TcpListener::bind(second.trim_start_matches("http://")).unwrap();
    drop(rebound);
}

#[test]
fn reload_and_close_guard_unsaved_configuration_and_failed_saves() {
    let mut app = opened(0);
    click(&mut app.harness, "Mock route 1");
    type_into(&mut app.harness, "Mock response body", "-kept");
    click(&mut app.harness, "Requests");
    click(&mut app.harness, "Mocks");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].body,
        "first-kept"
    );
    click(&mut app.harness, "Reload mocks");
    app.harness.get_by_label("Unsaved mocks");
    click(&mut app.harness, "Cancel");
    assert!(app.harness.state().state.mocks.dirty());
    app.harness.state().store().call_blocking(|db| {
        db.execute_batch("CREATE TRIGGER fail_mock BEFORE INSERT ON settings WHEN NEW.key = 'mock_server' BEGIN SELECT RAISE(FAIL, 'injected save failure'); END;")?;
        Ok(())
    }).unwrap();
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness.get_by_label("Unsaved mocks");
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Open);
    assert!(app.harness.state().state.mocks.dirty());
    app.harness.get_by_label("Save failed");
    click(&mut app.harness, "Reload mocks");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.mocks.form.routes[0].body, "first");
}

#[test]
fn occupied_ports_report_an_error_and_mock_controls_fit_the_minimum_window() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut app = opened(listener.local_addr().unwrap().port());
    click(&mut app.harness, "Start mock server");
    wait_for(&mut app, |app| app.state.mocks.notice.is_some());
    assert!(app.harness.state().state.mocks.run.is_none());
    assert!(
        app.harness
            .state()
            .state
            .mocks
            .notice
            .as_ref()
            .unwrap()
            .contains("Mock server")
    );
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    click(&mut app.harness, "Mock route 1");
    let viewport = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::Vec2::from(imposter_ui::layout::MIN_WINDOW),
    );
    for orientation in [
        imposter_ui::layout::Orientation::SideBySide,
        imposter_ui::layout::Orientation::Stacked,
    ] {
        app.harness.state_mut().state.layout.orientation = orientation;
        app.harness.run_steps(2);
        for label in [
            "Save mocks",
            "Mock port",
            "Start mock server",
            "Reload mocks",
            "Mock method",
            "Mock path",
        ] {
            assert!(
                viewport.contains_rect(app.harness.get_by_label(label).rect()),
                "{label}"
            );
        }
    }
}

#[test]
fn route_editing_validation_two_click_removal_and_save_shortcut() {
    let mut app = opened(0);
    click(&mut app.harness, "Add mock route");
    click(&mut app.harness, "Save mocks");
    settle(&mut app.harness);
    app.harness.get_by_label("Save failed");
    assert!(
        app.harness
            .state()
            .state
            .mocks
            .error
            .as_ref()
            .unwrap()
            .contains("already has this method")
    );
    type_into(&mut app.harness, "Mock path", "new");
    type_into(&mut app.harness, "Mock response body", "response");
    click(&mut app.harness, "Add mock header");
    type_into(&mut app.harness, "Mock header 1 name", "X-Mock");
    type_into(&mut app.harness, "Mock header 1 value", "yes");
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    app.harness.run_steps(2);
    settle(&mut app.harness);
    assert!(!app.harness.state().state.mocks.dirty());
    let saved: MockConfig = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, Table::Settings, "mock_server"))
        .unwrap()
        .unwrap();
    assert_eq!(saved.routes[1].path, "/new");
    assert_eq!(saved.routes[1].headers[0].value, "yes");
    click(&mut app.harness, "Remove mock header 1");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[1].headers.len(),
        1
    );
    click(&mut app.harness, "Click again to remove mock header 1");
    assert_eq!(app.harness.state().state.mocks.form.routes[1].headers, []);
    click(&mut app.harness, "Remove mock route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 2);
    click(&mut app.harness, "Click again to remove mock route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 1);
    click(&mut app.harness, "Reload mocks");
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert!(!app.harness.state().state.mocks.dirty());
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 1);
}

#[test]
fn pending_reload_disables_fields_until_the_database_reply_arrives() {
    use egui_kittest::kittest::NodeT as _;
    let mut app = opened(0);
    click(&mut app.harness, "Mock route 1");
    let store = app.harness.state().store().clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocked = std::thread::spawn(move || {
        store
            .call_blocking(move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
            .unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    click(&mut app.harness, "Reload mocks");
    let port_disabled = app
        .harness
        .get_by_label("Mock port")
        .accesskit_node()
        .is_disabled();
    let path_disabled = app
        .harness
        .get_by_label("Mock path")
        .accesskit_node()
        .is_disabled();
    release_tx.send(()).unwrap();
    blocked.join().unwrap();
    assert!(
        port_disabled && path_disabled,
        "loading must not overwrite fresh edits"
    );
    settle(&mut app.harness);
    assert!(
        !app.harness
            .get_by_label("Mock path")
            .accesskit_node()
            .is_disabled()
    );
}
