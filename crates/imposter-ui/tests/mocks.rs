//! The real mock screen, database, listener, and close guards.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // tests drive the app outside its frame loop

mod support;

use std::io::{Read as _, Write as _};
use std::time::{Duration, Instant};

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::mock::{MockConfig, MockRoute};
use imposter_app::core::model::ParamItem;
use imposter_app::store::kv::{self, Table};
use imposter_ui::app::{CloseState, Screen};
use imposter_ui::theme::{self, Tone};
use support::{
    advance_time, click, click_button, manual_save, settle, start_with, text_input, type_into,
};

fn opened(port: u16) -> support::App {
    open_with_port(port, |_| {})
}

fn open_with_port(port: u16, prepare: impl FnOnce(&imposter_app::store::Store)) -> support::App {
    let mut app = start_with(|store| {
        prepare(store);
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
    support::open_screen(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    app
}

fn stored_mocks(app: &support::App) -> MockConfig {
    app.harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, Table::Settings, "mock_server"))
        .unwrap()
        .unwrap()
}

fn fail_mock_saves(app: &support::App) {
    app.harness.state().store().call_blocking(|db| {
        db.execute_batch("CREATE TRIGGER fail_mock BEFORE INSERT ON settings WHEN NEW.key = 'mock_server' BEGIN SELECT RAISE(FAIL, 'injected save failure'); END;")?;
        Ok(())
    }).unwrap();
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
fn route_edits_autosave_after_the_last_edit() {
    let mut app = opened(0);
    advance_time(&mut app.harness, 0.0);
    type_into(&mut app.harness, "Mock path", "health");
    advance_time(&mut app.harness, 0.4);
    type_into(&mut app.harness, "Mock response body", "-edited");
    advance_time(&mut app.harness, 0.2);
    settle(&mut app.harness);
    assert_eq!(stored_mocks(&app).routes[0].path, "/");
    assert!(app.harness.query_by_label("Unsaved").is_none());

    advance_time(&mut app.harness, 0.4);
    settle(&mut app.harness);
    let saved = stored_mocks(&app);
    assert_eq!(saved.routes[0].path, "/health");
    assert_eq!(saved.routes[0].body, "first-edited");
    assert_eq!(support::save_state(&app.harness, "Save mocks"), "Saved");
}

#[test]
fn invalid_mock_route_edits_stay_unsaved_and_guard_navigation() {
    let mut app = opened(0);
    advance_time(&mut app.harness, 0.0);
    click(&mut app.harness, "Add mock route");
    advance_time(&mut app.harness, 0.6);
    settle(&mut app.harness);
    assert_eq!(stored_mocks(&app).routes.len(), 1);
    app.harness.get_by_label("Save failed");
    app.harness.get_by_label_contains("already answers");

    support::open_screen(&mut app.harness, "Requests");
    settle(&mut app.harness);
    app.harness.get_by_label("Unsaved mocks");
    click(&mut app.harness, "Cancel");
    assert_eq!(app.harness.state().state.screen, Screen::Mocks);
}

#[test]
fn failed_autosave_keeps_mock_edits_and_allows_retry_before_leaving() {
    let mut app = opened(0);
    advance_time(&mut app.harness, 0.0);
    type_into(&mut app.harness, "Mock response body", "-edited");
    fail_mock_saves(&app);

    support::open_screen(&mut app.harness, "Requests");
    settle(&mut app.harness);
    app.harness.get_by_label("Unsaved mocks");
    assert_eq!(app.harness.state().state.screen, Screen::Mocks);
    assert_eq!(stored_mocks(&app).routes[0].body, "first");
    assert_eq!(
        support::save_state(&app.harness, "Save mocks"),
        "Save failed"
    );

    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            db.execute_batch("DROP TRIGGER fail_mock")?;
            Ok(())
        })
        .unwrap();
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Requests);
    assert_eq!(stored_mocks(&app).routes[0].body, "first-edited");
}

#[test]
fn closing_with_pending_mock_edits_saves_without_a_dialog() {
    let mut app = opened(0);
    advance_time(&mut app.harness, 0.0);
    type_into(&mut app.harness, "Mock response body", "-edited");
    assert_eq!(stored_mocks(&app).routes[0].body, "first");

    app.harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    assert!(app.harness.query_by_label("Unsaved mocks").is_none());
    assert_eq!(stored_mocks(&app).routes[0].body, "first-edited");
}

#[test]
fn saved_edits_apply_only_after_restart_and_close_stops_the_listener() {
    let mut app = open_with_port(0, manual_save);
    assert!(
        app.harness.state().state.mocks.run.is_none(),
        "never starts automatically"
    );
    start(&mut app);
    let first = address(&app);
    assert!(response(&first).ends_with("\r\n\r\nfirst"));
    let port = first.rsplit(':').next().unwrap().to_owned();
    app.harness.run_steps(2);
    let global = format!("Mocks :{port}");
    assert!(app.harness.query_by_label(&global).is_none());
    support::open_screen(&mut app.harness, "Requests");
    click(&mut app.harness, &global);
    click(&mut app.harness, "Mock route 1");
    click_button(&mut app.harness, "Body");
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
    let mut app = open_with_port(0, manual_save);
    click(&mut app.harness, "Mock route 1");
    click_button(&mut app.harness, "Body");
    type_into(&mut app.harness, "Mock response body", "-kept");
    // The Mocks rail item reveals unsaved edits.
    let marked = |app: &support::App| {
        app.harness
            .get_by_role_and_label(egui::Role::Button, "Mocks")
            .accesskit_node()
            .description()
    };
    assert_eq!(marked(&app).as_deref(), Some("Unsaved changes"));
    // Amber, as the word "Unsaved": a request tab's green dot means it holds something. It is
    // painted over the rail item's corner.
    let tab = app
        .harness
        .get_by_role_and_label(egui::Role::Button, "Mocks")
        .rect();
    let dots: Vec<egui::Color32> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if tab.contains(dot.center) => Some(dot.fill),
            _ => None,
        })
        .collect();
    assert_eq!(dots, [theme::palette().warning]);
    support::open_screen(&mut app.harness, "Requests");
    support::open_screen(&mut app.harness, "Mocks");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].body,
        "first-kept"
    );
    click(&mut app.harness, "Revert to saved mocks");
    app.harness.get_by_label("Unsaved mocks");
    click(&mut app.harness, "Cancel");
    assert!(app.harness.state().state.mocks.dirty());
    fail_mock_saves(&app);
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness.get_by_label("Unsaved mocks");
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Open);
    assert!(app.harness.state().state.mocks.dirty());
    app.harness.get_by_label("Save failed");
    click(&mut app.harness, "Revert to saved mocks");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.mocks.form.routes[0].body, "first");
    assert_eq!(marked(&app).as_deref(), Some("Mocks"));
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
            "Revert to saved mocks",
            "HTTP method",
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
    let mut app = open_with_port(0, manual_save);
    support::assert_icon_button(&app.harness, "Add mock route");
    click_button(&mut app.harness, "Headers");
    text_input(&app.harness, "New mock header name");
    text_input(&app.harness, "New mock header value");
    assert!(!app.harness.state().state.mocks.dirty());
    click(&mut app.harness, "Add mock route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 2);
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
            .contains("already answers")
    );
    type_into(&mut app.harness, "Mock path", "new");
    click_button(&mut app.harness, "Body");
    type_into(&mut app.harness, "Mock response body", "response");
    click_button(&mut app.harness, "Headers");
    type_into(&mut app.harness, "New mock header name", "X-Mock");
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
    assert_eq!(saved.routes[1].headers, [ParamItem::new("X-Mock", "yes")]);
    click(&mut app.harness, "Remove mock header 1");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[1].headers.len(),
        1
    );
    click(&mut app.harness, "Click again to remove mock header 1");
    assert_eq!(app.harness.state().state.mocks.form.routes[1].headers, []);
    click(&mut app.harness, "Delete route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 2);
    click(&mut app.harness, "Click again to remove mock route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 1);
    click(&mut app.harness, "Revert to saved mocks");
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
    click(&mut app.harness, "Revert to saved mocks");
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

#[test]
fn route_problems_show_beside_their_fields_while_typing() {
    let mut app = opened(0);
    click(&mut app.harness, "Mock route 1");
    type_into(&mut app.harness, "Mock path", "orders bad path");
    app.harness
        .get_by_label("Paths can't contain spaces; write %20");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].path,
        "/orders bad path"
    );

    click(&mut app.harness, "Save mocks");
    settle(&mut app.harness);
    app.harness.get_by_label("Save failed");
    let start = app.harness.get_by_label("Start mock server");
    assert!(start.accesskit_node().is_disabled());
    start.hover();
    app.harness.run_steps(60);
    app.harness
        .get_by_label("Fix route 1 first: Paths can't contain spaces; write %20");

    click(&mut app.harness, "Add mock route");
    click(&mut app.harness, "Add mock route");
    app.harness
        .get_by_label("Another enabled route already answers GET /");
}

#[test]
fn the_server_state_reads_in_words_and_its_controls_stay_put() {
    let mut app = opened(0);
    // Port 0 picks a free port: it reads "auto".
    assert_eq!(
        app.harness
            .get_by_label("Mock port")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("auto")
    );
    assert_eq!(app.harness.query_all_by_label("Stopped").count(), 1);
    let start_at = app.harness.get_by_label("Start mock server").rect();
    start(&mut app);
    let url = address(&app);
    assert_eq!(
        app.harness
            .query_all_by_label(&format!("Running at {url}"))
            .count(),
        1
    );
    assert!(app.harness.query_by_label_contains("Mocks :").is_none());
    assert!(app.harness.query_by_label("Stopped").is_none());
    support::assert_icon_button(&app.harness, "Delete route");
    let stop_at = app.harness.get_by_label("Stop mock server").rect();
    assert!(
        (stop_at.right() - start_at.right()).abs() < 0.5,
        "Start and Stop share a place: {start_at:?} {stop_at:?}"
    );
    app.harness.get_by_label("Copy mock server URL").click();
    app.harness.step();
    let copied: Vec<String> = app
        .harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, [url]);
    click(&mut app.harness, "Stop mock server");
    wait_for(&mut app, |app| app.state.mocks.run.is_none());
}

#[test]
fn routes_are_a_list_beside_the_selected_routes_editor() {
    let mut app = opened(0);
    let row = app.harness.get_by_label("Mock route 1").rect();
    // The first route is open without a click.
    let path = text_input(&app.harness, "Mock path").rect();
    assert!(path.left() > row.right(), "the editor is beside the list");
    // The list's box is the route's only on/off control; the editor has no second one.
    assert!(app.harness.query_by_label("Enabled").is_none());
    click(&mut app.harness, "Mock route 1 enabled");
    assert!(!app.harness.state().state.mocks.form.routes[0].enabled);
    click(&mut app.harness, "Mock route 1 enabled");
    // The method is picked from the list, as in the request bar.
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "POST");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].method,
        "POST"
    );
    // A route with a problem is marked in the list, and screen readers hear why.
    type_into(&mut app.harness, "Mock path", " bad");
    let marked = app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.job.text.contains('⚠'))
    });
    assert!(marked, "a broken route is marked");
    let description = app
        .harness
        .get_by_label("Mock route 1")
        .accesskit_node()
        .description();
    assert!(
        description
            .as_deref()
            .is_some_and(|d| d.contains("problem")),
        "{description:?}"
    );
    // Deleting takes two clicks.
    click(&mut app.harness, "Delete route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 1);
    click(&mut app.harness, "Click again to remove mock route");
    assert_eq!(app.harness.state().state.mocks.form.routes.len(), 0);
    app.harness
        .get_by_label("Add a route to answer requests with a fixed response.");
}

#[test]
fn the_mock_route_list_resizes_and_keeps_its_width_after_restart() {
    let mut app = opened(0);
    let wide = egui::vec2(1400.0, 900.0);
    let small = egui::Vec2::from(imposter_ui::layout::MIN_WINDOW);
    let editor = |app: &support::App| text_input(&app.harness, "Mock path").rect();
    let grip = |app: &support::App| {
        let row = app.harness.get_by_label("Mock route 1").rect();
        egui::pos2(row.right() + 6.0, 300.0)
    };
    app.harness.set_size(small);
    app.harness.run_steps(2);
    let narrow = editor(&app);
    app.harness.set_size(wide);
    app.harness.run_steps(2);
    let before = editor(&app);

    let edge = grip(&app);
    support::drag_horizontally(&mut app.harness, edge, 80.0);
    let wider = editor(&app);
    assert!(
        wider.left() > before.left() + 60.0,
        "{before:?} -> {wider:?}"
    );
    let edge = grip(&app);
    support::assert_split_grip(&app.harness, edge);
    support::drag_horizontally(&mut app.harness, edge, -280.0);
    let minimum = editor(&app);
    assert!(
        (before.left() - minimum.left() - 60.0).abs() <= 1.0,
        "{minimum:?}"
    );
    let edge = grip(&app);
    support::drag_horizontally(&mut app.harness, edge, 540.0);
    let maximum = editor(&app);
    assert!(
        (maximum.left() - before.left() - 220.0).abs() <= 1.0,
        "{maximum:?}"
    );

    app.harness.set_size(small);
    app.harness.run_steps(2);
    assert_eq!(
        editor(&app),
        narrow,
        "the editor keeps its room at 720 points"
    );
    app.harness.set_size(wide);
    app.harness.run_steps(2);
    assert_eq!(editor(&app), maximum, "the preferred width returns");
    app.harness.set_size(small);
    app.harness.run_steps(2);
    click_button(&mut app.harness, "Headers");
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    drop(app.harness);
    app.harness = support::harness_at(app.dir.path(), wide);
    support::open_screen(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    assert_eq!(editor(&app), maximum);
    text_input(&app.harness, "Mock response body");
}

fn opened_with(routes: Vec<MockRoute>) -> support::App {
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                kv::set(
                    db,
                    Table::Settings,
                    "mock_server",
                    &MockConfig { port: 0, routes },
                )
            })
            .unwrap();
    });
    support::open_screen(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    app
}

fn scripted(script: &str) -> MockRoute {
    MockRoute {
        script: script.into(),
        ..Default::default()
    }
}

#[test]
fn adding_response_headers_does_not_move_or_resize_the_body() {
    let mut app = opened(0);
    let body = text_input(&app.harness, "Mock response body").rect();
    click_button(&mut app.harness, "Script");
    let script = text_input(&app.harness, "Mock response script").rect();
    click_button(&mut app.harness, "Headers");
    for name in ["X-First", "X-Second", "X-Third"] {
        type_into(&mut app.harness, "New mock header name", name);
    }
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].headers.len(),
        3
    );
    click_button(&mut app.harness, "Body");
    assert_eq!(
        text_input(&app.harness, "Mock response body").rect(),
        body,
        "response headers must leave the body editor in place"
    );
    click_button(&mut app.harness, "Script");
    assert_eq!(
        text_input(&app.harness, "Mock response script").rect(),
        script
    );

    click_button(&mut app.harness, "Headers");
    click(&mut app.harness, "Remove mock header 2");
    click(&mut app.harness, "Click again to remove mock header 2");
    assert_eq!(
        app.harness.state().state.mocks.form.routes[0].headers.len(),
        2
    );
    click_button(&mut app.harness, "Script");
    assert_eq!(
        text_input(&app.harness, "Mock response script").rect(),
        script
    );
    click_button(&mut app.harness, "Body");
    assert_eq!(text_input(&app.harness, "Mock response body").rect(), body);
}

fn tab_description(app: &support::App, tab: &str) -> Option<String> {
    app.harness
        .get_by_role_and_label(egui::Role::Button, tab)
        .accesskit_node()
        .description()
}

fn assert_tab_mark(app: &support::App, tab: &str, mark: &str, tone: Tone) {
    let text = format!("{tab}{mark}");
    assert!(
        app.harness.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(shape) if shape.galley.job.text == text
            && shape.galley.job.sections.last().is_some_and(|section| {
                section.format.color == theme::palette().tone(tone)
            }))
        }),
        "expected {tab} to show {mark} in {tone:?}"
    );
}

#[test]
fn response_tabs_count_headers_in_use_and_mark_a_set_script() {
    let mut app = opened(0);
    assert_eq!(tab_description(&app, "Headers"), None);
    assert_eq!(tab_description(&app, "Script"), None);
    click_button(&mut app.harness, "Headers");
    for name in ["X-First", "X-Second"] {
        type_into(&mut app.harness, "New mock header name", name);
    }
    assert_eq!(
        tab_description(&app, "Headers").as_deref(),
        Some("2 in use")
    );
    assert_tab_mark(&app, "Headers", "2", Tone::Muted);
    click(&mut app.harness, "Mock header 2 enabled");
    assert_eq!(
        tab_description(&app, "Headers").as_deref(),
        Some("1 in use")
    );
    type_into(&mut app.harness, "New mock header value", "missing name");
    assert_eq!(
        tab_description(&app, "Headers").as_deref(),
        Some("1 in use. Header names are one word without spaces")
    );
    click_button(&mut app.harness, "Script");
    type_into(
        &mut app.harness,
        "Mock response script",
        "response.status = 201;",
    );
    click_button(&mut app.harness, "Body");
    assert_eq!(tab_description(&app, "Script").as_deref(), Some("Set"));
    assert_tab_mark(&app, "Script", "●", Tone::Ok);
    assert_tab_mark(&app, "Headers", "1", Tone::Error);
}

#[test]
fn a_hidden_header_problem_marks_the_tab_and_stays_beside_the_field() {
    let mut app = opened(0);
    click_button(&mut app.harness, "Headers");
    type_into(&mut app.harness, "New mock header value", "missing name");
    let error = "Header names are one word without spaces";
    app.harness.get_by_label(error);
    click_button(&mut app.harness, "Body");
    assert!(app.harness.query_by_label(error).is_none());
    assert_eq!(tab_description(&app, "Headers").as_deref(), Some(error));
    assert_tab_mark(&app, "Headers", "●", Tone::Error);
    click(&mut app.harness, "Save mocks");
    settle(&mut app.harness);
    app.harness.get_by_label("Save failed");
    text_input(&app.harness, "Mock response body");
    click_button(&mut app.harness, "Headers");
    app.harness.get_by_label(error);
    type_into(&mut app.harness, "Mock header 1 name", "X-Fixed");
    assert!(app.harness.query_by_label(error).is_none());
    assert_eq!(
        tab_description(&app, "Headers").as_deref(),
        Some("1 in use")
    );
}

#[test]
fn the_response_tab_stays_selected_when_switching_routes_and_screens() {
    let mut app = opened_with(vec![
        scripted("response.status = 201;"),
        MockRoute {
            path: "/other".into(),
            ..scripted("response.status = 202;")
        },
    ]);
    click_button(&mut app.harness, "Script");
    click(&mut app.harness, "Mock route 2");
    support::open_screen(&mut app.harness, "Requests");
    support::open_screen(&mut app.harness, "Mocks");
    assert_eq!(
        text_input(&app.harness, "Mock response script")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("response.status = 202;")
    );
    assert!(!app.harness.state().state.mocks.dirty());
}

#[test]
fn long_response_headers_scroll_below_the_route_fields_and_tabs() {
    let mut app = opened_with(vec![MockRoute {
        headers: (1..=20)
            .map(|index| ParamItem::new(format!("X-Header-{index}"), "value"))
            .collect(),
        ..MockRoute::default()
    }]);
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(2);
    click_button(&mut app.harness, "Headers");
    let tabs = app.harness.get_by_label("Headers").rect();
    let path = text_input(&app.harness, "Mock path").rect();
    let last = text_input(&app.harness, "Mock header 20 name");
    let bottom = support::status_bar(&app).top();
    assert!(last.rect().top() > bottom);
    last.scroll_to_me();
    app.harness.run_steps(12);
    let last = text_input(&app.harness, "Mock header 20 name").rect();
    assert!(
        last.top() > tabs.bottom() && last.bottom() < bottom,
        "{last:?}"
    );
    assert_eq!(app.harness.get_by_label("Headers").rect(), tabs);
    assert_eq!(text_input(&app.harness, "Mock path").rect(), path);
}

#[test]
fn the_mock_body_fills_its_tab_and_uses_the_request_json_highlighting() {
    let mut app = opened_with(vec![MockRoute {
        body: "{\"answer\": 42}".into(),
        ..MockRoute::default()
    }]);
    let body = text_input(&app.harness, "Mock response body").rect();
    let tabs = app.harness.get_by_label("Body").rect();
    assert!(
        (body.top() - tabs.bottom()).abs() < 24.0,
        "{body:?}, {tabs:?}"
    );
    assert!((support::status_bar(&app).top() - body.bottom()).abs() < 24.0);
    let has_number_color = |app: &support::App| {
        let color = theme::token_color(imposter_ui::highlight::Token::Number);
        app.harness.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("{\"answer\": 42}")
                && text.galley.job.sections.iter().any(|section| section.format.color == color))
        })
    };
    assert!(has_number_color(&app));
    app.harness.state_mut().state.mocks.form.routes[0]
        .body
        .push_str(&" ".repeat(imposter_ui::widgets::HIGHLIGHT_LIMIT));
    app.harness.run_steps(2);
    assert!(!has_number_color(&app));
}

#[test]
fn a_route_script_compiles_or_keeps_the_server_from_starting() {
    let mut app = opened_with(vec![scripted("response.status = 201;")]);
    click_button(&mut app.harness, "Script");
    let editor = text_input(&app.harness, "Mock response script").rect();
    assert!(editor.height() > 4.0 * theme::spacing().control);
    assert!((support::status_bar(&app).top() - editor.bottom()).abs() < 24.0);
    let status = app.harness.get_by_label("Compiles").rect();
    let examples = app
        .harness
        .get_by_label("Insert an example into the Mock response script")
        .rect();
    assert!((status.center().y - examples.center().y).abs() < 1.0);
    assert_eq!(app.harness.query_all_by_label("Compiles").count(), 1);
    let described = |app: &support::App| {
        app.harness
            .get_by_label("Mock route 1")
            .accesskit_node()
            .description()
    };
    assert_eq!(
        described(&app).as_deref(),
        Some("Status 200, answered by its script")
    );
    advance_time(&mut app.harness, 0.0);
    text_input(&app.harness, "Mock response script").click();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.run_steps(1);
    text_input(&app.harness, "Mock response script").type_text("let x = ;");
    app.harness.run_steps(3);
    app.harness.get_by_label_contains("Line 1, column 9");
    click_button(&mut app.harness, "Body");
    assert!(
        tab_description(&app, "Script")
            .unwrap()
            .contains("Mock script, line 1, column 9")
    );
    assert_tab_mark(&app, "Script", "●", Tone::Error);
    advance_time(&mut app.harness, 0.6);
    settle(&mut app.harness);
    assert_eq!(
        stored_mocks(&app).routes[0].script,
        "response.status = 201;"
    );
    app.harness.get_by_label("Save failed");
    click(&mut app.harness, "Save mocks");
    settle(&mut app.harness);
    assert!(
        !app.harness.state().state.mocks.dirty(),
        "a broken script can be saved"
    );
    let start = app.harness.get_by_label("Start mock server");
    assert!(
        start.accesskit_node().is_disabled(),
        "a broken script blocks Start"
    );
    assert_eq!(described(&app).as_deref(), Some("Has a problem"));
}

#[test]
fn the_request_log_shows_scripted_answers_and_errors() {
    let mut app = opened_with(vec![scripted(
        "state.calls = (state.calls ?? 0) + 1;\nprint(`call ${state.calls}`);\nif state.calls == 1 { response.status = 503; }\nif state.calls == 3 { throw \"third time\"; }",
    )]);
    start(&mut app);
    let url = address(&app);
    for _ in 0..3 {
        response(&url);
    }
    wait_for(&mut app, |app| app.state.mocks.log.len() == 3);
    app.harness.get_by_label("Answered requests");
    app.harness.get_by_label("→ 503");
    app.harness.get_by_label("→ 200");
    app.harness.get_by_label("→ 500");
    app.harness.get_by_label("Mock script, line 4: third time");
    app.harness.get_by_label("call 2");
    click(&mut app.harness, "Clear the mock request log");
    assert_eq!(app.harness.state().state.mocks.log.len(), 0);
    click(&mut app.harness, "Stop mock server");
    settle(&mut app.harness);
}
