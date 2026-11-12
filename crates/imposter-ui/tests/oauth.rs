//! OAuth 2.0 in the app: the Auth tab says where the token stands, gets one on request and
//! clears it, and Send sends the token it has.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{OAuth2, RequestAuth, RequestTemplate, Variables};
use imposter_app::store::collections;
use imposter_testkit::{Received, Server, response};
use imposter_ui::app::EditorTab;
use support::{click, click_button, settle, start_with};

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    if received.target() == "/token" {
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"access_token": "token-from-endpoint", "token_type": "Bearer", "expires_in": 3600}"#,
        )
    } else {
        response("200 OK", &[], b"data")
    }
}

fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(reply));
    let base = server.url.clone();
    let app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                let api = collections::create_collection(db, "API", "", &Variables::new())?;
                collections::create_request(
                    db,
                    api.id,
                    None,
                    &RequestTemplate {
                        name: "Data".into(),
                        method: "GET".into(),
                        url: format!("{base}/data"),
                        auth: RequestAuth::oauth2(OAuth2 {
                            token_url: format!("{base}/token"),
                            client_id: "app".into(),
                            client_secret: "client-secret".into(),
                            ..OAuth2::default()
                        }),
                        ..RequestTemplate::default()
                    },
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

fn show_auth(f: &mut Fixture) {
    click(&mut f.app.harness, "GET Data");
    settle(&mut f.app.harness);
    f.app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Auth;
    f.app.harness.run_steps(2);
    settle(&mut f.app.harness);
}

#[test]
fn the_auth_tab_gets_shows_and_clears_the_token_and_send_uses_it() {
    let mut f = fixture();
    show_auth(&mut f);
    f.app.harness.get_by_label("No token yet: Send gets one.");
    assert!(f.app.harness.query_by_label("Clear token").is_none());

    click_button(&mut f.app.harness, "Get new token");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("Token valid for 59 min");
    f.app.harness.get_by_label("Got a new token");

    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    let requests = f.server.requests();
    assert_eq!(requests.len(), 2, "Send reused the token it had");
    assert_eq!(
        requests[1].header("authorization"),
        Some("Bearer token-from-endpoint")
    );

    click_button(&mut f.app.harness, "Clear token");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("No token yet: Send gets one.");
}

#[test]
fn a_refused_token_request_is_not_sent_and_says_why() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "401 Unauthorized",
            &[("Content-Type", "application/json")],
            br#"{"error": "invalid_client"}"#,
        )
    }));
    let mut f = Fixture {
        app: {
            let base = server.url.clone();
            start_with(move |store| {
                store
                    .call_blocking(move |db| {
                        let api = collections::create_collection(db, "API", "", &Variables::new())?;
                        collections::create_request(
                            db,
                            api.id,
                            None,
                            &RequestTemplate {
                                name: "Data".into(),
                                method: "GET".into(),
                                url: format!("{base}/data"),
                                auth: RequestAuth::oauth2(OAuth2 {
                                    token_url: format!("{base}/token"),
                                    client_id: "app".into(),
                                    ..OAuth2::default()
                                }),
                                ..RequestTemplate::default()
                            },
                        )?;
                        Ok(())
                    })
                    .unwrap();
            })
        },
        server,
        _runtime: runtime,
    };
    show_auth(&mut f);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("Token request failed: invalid_client");
    assert_eq!(
        f.server.requests().len(),
        1,
        "only the token request went out"
    );
}
