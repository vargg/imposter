//! Secrets end to end: `{{secret:name}}` reaches the wire from the active environment's
//! secrets before the global ones, a capture stores a secret and never a variable, a run
//! carries a captured secret forward, and neither History nor an export keeps a value.
#![cfg(test)]

use imposter_app::executor::{execute, write_captures};
use imposter_app::runner::{self, Options};
use imposter_app::transfer::{ExportScope, export_native, postman_export};
use imposter_core::capture::Capture;
use imposter_core::check::{Check, Op};
use imposter_core::execution::Via;
use imposter_core::model::{BodyType, ParamItem, Request, RequestTemplate, Variables};
use imposter_core::select::Selector;
use imposter_core::variables::{Scope, SecretScope};
use imposter_store::{Store, collections, environments, executions, scopes, secrets};
use imposter_testkit::{Received, Server, response};
use tokio_util::sync::CancellationToken;

const DEV_TOKEN: &str = "dev-token-1234";
const GLOBAL_TOKEN: &str = "global-token-1234";
const ISSUED: &str = "issued-token-5678";

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    let json = [("Content-Type", "application/json")];
    match received.target().split('?').next().unwrap_or_default() {
        "/login" => response(
            "200 OK",
            &json,
            format!(r#"{{"token": "{ISSUED}"}}"#).as_bytes(),
        ),
        _ => response("200 OK", &json, br#"{"ok": true}"#),
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    server: Server,
    dev: imposter_core::ids::EnvironmentId,
    collection: imposter_core::ids::CollectionId,
}

/// Dev (active) has a `token` secret, the globals another, and the collection a `base`.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let server = Server::start(reply).await;
    let base = server.url.clone();
    let (dev, collection) = store
        .call(move |db| {
            let dev = environments::create_environment(db, "Dev", &vars(&[]))?.id;
            environments::set_active_environment(db, Some(dev))?;
            secrets::write(
                db,
                SecretScope::Environment(dev),
                &vars(&[("token", DEV_TOKEN)]),
            )?;
            secrets::write(db, SecretScope::Global, &vars(&[("token", GLOBAL_TOKEN)]))?;
            let collection =
                collections::create_collection(db, "API", "", &vars(&[("base", &base)]))?.id;
            Ok((dev, collection))
        })
        .await
        .unwrap();
    Fixture {
        dir,
        store,
        server,
        dev,
        collection,
    }
}

async fn save(f: &Fixture, template: RequestTemplate) -> Request {
    let collection = f.collection;
    f.store
        .call(move |db| collections::create_request(db, collection, None, &template))
        .await
        .unwrap()
}

fn uses_the_token() -> RequestTemplate {
    RequestTemplate {
        name: "Items".into(),
        method: "POST".into(),
        url: "{{base}}/items?key={{secret:token}}".into(),
        headers: vec![ParamItem::new("Authorization", "Bearer {{secret:token}}")],
        body_type: BodyType::Json,
        body: br#"{"token": "{{secret:token}}"}"#.to_vec(),
        checks: vec![Check::new(
            Selector::Json("$.ok".into()),
            Op::NotEquals,
            "{{secret:token}}",
        )],
        ..RequestTemplate::default()
    }
}

fn login() -> RequestTemplate {
    RequestTemplate {
        name: "Login".into(),
        method: "POST".into(),
        url: "{{base}}/login".into(),
        body_type: BodyType::None,
        captures: vec![Capture {
            secret: true,
            ..Capture::new("issued", Selector::Json("$.token".into()))
        }],
        ..RequestTemplate::default()
    }
}

#[tokio::test]
async fn a_secret_reaches_the_wire_but_not_history() {
    let f = fixture().await;
    let request = save(&f, uses_the_token()).await;
    let executed = execute(&f.store.handle(), request, CancellationToken::new())
        .await
        .unwrap();
    let received = f.server.requests().remove(0);
    assert_eq!(
        received.target(),
        format!("/items?key={DEV_TOKEN}"),
        "Dev beats Global"
    );
    assert_eq!(
        received.header("authorization"),
        Some(format!("Bearer {DEV_TOKEN}").as_str())
    );
    assert!(String::from_utf8_lossy(&received.body).contains(DEV_TOKEN));
    assert!(
        String::from_utf8_lossy(&executed.sent.template.body).contains(DEV_TOKEN),
        "the live request as sent keeps its values"
    );

    let id = executed.record.unwrap();
    let (payload, summary) = f
        .store
        .call(move |db| {
            let payload: String = db.query_row(
                "SELECT payload FROM executions WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )?;
            Ok((payload, executions::page(db, None, 0, 1)?.remove(0)))
        })
        .await
        .unwrap();
    for secret in [DEV_TOKEN, GLOBAL_TOKEN] {
        assert!(!payload.contains(secret), "{secret} in the History payload");
        assert!(!summary.url.contains(secret));
    }
    let record = f
        .store
        .call(move |db| executions::execution(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.resolved.template.url,
        format!("{}/items?key=«redacted»", f.server.url)
    );
    assert_eq!(record.response.checks[0].check.expected, "«redacted»");
}

#[tokio::test]
async fn a_secret_capture_is_stored_as_a_secret_and_carried_through_a_run() {
    let f = fixture().await;
    let login = save(&f, login()).await;
    let executed = execute(&f.store.handle(), login, CancellationToken::new())
        .await
        .unwrap();
    assert!(executed.writes.iter().all(|w| w.secret));
    write_captures(&f.store.handle(), executed.writes.clone())
        .await
        .unwrap();
    let dev = f.dev;
    let (dev_secrets, dev_variables, payload) = f
        .store
        .call(move |db| {
            Ok((
                secrets::read(db, SecretScope::Environment(dev))?,
                scopes::read(db, Scope::Environment(dev))?.0,
                db.query_row("SELECT payload FROM executions", [], |row| {
                    row.get::<_, String>(0)
                })?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(dev_secrets.get("issued").map(String::as_str), Some(ISSUED));
    assert!(dev_variables.is_empty(), "no variable was set");
    assert!(
        !payload.contains(ISSUED),
        "the response body that held it is redacted"
    );

    // In a run the captured secret reaches the next request and is stored nowhere.
    save(
        &f,
        RequestTemplate {
            name: "Use".into(),
            url: "{{base}}/use?t={{secret:issued}}".into(),
            body_type: BodyType::None,
            ..RequestTemplate::default()
        },
    )
    .await;
    f.store
        .call(move |db| secrets::write(db, SecretScope::Environment(dev), &Variables::new()))
        .await
        .unwrap();
    let selected =
        imposter_app::profile::select_under(&f.store.handle(), f.collection, None, Some(dev))
            .await
            .unwrap();
    assert_eq!(selected.len(), 2, "Login, then Use");
    let steps = runner::run(
        selected,
        Options {
            keep_going: false,
            record: Some((f.store.handle(), Via::App)),
        },
        CancellationToken::new(),
        |_| {},
    )
    .await;
    assert_eq!(steps.len(), 2);
    let targets: Vec<String> = f
        .server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets.last().unwrap(), &format!("/use?t={ISSUED}"));
    let (stored, payloads) = f
        .store
        .call(move |db| {
            let mut statement = db.prepare("SELECT payload FROM executions")?;
            let payloads = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((secrets::read(db, SecretScope::Environment(dev))?, payloads))
        })
        .await
        .unwrap();
    assert!(stored.is_empty(), "a run writes nothing back");
    assert!(payloads.iter().all(|p| !p.contains(ISSUED)));
}

#[tokio::test]
async fn exports_never_carry_a_secret_value() {
    let f = fixture().await;
    save(&f, uses_the_token()).await;
    save(&f, login()).await;
    let out = f.dir.path().join("out.imposter.json");
    let scope = ExportScope {
        collection: Some(f.collection),
        environments: vec![f.dev],
        globals: true,
        draft: None,
    };
    export_native(&f.store.handle(), scope.clone(), out.clone())
        .await
        .unwrap();
    let native = std::fs::read_to_string(&out).unwrap();
    assert!(native.contains("{{secret:token}}"), "references stay");
    assert!(
        native.contains("\"version\": 4"),
        "a secret capture needs version 4"
    );
    let (_, postman, _) = postman_export(&f.store.handle(), scope).await.unwrap();
    let postman = postman.to_string();
    for secret in [DEV_TOKEN, GLOBAL_TOKEN] {
        assert!(!native.contains(secret));
        assert!(!postman.contains(secret));
    }
}
