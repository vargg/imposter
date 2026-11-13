//! OAuth 2.0 authorization code with PKCE end to end: signing in through a browser played
//! by the test against a loopback authorization server and token endpoint, then sending
//! with the token; a send never signs in, says so, and forgets a token that can no longer
//! be refreshed; the command line refuses.
#![cfg(test)]

use std::sync::{Arc, Mutex};

use imposter_app::document::RequestPath;
use imposter_app::executor::{ExecuteError, execute};
use imposter_app::oauth::{TokenState, fetch};
use imposter_app::profile::{self, Purpose};
use imposter_core::model::{OAuth2, OAuthGrant, Request, RequestAuth, RequestTemplate, Variables};
use imposter_core::url::{parse_pairs, split};
use imposter_store::{Store, collections, environments, oauth};
use imposter_testkit::{Received, Server, response};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;

struct Api {
    _dir: tempfile::TempDir,
    store: Store,
    server: Server,
    /// The PKCE challenge of the last authorization request, for the token endpoint.
    challenge: Arc<Mutex<String>>,
    /// What the token endpoint answers a refresh with.
    refresh_works: Arc<Mutex<bool>>,
    request: Request,
}

fn pairs(query: &str) -> Vec<(String, String)> {
    parse_pairs(query)
}

fn get(pairs: &[(String, String)], name: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

async fn api() -> Api {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let challenge: Arc<Mutex<String>> = Arc::default();
    let refresh_works = Arc::new(Mutex::new(true));
    let (expected, refreshes) = (challenge.clone(), refresh_works.clone());
    let server = Server::start(move |_, received: &Received| {
        let json = [("Content-Type", "application/json")];
        if received.target() != "/token" {
            return response("200 OK", &[], b"data");
        }
        let form = pairs(&String::from_utf8_lossy(&received.body));
        let token = |access: &str| {
            format!(
                r#"{{"access_token": "{access}", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "refresh-1"}}"#
            )
        };
        match get(&form, "grant_type").as_str() {
            "authorization_code"
                if get(&form, "code") == "code-1"
                    && imposter_http::sign_in::challenge(&get(&form, "code_verifier"))
                        == *expected.lock().unwrap() =>
            {
                response("200 OK", &json, token("signed-in").as_bytes())
            }
            "refresh_token" if *refreshes.lock().unwrap() => {
                response("200 OK", &json, token("refreshed").as_bytes())
            }
            _ => response("400 Bad Request", &json, br#"{"error": "invalid_grant"}"#),
        }
    })
    .await;
    let base = server.url.clone();
    let request = store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("base", &base)]))?;
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(
                db,
                collection.id,
                None,
                &RequestTemplate {
                    name: "me".into(),
                    method: "GET".into(),
                    url: "{{base}}/me".into(),
                    auth: RequestAuth::oauth2(OAuth2 {
                        grant: OAuthGrant::AuthorizationCode,
                        auth_url: "https://auth.test/authorize".into(),
                        token_url: "{{base}}/token".into(),
                        client_id: "desktop".into(),
                        scope: "openid".into(),
                        ..OAuth2::default()
                    }),
                    ..RequestTemplate::default()
                },
            )
        })
        .await
        .unwrap();
    Api {
        _dir: dir,
        store,
        server,
        challenge,
        refresh_works,
        request,
    }
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A browser where the person signs in and the authorization server sends them back with
/// `code-1` and the `state` it was given, or with `state` when one is named.
fn browser(
    api: &Api,
    state: Option<&'static str>,
) -> (imposter_app::oauth::OpenBrowser, Arc<Mutex<String>>) {
    let opened: Arc<Mutex<String>> = Arc::default();
    let (seen, challenge) = (opened.clone(), api.challenge.clone());
    let open = Box::new(move |url: &str| {
        *seen.lock().unwrap() = url.to_owned();
        let query = pairs(&split(url).query);
        *challenge.lock().unwrap() = get(&query, "code_challenge");
        let back = format!(
            "{}?code=code-1&state={}",
            get(&query, "redirect_uri"),
            state.map_or_else(|| get(&query, "state"), str::to_owned)
        );
        tokio::spawn(async move {
            let address = back
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap()
                .to_owned();
            let target = &back[back.find("/callback").unwrap()..];
            let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut page = String::new();
            let _ = stream.read_to_string(&mut page).await;
        });
        Ok(())
    });
    (open, opened)
}

async fn sign_in(
    api: &Api,
    state: Option<&'static str>,
) -> (Result<TokenState, ExecuteError>, String) {
    let (open, opened) = browser(api, state);
    let result = fetch(
        &api.store.handle(),
        &api.request,
        &CancellationToken::new(),
        open,
        imposter_app::executor::now().0,
    )
    .await;
    let url = opened.lock().unwrap().clone();
    (result, url)
}

async fn send(api: &Api) -> Result<imposter_app::executor::Executed, ExecuteError> {
    execute(
        &api.store.handle(),
        api.request.clone(),
        CancellationToken::new(),
    )
    .await
}

fn api_requests(api: &Api) -> Vec<Received> {
    api.server
        .requests()
        .into_iter()
        .filter(|r| r.target() == "/me")
        .collect()
}

#[tokio::test]
async fn a_send_without_a_token_asks_to_sign_in_and_sends_nothing() {
    let api = api().await;
    let error = send(&api).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Sign in to get a token: there is no token yet"
    );
    assert_eq!(api.server.requests().len(), 0);
}

#[tokio::test]
async fn signing_in_with_pkce_gives_the_token_a_send_uses() {
    let api = api().await;
    let (result, url) = sign_in(&api, None).await;

    assert!(
        matches!(
            result,
            Ok(TokenState::Good {
                expires_at: Some(_)
            })
        ),
        "{result:?}"
    );
    let query = pairs(&split(&url).query);
    assert!(url.starts_with("https://auth.test/authorize?response_type=code&client_id=desktop"));
    assert_eq!(get(&query, "code_challenge_method"), "S256");
    assert_eq!(get(&query, "scope"), "openid");
    assert!(get(&query, "redirect_uri").starts_with("http://127.0.0.1:"));
    assert_eq!(get(&query, "state").len(), 22);
    let token_request = &api.server.requests()[0];
    assert_eq!(
        token_request.header("authorization"),
        None,
        "a public client"
    );

    send(&api).await.unwrap();
    assert_eq!(
        api_requests(&api)[0].header("authorization"),
        Some("Bearer signed-in")
    );
}

#[tokio::test]
async fn another_sign_ins_answer_is_refused() {
    let api = api().await;
    let (result, _) = sign_in(&api, Some("forged")).await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Sign-in failed: the sign-in answer did not match this sign-in"
    );
    assert_eq!(api.server.requests().len(), 0, "no code was exchanged");
}

async fn expire(api: &Api) {
    let mut token = api
        .store
        .call(|db| oauth::read(db, None))
        .await
        .unwrap()
        .remove(0);
    token.expires_at = Some(0);
    api.store
        .call(move |db| oauth::put(db, None, &token))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_one_that_cannot_be_is_forgotten() {
    let api = api().await;
    sign_in(&api, None).await.0.unwrap();

    expire(&api).await;
    send(&api).await.unwrap();
    assert_eq!(
        api_requests(&api)[0].header("authorization"),
        Some("Bearer refreshed")
    );

    expire(&api).await;
    *api.refresh_works.lock().unwrap() = false;
    let error = send(&api).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Sign in to get a token: the token expired and could not be refreshed (invalid_grant)"
    );
    assert_eq!(api_requests(&api).len(), 1, "nothing more was sent");
    let left = api.store.call(|db| oauth::read(db, None)).await.unwrap();
    assert_eq!(
        left,
        [],
        "the dead token is forgotten, so the app says to sign in"
    );
}

#[tokio::test]
async fn the_command_line_never_signs_in_nor_borrows_the_apps_token() {
    let api = api().await;
    sign_in(&api, None).await.0.unwrap();

    let selected = profile::select(
        &api.store.handle(),
        &RequestPath::new(&["API"], "me"),
        None,
        &Variables::new(),
        Purpose::Run,
    )
    .await
    .unwrap();
    let error =
        imposter_app::executor::run(&selected.inputs, selected.request, CancellationToken::new())
            .await
            .unwrap_err();
    assert!(matches!(error, ExecuteError::SignIn { .. }), "{error:?}");
    assert_eq!(api_requests(&api).len(), 0);
}
