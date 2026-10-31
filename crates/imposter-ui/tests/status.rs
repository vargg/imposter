//! The status bar: the latest action's result in plain language, only while it applies, and
//! what is in progress.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui_kittest::kittest::Queryable;
use imposter_testkit::{Server, response};
use support::{click, collection, request, settle, start_with};

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
    let app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Ping", &format!("{base}/ping"));
        request(store, api.id, "Slow", &format!("{base}/slow"));
        request(store, api.id, "Broken", &format!("{base}/{{{{missing}}}}"));
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
        .get_by_label("Not sent: Unresolved variables: missing");
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
    app.harness.get_by_label_contains("Sending… ");
    settle(&mut app.harness);
    assert!(app.harness.query_by_label_contains("Sending… ").is_none());
    app.harness.get_by_label_contains("200 OK · ");
}

#[test]
fn the_right_end_names_the_environment() {
    let fixture = fixture();
    fixture.app.harness.get_by_label("No environment");
}
