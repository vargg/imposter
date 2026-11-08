//! Secrets in the app: defined from a blocked send on the Variables screen's Secrets
//! section, refused when too short, sent, always masked in the inspector, and captured
//! from a response with the capture's lock, never into a variable.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
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
        .call_blocking(|db| environments::active_environment(db))
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

    click(&mut f.app.harness, "Requests");
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
    click(&mut f.app.harness, "Variables");
    settle(&mut f.app.harness);
    let dev = dev(&f.app);
    f.app
        .harness
        .state_mut()
        .state
        .actions
        .push(imposter_ui::app::Action::Variables(
            imposter_ui::app::VariablesAction::SelectScope(Scope::Environment(dev)),
        ));
    settle(&mut f.app.harness);
    type_into(&mut f.app.harness, "Secret 1 value", "-edited");
    click(&mut f.app.harness, "Requests");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label_contains("Not set: secret:token");
}
