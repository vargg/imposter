//! Running a request saved in the personal profile from outside the app: chosen by its
//! breadcrumb, with the named environment only, production refused unless the app allows
//! it, captures kept in the run, and the run recorded in History as the command line's.
#![cfg(test)]

use imposter_app::document::{DocumentError, RequestPath};
use imposter_app::executor::{Writes, record, run};
use imposter_app::paths::{OpenError, open_existing_profile};
use imposter_app::profile::{ProfileError, request_paths, select};
use imposter_core::capture::{Capture, Taken};
use imposter_core::execution::Via;
use imposter_core::model::{EnvColor, ParamItem, RequestTemplate, Variables};
use imposter_core::select::Selector;
use imposter_store::{Store, collections, environments, executions, kv};
use imposter_testkit::{Server, response};
use tokio_util::sync::CancellationToken;

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// "Shop API / Users / List users" with a capture, globals, an active Dev environment and a
/// red Prod one.
async fn profile(base: &str) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let base = base.to_owned();
    store
        .call(move |db| {
            environments::set_globals(db, &vars(&[("scope", "global"), ("base", &base)]))?;
            let dev = environments::create_environment(db, "Dev", &vars(&[("scope", "dev")]))?;
            environments::set_active_environment(db, Some(dev.id))?;
            let prod = environments::create_environment(db, "Prod", &vars(&[("scope", "prod")]))?;
            environments::set_environment_color(db, prod.id, Some(EnvColor::Red))?;
            let shop = collections::create_collection(db, "Shop API", "", &Variables::new())?;
            let users = collections::create_folder(db, shop.id, None, "Users", "", 0)?;
            collections::create_request(
                db,
                shop.id,
                Some(users.id),
                &RequestTemplate {
                    name: "List users".into(),
                    method: "GET".into(),
                    url: "{{base}}/users".into(),
                    headers: vec![ParamItem::new("X-Scope", "{{scope}}")],
                    captures: vec![Capture::new("token", Selector::Json("$.token".into()))],
                    ..RequestTemplate::default()
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    (dir, store)
}

fn path(text: &str) -> RequestPath {
    RequestPath::parse(text).unwrap()
}

#[tokio::test]
async fn the_breadcrumb_names_a_request_and_only_the_named_environment_applies() {
    let server = Server::start(|_, _| response("200 OK", &[], br#"{"token": "t-9"}"#)).await;
    let (_dir, store) = profile(&server.url).await;
    let none = Variables::new();
    for (environment, values, expected) in [
        (None, none.clone(), "global"),
        (Some("Dev"), none.clone(), "dev"),
        (Some("Dev"), vars(&[("scope", "input")]), "input"),
    ] {
        let selected = select(
            &store.handle(),
            &path("Shop API / Users / List users"),
            environment,
            &values,
        )
        .await
        .unwrap();
        assert_eq!(selected.inputs.writes, Writes::Run);
        let ran = run(&selected.inputs, selected.request, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            server.requests().pop().unwrap().header("x-scope"),
            Some(expected),
            "{environment:?}: the app's active Dev applies only when named"
        );
        assert_eq!(
            ran.record.response.captures[0].taken,
            Taken::Value("t-9".into())
        );
        assert!(
            ran.writes.iter().all(|w| w.scope.is_none()),
            "kept in the run"
        );
    }
    assert_eq!(
        request_paths(&store.handle()).await.unwrap(),
        ["Shop API/Users/List users"]
    );
}

#[tokio::test]
async fn wrong_paths_and_environments_say_what_exists() {
    let (_dir, store) = profile("http://127.0.0.1:1").await;
    let none = Variables::new();
    let missing = select(&store.handle(), &path("Users/List users"), None, &none)
        .await
        .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "No request at Users/List users. Requests: Shop API/Users/List users"
    );
    let unknown = select(
        &store.handle(),
        &path("Shop API/Users/List users"),
        Some("Staging"),
        &none,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        unknown,
        ProfileError::Path(DocumentError::NoEnvironment { ref available, .. }) if available == &["Dev", "Prod"]
    ));
    store
        .call(|db| {
            let twin = collections::create_collection(db, "Shop API", "", &Variables::new())?;
            let users = collections::create_folder(db, twin.id, None, "Users", "", 0)?;
            collections::create_request(
                db,
                twin.id,
                Some(users.id),
                &RequestTemplate {
                    name: "List users".into(),
                    ..RequestTemplate::default()
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(matches!(
        select(
            &store.handle(),
            &path("Shop API/Users/List users"),
            None,
            &none
        )
        .await,
        Err(ProfileError::Path(DocumentError::Ambiguous {
            count: 2,
            ..
        }))
    ));
}

#[tokio::test]
async fn production_needs_the_apps_setting() {
    let server = Server::start(|_, _| response("200 OK", &[], b"")).await;
    let (_dir, store) = profile(&server.url).await;
    let refused = select(
        &store.handle(),
        &path("Shop API/Users/List users"),
        Some("Prod"),
        &Variables::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(refused, ProfileError::Production { ref name } if name == "Prod"));
    assert!(refused.to_string().contains("Settings → Command line"));
    store
        .call(|db| {
            let mut settings = kv::app_settings(db)?;
            settings.allow_production_from_cli = true;
            kv::set_app_settings(db, &settings)
        })
        .await
        .unwrap();
    let selected = select(
        &store.handle(),
        &path("Shop API/Users/List users"),
        Some("Prod"),
        &Variables::new(),
    )
    .await
    .unwrap();
    run(&selected.inputs, selected.request, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(server.requests()[0].header("x-scope"), Some("prod"));
}

#[tokio::test]
async fn a_recorded_run_is_the_command_lines_and_changes_no_variable() {
    let server = Server::start(|_, _| response("200 OK", &[], br#"{"token": "t-9"}"#)).await;
    let (_dir, store) = profile(&server.url).await;
    let selected = select(
        &store.handle(),
        &path("Shop API/Users/List users"),
        Some("Dev"),
        &Variables::new(),
    )
    .await
    .unwrap();
    let mut ran = run(&selected.inputs, selected.request, CancellationToken::new())
        .await
        .unwrap();
    ran.record.via = Via::CommandLine;
    let executed = record(&store.handle(), ran).await;
    assert!(executed.record.is_some());
    let (summary, dev) = store
        .call(|db| {
            let summary = executions::page(db, None, 0, 1)?.remove(0);
            let dev = environments::environments(db)?
                .into_iter()
                .find(|e| e.name == "Dev")
                .map(|e| e.variables);
            Ok((summary, dev))
        })
        .await
        .unwrap();
    assert_eq!(summary.via, Via::CommandLine);
    assert_eq!(summary.environment, "Dev");
    assert_eq!(dev, Some(vars(&[("scope", "dev")])), "no token written");
}

#[test]
fn only_an_existing_profile_opens() {
    let home = tempfile::tempdir().unwrap();
    assert!(matches!(
        open_existing_profile(None, Some(home.path().to_owned())),
        Err(OpenError::NoProfile { .. })
    ));
    assert_eq!(
        std::fs::read_dir(home.path()).unwrap().count(),
        0,
        "nothing was created"
    );
    let explicit = home.path().join("data");
    assert!(matches!(
        open_existing_profile(Some(explicit.clone().into()), None),
        Err(OpenError::NoProfile { .. })
    ));
    assert!(matches!(
        open_existing_profile(Some("".into()), None),
        Err(OpenError::EmptyDirectory)
    ));
    std::fs::create_dir(&explicit).unwrap();
    Store::open(&explicit.join("imposter.db"))
        .unwrap()
        .close()
        .unwrap();
    assert!(open_existing_profile(Some(explicit.into()), None).is_ok());
}
