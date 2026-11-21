//! Secrets in the app: defined from a blocked send on the Variables screen's Secrets
//! section, refused when too short, sent, always masked in the inspector, and captured
//! from a response with the capture's lock, never into a variable.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{RequestTemplate, Variables};
use imposter_app::core::variables::{Scope, SecretScope};
use imposter_app::store::{collections, environments, scopes, secrets};
use imposter_testkit::{Received, Server, response};
use imposter_ui::app::Screen;
use support::{click, click_button, settle, start_with, type_into};

const TOKEN: &str = "dev-token-1234";
const ISSUED: &str = "issued-token-5678";

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    let json = [("Content-Type", "application/json")];
    match received.target() {
        "/login" => response(
            "200 OK",
            &json,
            format!(r#"{{"token": "{ISSUED}"}}"#).as_bytes(),
        ),
        _ => response("200 OK", &json, b"{}"),
    }
}

/// API with "Use" (sends `{{secret:token}}`) and "Login" (captures `token` from its body);
/// Dev is active and has no secrets yet.
fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(reply));
    let base = server.url.clone();
    let app =
        start_with(move |store| {
            store
            .call_blocking(move |db| {
                let dev = environments::create_environment(db, "Dev", &Variables::new())?;
                environments::set_active_environment(db, Some(dev.id))?;
                let api = collections::create_collection(
                    db,
                    "API",
                    "",
                    &[("base".to_owned(), base.clone())].into_iter().collect(),
                )?;
                let template = |name: &str, url: &str, extra: serde_json::Value| {
                    let mut value = serde_json::json!({
                        "name": name, "method": "GET", "url": url, "body": "", "body_type": "none",
                    });
                    if let (Some(map), serde_json::Value::Object(extra)) =
                        (value.as_object_mut(), extra)
                    {
                        map.extend(extra);
                    }
                    serde_json::from_value::<RequestTemplate>(value).unwrap()
                };
                collections::create_request(
                    db,
                    api.id,
                    None,
                    &template("Use", "{{base}}/use/{{secret:token}}", serde_json::json!({})),
                )?;
                collections::create_request(
                    db,
                    api.id,
                    None,
                    &template(
                        "Login",
                        "{{base}}/login",
                        serde_json::json!({
                            "captures": [{"variable": "token", "selector": {"json": "$.token"}}]
                        }),
                    ),
                )?;
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

fn dev(app: &support::App) -> imposter_app::core::ids::EnvironmentId {
    app.harness
        .state()
        .store()
        .call_blocking(environments::active_environment)
        .unwrap()
        .unwrap()
}

fn stored(app: &support::App) -> (Variables, Variables) {
    let dev = dev(app);
    app.harness
        .state()
        .store()
        .call_blocking(move |db| {
            Ok((
                secrets::read(db, SecretScope::Environment(dev))?,
                scopes::read(db, Scope::Environment(dev))?.0,
            ))
        })
        .unwrap()
}

#[test]
fn each_tab_shows_only_its_table_and_collections_offer_only_variables() {
    let mut app = start_with(|store| {
        support::collection(store, "API");
    });
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);

    let has = |app: &support::App, label| app.harness.query_by_label(label).is_some();
    assert!(has(&app, "New variable name"));
    assert!(!has(&app, "New secret name"));
    assert!(!has(&app, "No cookies yet."));

    click(&mut app.harness, "Secrets");
    assert!(has(&app, "New secret name"));
    assert!(!has(&app, "New variable name"));
    assert!(!has(&app, "No cookies yet."));
    support::assert_help(
        &app.harness,
        "Used as {{secret:name}}. Kept only on this computer: never exported, and removed from History.",
    );

    click(&mut app.harness, "Cookies");
    assert!(has(&app, "No cookies yet."));
    assert!(!has(&app, "New secret name"));
    assert!(!has(&app, "New variable name"));

    click(&mut app.harness, "Collection · API");
    settle(&mut app.harness);
    assert!(has(&app, "Variables tab"));
    assert!(has(&app, "New variable name"));
    assert!(!has(&app, "Secrets"));
    assert!(!has(&app, "Cookies"));
}

#[test]
fn variable_and_secret_composers_stay_out_of_the_saved_scope() {
    let mut app = start_with(|_| {});
    app.harness.set_size(egui::vec2(1000.0, 700.0));
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    assert_eq!(support::save_state(&app.harness, "Save variables"), "Saved");
    assert!(!app.harness.state().state.variables.dirty());
    assert_eq!(app.harness.state().state.variables.rows, []);
    assert_eq!(app.harness.state().state.variables.secrets, []);
    support::text_input(&app.harness, "New variable name");
    assert!(app.harness.query_by_label("Remove variable 1").is_none());
    click(&mut app.harness, "Secrets");
    support::text_input(&app.harness, "New secret name");
    assert!(app.harness.query_by_label("Remove secret 1").is_none());

    type_into(&mut app.harness, "New secret value", "dev-");
    assert!(app.harness.state().state.variables.dirty());
    assert!(support::text_input(&app.harness, "Secret 1 value").is_focused());
    app.harness.event(egui::Event::Text("token-1234".into()));
    app.harness.step();
    click(&mut app.harness, "Save variables");
    app.harness
        .get_by_label_contains("Name the secret before saving its value");
    type_into(&mut app.harness, "Secret 1 name", "token");
    click(&mut app.harness, "Variables tab");
    type_into(&mut app.harness, "New variable name", "host");
    type_into(&mut app.harness, "Add value to variable 1", "example.test");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);

    assert!(!app.harness.state().state.variables.dirty());
    let (values, secrets) = app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            Ok((
                environments::globals(db)?,
                secrets::read(db, SecretScope::Global)?,
            ))
        })
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values["host"], "example.test");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets["token"], TOKEN);
    assert_eq!(app.harness.state().state.variables.rows.len(), 1);
    assert_eq!(app.harness.state().state.variables.secrets.len(), 1);
    support::text_input(&app.harness, "New variable name");
    click(&mut app.harness, "Secrets");
    support::text_input(&app.harness, "New secret value");
}

#[test]
fn a_secret_is_defined_from_a_blocked_send_masked_and_sent() {
    let mut f = fixture();
    click(&mut f.app.harness, "GET Use");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    assert_eq!(f.server.requests().len(), 0, "nothing was sent");

    click(&mut f.app.harness, "Define {{secret:token}} in Dev secrets");
    settle(&mut f.app.harness);
    assert_eq!(f.app.harness.state().state.screen, Screen::Variables);
    assert_eq!(
        f.app.harness.state().state.variables.scope,
        Scope::Environment(dev(&f.app)),
    );
    assert_eq!(
        f.app
            .harness
            .get_by_label("Secrets")
            .accesskit_node()
            .is_selected(),
        Some(true),
    );
    assert!(support::text_input(&f.app.harness, "Secret 1 name").is_focused());
    assert_eq!(
        f.app.harness.state().state.variables.secrets[0].name,
        "token"
    );
    type_into(&mut f.app.harness, "Secret 1 value", "abc");
    click(&mut f.app.harness, "Save variables");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("needs at least 4 characters");
    assert!(stored(&f.app).0.is_empty(), "nothing saved");

    type_into(&mut f.app.harness, "Secret 1 value", &TOKEN[3..]);
    click(&mut f.app.harness, "Save variables");
    settle(&mut f.app.harness);
    let (dev_secrets, dev_variables) = stored(&f.app);
    assert_eq!(
        dev_secrets.get("token").map(String::as_str),
        Some(format!("abc{}", &TOKEN[3..]).as_str())
    );
    assert!(dev_variables.is_empty(), "a secret is not a variable");

    support::open_screen(&mut f.app.harness, "Requests");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    assert_eq!(
        f.server.requests()[0].target(),
        format!("/use/abc{}", &TOKEN[3..])
    );

    // The inspector hides a secret even when opened from a plain field.
    let anchor = support::text_input(&f.app.harness, "Request URL")
        .rect()
        .left_bottom();
    f.app.harness.state_mut().state.inspect = Some(imposter_ui::widgets::Inspection {
        name: "secret:token".into(),
        masked: false,
        anchor,
    });
    f.app.harness.run_steps(2);
    f.app.harness.get_by_label("••••••••");
    assert!(f.app.harness.query_by_label_contains(&TOKEN[3..]).is_none());
    click(&mut f.app.harness, "Reveal");
    f.app.harness.get_by_label_contains(&TOKEN[3..]);
}

#[test]
fn the_lock_stores_a_capture_as_a_secret_and_never_shows_it() {
    let mut f = fixture();
    click(&mut f.app.harness, "GET Login");
    settle(&mut f.app.harness);
    support::open_tab(&mut f.app.harness, "Captures");
    click_button(&mut f.app.harness, "Capture 1 stored as a variable");
    let captures = &f
        .app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .captures;
    assert!(captures[0].secret, "the lock marks it");
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);

    let (dev_secrets, dev_variables) = stored(&f.app);
    assert_eq!(dev_secrets.get("token").map(String::as_str), Some(ISSUED));
    assert!(dev_variables.is_empty());
    // The request has a Tests tab too, so the response's is chosen directly.
    f.app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Tests;
    f.app.harness.run_steps(2);
    f.app.harness.get_by_label_contains("secret in Dev secrets");
    assert!(f.app.harness.query_by_label_contains(ISSUED).is_none());
    f.app.harness.get_by_label_contains("set secret:token");

    // With unsaved edits to Dev's page, a capture into it waits rather than racing them.
    support::open_screen(&mut f.app.harness, "Variables");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Environment · Dev");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Secrets");
    type_into(&mut f.app.harness, "Secret 1 value", "-edited");
    support::open_screen(&mut f.app.harness, "Requests");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label_contains("Not set: secret:token");
}
