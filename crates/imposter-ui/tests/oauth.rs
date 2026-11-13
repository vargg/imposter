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

/// A request that signs in, against a token endpoint that gives `signed-in` for code `c-1`.
fn signing_in() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, received: &Received| {
        if received.target() == "/token" {
            let body = String::from_utf8_lossy(&received.body).into_owned();
            if body.contains("grant_type=authorization_code&code=c-1") {
                return response(
                    "200 OK",
                    &[("Content-Type", "application/json")],
                    br#"{"access_token": "signed-in", "token_type": "Bearer", "expires_in": 3600}"#,
                );
            }
            return response("400 Bad Request", &[], b"");
        }
        response("200 OK", &[], b"me")
    }));
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
                            grant: imposter_app::core::model::OAuthGrant::AuthorizationCode,
                            auth_url: "https://auth.test/authorize".into(),
                            token_url: format!("{base}/token"),
                            client_id: "desktop".into(),
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

/// Send the browser back to the redirect URI in `url` with code `c-1` and its state.
fn come_back(url: &str) {
    let query = imposter_app::core::url::parse_pairs(&imposter_app::core::url::split(url).query);
    let get = |name: &str| {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let address = get("redirect_uri")
        .trim_start_matches("http://")
        .trim_end_matches("/callback")
        .to_owned();
    let target = format!("/callback?code=c-1&state={}", get("state"));
    std::thread::spawn(move || {
        use std::io::{Read as _, Write as _};
        let mut stream = std::net::TcpStream::connect(&address).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
        let mut page = String::new();
        let _ = stream.read_to_string(&mut page);
    });
}

#[test]
fn a_send_that_needs_sign_in_offers_it_and_then_sends() {
    let mut f = signing_in();
    f.app.harness.state_mut().script_browser(|url| {
        come_back(url);
        Ok(())
    });
    click(&mut f.app.harness, "GET Data");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label_contains("Sign in to get a token: there is no token yet");
    assert_eq!(f.server.requests().len(), 0);

    click_button(&mut f.app.harness, "Sign in");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("Got a new token");

    click(&mut f.app.harness, "Send");
    settle(&mut f.app.harness);
    let requests = f.server.requests();
    assert_eq!(
        requests.last().unwrap().header("authorization"),
        Some("Bearer signed-in")
    );
}

#[test]
fn a_sign_in_can_be_cancelled_while_it_waits() {
    let mut f = signing_in();
    // A browser where nobody signs in.
    f.app.harness.state_mut().script_browser(|_| Ok(()));
    show_auth(&mut f);
    f.app
        .harness
        .get_by_label("Not signed in: Sign in to get a token.");
    click_button(&mut f.app.harness, "Sign in");
    f.app.harness.run_steps(4);
    f.app
        .harness
        .get_by_label("Waiting for you to sign in in the browser…");

    click_button(&mut f.app.harness, "Cancel sign-in");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("Sign-in cancelled");
}
