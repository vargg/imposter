//! The status bar and the response pane's problems: the latest result in plain language,
//! only while it applies, what is in progress, and a way out of each failure.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui_kittest::kittest::Queryable;
use imposter_app::store::executions;
use imposter_testkit::{Server, response};
use imposter_ui::app::Screen;
use support::{click, collection, request, settle, start_with, text_input};

struct Fixture {
    app: support::App,
    _server: Server,
    _runtime: tokio::runtime::Runtime,
}

/// "Ping" answers at once, "Slow" after a second, "Broken" uses an undefined variable.
fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, received| {
        if received.request_line.contains("/slow") {
            std::thread::sleep(Duration::from_secs(1));
        }
        response("200 OK", &[], b"ok")
    }));
    let base = server.url.clone();
    // A port nothing listens on: bound, then released.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Ping", &format!("{base}/ping"));
        request(store, api.id, "Slow", &format!("{base}/slow"));
        request(store, api.id, "Broken", &format!("{base}/{{{{missing}}}}"));
        request(store, api.id, "Offline", &format!("http://{closed}/ping"));
        request(
            store,
            api.id,
            "Long",
            &format!("{base}/{{{{a_rather_long_customer_identifier}}}}"),
        );
    });
    Fixture {
        app,
        _server: server,
        _runtime: runtime,
    }
}

fn open_and_send(app: &mut support::App, name: &str) {
    click(&mut app.harness, name);
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
}

#[test]
fn results_read_plainly_and_a_blocked_send_never_shows_an_earlier_result() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    open_and_send(app, "GET Ping");
    app.harness.get_by_label_contains("200 OK · ");

    open_and_send(app, "GET Broken");
    app.harness.get_by_label("Not sent");
    app.harness
        .get_by_label("Not sent: {{missing}} is not defined");
    assert!(
        app.harness.query_by_label_contains("200 OK · ").is_none(),
        "the earlier result is gone"
    );
    assert!(
        app.harness
            .query_by_label_contains("TransportError")
            .is_none()
    );
}

#[test]
fn a_message_about_a_request_goes_with_it() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    open_and_send(app, "GET Ping");
    app.harness.get_by_label_contains("200 OK · ");

    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    assert!(app.harness.query_by_label_contains("200 OK · ").is_none());
    click(&mut app.harness, "Requests");
    settle(&mut app.harness);
    assert!(
        app.harness.query_by_label_contains("200 OK · ").is_none(),
        "it does not come back"
    );

    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    click(&mut app.harness, "GET Slow");
    settle(&mut app.harness);
    assert!(
        app.harness.query_by_label_contains("200 OK · ").is_none(),
        "another request clears it"
    );
}

#[test]
fn a_running_send_shows_its_time() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    click(&mut app.harness, "GET Slow");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.query_all_by_label_contains("Sending… ").count(),
        2,
        "the status bar and the response pane"
    );
    assert!(
        app.harness
            .query_by_label("Your response will appear here")
            .is_none(),
        "no invitation to send while sending"
    );
    settle(&mut app.harness);
    assert!(app.harness.query_by_label_contains("Sending… ").is_none());
    app.harness.get_by_label_contains("200 OK · ");
}

#[test]
fn the_right_end_names_the_environment() {
    let fixture = fixture();
    fixture.app.harness.get_by_label("No environment");
}

fn runs(app: &support::App) -> u64 {
    app.harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap()
}

#[test]
fn an_undefined_variable_can_be_defined_from_the_response_pane() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    open_and_send(app, "GET Ping");
    open_and_send(app, "GET Broken");
    assert!(
        app.harness.query_by_label_contains("200 OK").is_none(),
        "no earlier response beside the problem"
    );
    assert!(
        app.harness
            .query_by_label("Your response will appear here")
            .is_none()
    );
    click(&mut app.harness, "Define {{missing}} in Global");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Variables);
    let rows = &app.harness.state().state.variables.rows;
    assert_eq!(rows.last().map(|r| r.name.as_str()), Some("missing"));
    assert!(
        app.harness.state().state.variables.dirty(),
        "added, not yet saved"
    );
    assert_eq!(
        text_input(&app.harness, &format!("Variable {} name", rows.len()))
            .value()
            .as_deref(),
        Some("missing")
    );
}

#[test]
fn a_connection_failure_reads_plainly_fits_and_can_be_retried() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    open_and_send(app, "GET Offline");
    app.harness.get_by_label_contains("Couldn't connect · ");
    let hint = app
        .harness
        .get_by_label_contains("Is the server running")
        .rect();
    let window = app.harness.ctx.content_rect();
    assert!(
        hint.right() <= window.right() - 16.0,
        "{hint:?} wraps inside the pane"
    );
    assert!(
        app.harness.query_by_label_contains("os error").is_none(),
        "the raw text waits under Details"
    );
    click(&mut app.harness, "Details");
    app.harness.get_by_label_contains("Connection refused");
    assert!(
        app.harness
            .query_by_label_contains("TransportError")
            .is_none()
    );

    assert_eq!(runs(app), 1);
    // At the smallest window Retry is still reachable.
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    let retry = app.harness.get_by_label("Retry");
    retry.scroll_to_me();
    app.harness.run_steps(12);
    let rect = app.harness.get_by_label("Retry").rect();
    assert!(
        app.harness.ctx.content_rect().contains_rect(rect),
        "{rect:?}"
    );
    click(&mut app.harness, "Retry");
    settle(&mut app.harness);
    assert_eq!(runs(app), 2, "Retry sends again");

    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness
        .query_all_by_label_contains("History GET")
        .next()
        .unwrap()
        .click();
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.state_mut().state.history.tab = imposter_ui::app::HistoryTab::Response;
    app.harness.run_steps(2);
    app.harness.get_by_label_contains("Is the server running");
    assert!(
        app.harness.query_by_label("Retry").is_none(),
        "History only shows what happened"
    );
}

#[test]
fn a_long_define_button_keeps_one_line_in_the_smallest_window() {
    let mut fixture = fixture();
    let app = &mut fixture.app;
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    open_and_send(app, "GET Long");
    let define = app
        .harness
        .get_by_label_contains("Define {{a_rather_long_customer_identifier}}")
        .rect();
    let open = app.harness.get_by_label("Open Variables").rect();
    assert!(
        define.height() <= open.height() + 0.5,
        "{define:?} is taller than {open:?}: its label wrapped"
    );
    assert!(define.right() <= app.harness.ctx.content_rect().right());
}
