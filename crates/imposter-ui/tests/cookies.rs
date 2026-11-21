//! Cookies in the app: a login's cookie is kept and sent by the next request, listed
//! masked on the Variables screen under its environment, and removed one at a time or all
//! together, each after asking. Settings has the switch.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{RequestTemplate, Variables};
use imposter_app::store::{collections, environments};
use imposter_testkit::{Received, Server, response};
use support::{click, click_button, settle, start_with};

const SESSION: &str = "session-value-1234";

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    if received.target() == "/login" {
        let cookie = format!("session={SESSION}; Path=/; HttpOnly");
        response("200 OK", &[("Set-Cookie", &cookie)], b"in")
    } else {
        response("200 OK", &[], b"me")
    }
}

/// API with "Login" (sets a session cookie) and "Me"; Dev is active.
fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(reply));
    let base = server.url.clone();
    let app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                let dev = environments::create_environment(db, "Dev", &Variables::new())?;
                environments::set_active_environment(db, Some(dev.id))?;
                let api = collections::create_collection(db, "API", "", &Variables::new())?;
                for (name, path) in [("Login", "/login"), ("Me", "/me")] {
                    collections::create_request(
                        db,
                        api.id,
                        None,
                        &RequestTemplate {
                            name: name.into(),
                            method: "GET".into(),
                            url: format!("{base}{path}"),
                            ..RequestTemplate::default()
                        },
                    )?;
                }
                Ok(())
            })
            .unwrap();
    });
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn send(f: &mut Fixture, request: &str) {
    support::open_screen(&mut f.app.harness, "Requests");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, request);
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
}

fn show_dev_cookies(f: &mut Fixture) {
    support::open_screen(&mut f.app.harness, "Variables");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Environment · Dev");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Cookies");
}

#[test]
fn a_login_cookie_is_listed_masked_sent_on_and_removed_after_asking() {
    let mut f = fixture();
    send(&mut f, "GET Login");
    send(&mut f, "GET Me");
    assert_eq!(
        f.server.requests()[1].header("cookie"),
        Some(format!("session={SESSION}").as_str())
    );

    show_dev_cookies(&mut f);
    f.app.harness.get_by_label("session");
    f.app.harness.get_by_label("Cookie session value");
    support::assert_help(
        &f.app.harness,
        "Set by servers for requests sent with Dev, and sent back to them. Kept only on this computer.",
    );
    f.app
        .harness
        .get_by_label_contains("127.0.0.1 · / · until cleared");
    assert!(
        f.app.harness.query_by_label_contains(SESSION).is_none(),
        "the value is masked"
    );

    support::assert_icon_button(&f.app.harness, "Remove cookie session");
    click(&mut f.app.harness, "Remove cookie session");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("Requests sent with Dev stop sending it");
    click_button(&mut f.app.harness, "Remove cookie");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("No cookies yet.");

    send(&mut f, "GET Me");
    assert_eq!(f.server.requests()[2].header("cookie"), None);
}

#[test]
fn clearing_asks_with_the_count_and_settings_has_the_switch() {
    let mut f = fixture();
    send(&mut f, "GET Login");
    show_dev_cookies(&mut f);
    support::assert_icon_button(&f.app.harness, "Clear cookies…");
    click(&mut f.app.harness, "Clear cookies…");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("Remove 1 cookie kept for requests sent with Dev?");
    click_button(&mut f.app.harness, "Clear cookies");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("No cookies yet.");

    support::open_screen(&mut f.app.harness, "Settings");
    settle(&mut f.app.harness);
    support::open_settings_category(&mut f.app.harness, "Network");
    f.app
        .harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Keep cookies");
}
