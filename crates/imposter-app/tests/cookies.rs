//! The cookie jar end to end: Send keeps what a login sets and sends it on, per
//! environment; the settings turn it off or stop it sending; runs and the command line use
//! jars of their own and write nothing back. Checked in the bytes a loopback server
//! receives and in the stored jar.
#![cfg(test)]

use imposter_app::document::RequestPath;
use imposter_app::executor::execute;
use imposter_app::profile::{self, Purpose};
use imposter_app::runner::{self, Options};
use imposter_core::ids::EnvironmentId;
use imposter_core::model::{Options as RequestOptions, Request, RequestTemplate, Variables};
use imposter_core::settings::AppSettings;
use imposter_store::{Store, collections, environments, kv};
use imposter_testkit::{Received, Server, response};
use tokio_util::sync::CancellationToken;

struct Api {
    _dir: tempfile::TempDir,
    store: Store,
    server: Server,
    login: Request,
    me: Request,
    dev: EnvironmentId,
    prod: EnvironmentId,
}

/// `/login` sets a session cookie named for the environment it was sent with (from the
/// `who` header); `/me` answers either way.
async fn api() -> Api {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let server = Server::start(|_, received: &Received| {
        if received.target() == "/login" {
            let cookie = format!("session={}; Path=/", received.header("who").unwrap_or("-"));
            response("200 OK", &[("Set-Cookie", &cookie)], b"in")
        } else {
            response("200 OK", &[], b"me")
        }
    })
    .await;
    let base = server.url.clone();
    let (login, me, dev, prod) = store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("who", "none")]))?;
            let dev = environments::create_environment(db, "Dev", &vars(&[("who", "dev")]))?.id;
            let prod = environments::create_environment(db, "Prod", &vars(&[("who", "prod")]))?.id;
            environments::set_active_environment(db, Some(dev))?;
            let collection =
                collections::create_collection(db, "API", "", &vars(&[("base", &base)]))?;
            let request = |name: &str, path: &str| RequestTemplate {
                name: name.into(),
                method: "GET".into(),
                url: format!("{{{{base}}}}{path}"),
                headers: vec![imposter_core::model::ParamItem::new("who", "{{who}}")],
                ..RequestTemplate::default()
            };
            let login =
                collections::create_request(db, collection.id, None, &request("login", "/login"))?;
            let me = collections::create_request(db, collection.id, None, &request("me", "/me"))?;
            Ok((login, me, dev, prod))
        })
        .await
        .unwrap();
    Api {
        _dir: dir,
        store,
        server,
        login,
        me,
        dev,
        prod,
    }
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

async fn send(api: &Api, request: &Request) -> imposter_app::executor::Executed {
    execute(
        &api.store.handle(),
        request.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap()
}

async fn activate(api: &Api, environment: Option<EnvironmentId>) {
    api.store
        .call(move |db| environments::set_active_environment(db, environment))
        .await
        .unwrap();
}

async fn kept(api: &Api, environment: Option<EnvironmentId>) -> Vec<String> {
    imposter_app::cookies::list(&api.store.handle(), environment)
        .await
        .unwrap()
        .into_iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect()
}

fn last_cookie(api: &Api) -> Option<String> {
    api.server
        .requests()
        .last()
        .and_then(|r| r.header("cookie").map(str::to_owned))
}

#[tokio::test]
async fn a_login_cookie_is_kept_per_environment_and_sent_on() {
    let api = api().await;

    send(&api, &api.login).await;
    let me = send(&api, &api.me).await;
    assert_eq!(last_cookie(&api).as_deref(), Some("session=dev"));
    assert_eq!(
        me.sent.template.cookies,
        [imposter_core::model::ParamItem::new("session", "dev")],
        "the request as sent shows the jar's cookie"
    );
    assert_eq!(kept(&api, Some(api.dev)).await, ["session=dev"]);

    activate(&api, Some(api.prod)).await;
    send(&api, &api.me).await;
    assert_eq!(last_cookie(&api), None, "Prod has a jar of its own");
    send(&api, &api.login).await;
    assert_eq!(kept(&api, Some(api.prod)).await, ["session=prod"]);
    assert_eq!(kept(&api, Some(api.dev)).await, ["session=dev"]);

    activate(&api, None).await;
    send(&api, &api.me).await;
    assert_eq!(
        last_cookie(&api),
        None,
        "and so do requests without an environment"
    );
}

#[tokio::test]
async fn the_settings_turn_the_jar_off_or_stop_it_sending() {
    let api = api().await;
    let settings = |settings: AppSettings| {
        api.store
            .call(move |db| kv::set_app_settings(db, &settings))
    };

    settings(AppSettings {
        cookie_jar: false,
        ..AppSettings::default()
    })
    .await
    .unwrap();
    send(&api, &api.login).await;
    assert_eq!(kept(&api, Some(api.dev)).await, [] as [String; 0]);

    // A request can keep cookies while the application does not.
    let mut login = api.login.clone();
    login.template.options = RequestOptions {
        cookie_jar: Some(true),
        ..RequestOptions::default()
    };
    send(&api, &login).await;
    assert_eq!(kept(&api, Some(api.dev)).await, ["session=dev"]);

    // Without attaching cookies the jar still keeps what arrives, and sends nothing.
    settings(AppSettings {
        attach_cookies: false,
        ..AppSettings::default()
    })
    .await
    .unwrap();
    send(&api, &api.me).await;
    assert_eq!(last_cookie(&api), None);
}

/// Run the whole collection in the app with `environment`; the `Cookie` header each
/// request of the run arrived with.
async fn run_in(api: &Api, environment: EnvironmentId) -> Vec<Option<String>> {
    let collection = api.login.collection_id.unwrap();
    let selected = profile::select_under(&api.store.handle(), collection, None, Some(environment))
        .await
        .unwrap();
    let names: Vec<String> = selected
        .iter()
        .map(|s| s.request.template.name.clone())
        .collect();
    assert_eq!(names, ["login", "me"]);
    let before = api.server.requests().len();
    runner::run(
        selected,
        Options::default(),
        CancellationToken::new(),
        |_| {},
    )
    .await;
    api.server.requests()[before..]
        .iter()
        .map(|r| r.header("cookie").map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn runs_share_a_jar_of_their_own_and_write_nothing_back() {
    let api = api().await;
    send(&api, &api.login).await;

    assert_eq!(
        run_in(&api, api.dev).await,
        [
            Some("session=dev".to_owned()),
            Some("session=dev".to_owned())
        ],
        "the app's run starts from a copy of the stored jar"
    );
    assert_eq!(
        run_in(&api, api.prod).await,
        [None, Some("session=prod".to_owned())],
        "the run's login reaches the request after it"
    );
    assert_eq!(
        kept(&api, Some(api.prod)).await,
        [] as [String; 0],
        "and nothing goes back"
    );

    // The command line starts empty, even where the profile has cookies.
    let me = profile::select(
        &api.store.handle(),
        &RequestPath::new(&["API"], "me"),
        Some("Dev"),
        &Variables::new(),
        Purpose::Run,
    )
    .await
    .unwrap();
    imposter_app::executor::run(&me.inputs, me.request, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(last_cookie(&api), None);
    assert_eq!(kept(&api, Some(api.dev)).await, ["session=dev"]);
}
