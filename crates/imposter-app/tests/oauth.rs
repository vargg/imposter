//! OAuth 2.0 client credentials end to end, against a loopback token endpoint and API: the
//! first send fetches a token and the next reuses it, an expired one is refreshed, a failed
//! refresh asks for a new one, a refused request is not sent, a missing secret stops
//! before anything goes, and neither History nor a command-line run keeps a token.
#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use imposter_app::document::RequestPath;
use imposter_app::executor::{ExecuteError, execute};
use imposter_app::profile::{self, Purpose};
use imposter_core::model::{ClientAuth, OAuth2, Request, RequestAuth, RequestTemplate, Variables};
use imposter_store::oauth::StoredToken;
use imposter_store::{Store, collections, environments, executions, oauth};
use imposter_testkit::{Received, Server, response};
use tokio_util::sync::CancellationToken;

const SECRET: &str = "client-secret-1234";

struct Api {
    _dir: tempfile::TempDir,
    store: Store,
    server: Server,
    /// What the token endpoint answers next, in order; then a token "a-default".
    answers: Arc<Mutex<VecDeque<Vec<u8>>>>,
    request: Request,
}

fn token_answer(access: &str, extra: &str) -> Vec<u8> {
    response(
        "200 OK",
        &[("Content-Type", "application/json")],
        format!(r#"{{"access_token": "{access}", "token_type": "Bearer"{extra}}}"#).as_bytes(),
    )
}

async fn api(secret: &str) -> Api {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let answers: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::default();
    let queue = answers.clone();
    let server = Server::start(move |_, received: &Received| {
        if received.target() == "/token" {
            queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| token_answer("a-default", r#", "expires_in": 3600"#))
        } else {
            response("200 OK", &[], b"data")
        }
    })
    .await;
    let base = server.url.clone();
    let secret = secret.to_owned();
    let request = store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("base", &base)]))?;
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(
                db,
                collection.id,
                None,
                &RequestTemplate {
                    name: "data".into(),
                    method: "GET".into(),
                    url: "{{base}}/data".into(),
                    auth: RequestAuth::oauth2(OAuth2 {
                        token_url: "{{base}}/token".into(),
                        client_id: "app".into(),
                        client_secret: secret,
                        scope: "read".into(),
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
        answers,
        request,
    }
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn answer(api: &Api, answer: Vec<u8>) {
    api.answers.lock().unwrap().push_back(answer);
}

async fn send(api: &Api) -> Result<imposter_app::executor::Executed, ExecuteError> {
    execute(
        &api.store.handle(),
        api.request.clone(),
        CancellationToken::new(),
    )
    .await
}

/// What the server received, as `/token grant_type=…` or `/data Bearer …`.
fn received(api: &Api) -> Vec<String> {
    api.server
        .requests()
        .iter()
        .map(|r| {
            if r.target() == "/token" {
                format!("/token {}", String::from_utf8_lossy(&r.body))
            } else {
                format!(
                    "{} {}",
                    r.target(),
                    r.header("authorization").unwrap_or("-")
                )
            }
        })
        .collect()
}

async fn cached(api: &Api) -> Vec<StoredToken> {
    api.store.call(|db| oauth::read(db, None)).await.unwrap()
}

#[tokio::test]
async fn the_first_send_fetches_a_token_and_the_next_reuses_it() {
    let api = api(SECRET).await;
    answer(
        &api,
        token_answer(
            "access-token-1",
            r#", "expires_in": 3600, "refresh_token": "refresh-1""#,
        ),
    );

    let executed = send(&api).await.unwrap();
    send(&api).await.unwrap();

    assert_eq!(
        received(&api),
        [
            "/token grant_type=client_credentials&scope=read",
            "/data Bearer access-token-1",
            "/data Bearer access-token-1",
        ]
    );
    let token_request = &api.server.requests()[0];
    assert!(
        token_request
            .header("authorization")
            .is_some_and(|h| h.starts_with("Basic ")),
        "the client authenticates with a Basic header by default"
    );
    let stored = cached(&api).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].access, "access-token-1");
    assert_eq!(stored[0].refresh.as_deref(), Some("refresh-1"));

    // History keeps no token, refresh token or client secret.
    let id = executed.record.unwrap();
    let payload: String = api
        .store
        .call(move |db| {
            Ok(db.query_row(
                "SELECT payload FROM executions WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    for secret in ["access-token-1", "refresh-1", SECRET] {
        assert!(!payload.contains(secret), "{secret} in the History payload");
    }
    assert_eq!(
        executed.sent.template.auth,
        RequestAuth::bearer("access-token-1"),
        "the live request as sent shows the Bearer token"
    );
}

async fn expire(api: &Api, refresh: Option<&str>) {
    let mut token = cached(api).await.remove(0);
    token.expires_at = Some(0);
    token.refresh = refresh.map(str::to_owned);
    api.store
        .call(move |db| oauth::put(db, None, &token))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_a_failed_refresh_asks_anew() {
    let api = api(SECRET).await;
    send(&api).await.unwrap();

    expire(&api, Some("refresh-1")).await;
    answer(&api, token_answer("access-2", r#", "expires_in": 3600"#));
    send(&api).await.unwrap();

    expire(&api, Some("refresh-2")).await;
    answer(
        &api,
        response(
            "400 Bad Request",
            &[("Content-Type", "application/json")],
            br#"{"error": "invalid_grant"}"#,
        ),
    );
    answer(&api, token_answer("access-3", ""));
    send(&api).await.unwrap();

    assert_eq!(
        received(&api)[2..],
        [
            "/token grant_type=refresh_token&refresh_token=refresh-1&scope=read".to_owned(),
            "/data Bearer access-2".to_owned(),
            "/token grant_type=refresh_token&refresh_token=refresh-2&scope=read".to_owned(),
            "/token grant_type=client_credentials&scope=read".to_owned(),
            "/data Bearer access-3".to_owned(),
        ]
    );
    assert_eq!(cached(&api).await[0].access, "access-3");
}

#[tokio::test]
async fn a_refused_token_request_sends_nothing_and_records_nothing() {
    let api = api(SECRET).await;
    answer(
        &api,
        response(
            "401 Unauthorized",
            &[("Content-Type", "application/json")],
            br#"{"error": "invalid_client", "error_description": "Unknown client"}"#,
        ),
    );

    let error = send(&api).await.unwrap_err();

    assert_eq!(
        error.to_string(),
        "Token request failed: invalid_client: Unknown client"
    );
    assert_eq!(received(&api).len(), 1, "only the token request went out");
    let history = api.store.call(|db| executions::count(db)).await.unwrap();
    assert_eq!(history, 0);
    assert_eq!(cached(&api).await, []);
}

#[tokio::test]
async fn a_missing_secret_stops_the_send_before_the_token_request() {
    let api = api("{{secret:client_secret}}").await;

    let error = send(&api).await.unwrap_err();

    assert!(
        matches!(&error, ExecuteError::Unresolved(names) if names == &["secret:client_secret"]),
        "{error:?}"
    );
    assert_eq!(received(&api), [] as [String; 0]);
}

#[tokio::test]
async fn the_command_line_fetches_its_own_token_and_keeps_none() {
    let api = api(SECRET).await;
    send(&api).await.unwrap();
    let before = cached(&api).await;

    let selected = profile::select(
        &api.store.handle(),
        &RequestPath::new(&["API"], "data"),
        None,
        &Variables::new(),
        Purpose::Run,
    )
    .await
    .unwrap();
    let ran =
        imposter_app::executor::run(&selected.inputs, selected.request, CancellationToken::new())
            .await
            .unwrap();

    assert_eq!(
        received(&api)[2..],
        [
            "/token grant_type=client_credentials&scope=read".to_owned(),
            "/data Bearer a-default".to_owned(),
        ],
        "the profile's token is not borrowed"
    );
    assert!(ran.secrets.iter().any(|s| s == "a-default"));
    assert_eq!(cached(&api).await, before, "and the run's is not kept");
}

#[tokio::test]
async fn client_credentials_can_go_in_the_form() {
    let api = api(SECRET).await;
    let mut request = api.request.clone();
    if let Some(oauth) = &mut request.template.auth.oauth2 {
        oauth.client_auth = ClientAuth::Body;
    }
    execute(&api.store.handle(), request, CancellationToken::new())
        .await
        .unwrap();
    let token_request = &api.server.requests()[0];
    assert_eq!(token_request.header("authorization"), None);
    assert_eq!(
        String::from_utf8_lossy(&token_request.body),
        format!("grant_type=client_credentials&scope=read&client_id=app&client_secret={SECRET}")
    );
}

#[tokio::test]
async fn copy_as_curl_uses_the_cached_token_and_never_fetches_one() {
    let api = api(SECRET).await;
    let error = imposter_app::executor::curl_command(&api.store.handle(), &api.request)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "OAuth 2.0 needs a token: send the request, or get one in the Auth tab"
    );
    assert_eq!(received(&api), [] as [String; 0]);

    send(&api).await.unwrap();
    let curl = imposter_app::executor::curl_command(&api.store.handle(), &api.request)
        .await
        .unwrap();
    assert!(curl.contains("Bearer a-default"), "{curl}");
    assert_eq!(received(&api).len(), 2, "copying sent nothing");
}
