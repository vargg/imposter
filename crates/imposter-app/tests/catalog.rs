//! What the command line shows without running anything: listings, one request's
//! definition and the environments, from the profile and from a file alike, with
//! placeholders as stored and never a variable's value or a literal credential.
#![cfg(test)]

use imposter_app::catalog::{
    AuthView, environments_file, environments_profile, list_file, list_profile, show_file,
    show_profile,
};
use imposter_app::document::RequestPath;
use imposter_app::transfer::{import, read_preview};
use imposter_store::Store;
use imposter_transfer::native::{self, NativeDocument};
use serde_json::json;

const SECRET: &str = "tok-SECRET-42";

/// A collection whose environment and globals hold a secret value, with one request that
/// uses placeholders and one that has a literal token and a literal Authorization header.
fn document() -> NativeDocument {
    native::decode_value(json!({
        "format": "imposter",
        "version": 3,
        "collection": {
            "name": "Shop API",
            "variables": {"base": "http://127.0.0.1:1"},
            "scripts": {"before": "request.set_header(\"X-Collection\", \"1\");"},
            "folders": [{
                "name": "Users",
                "requests": [
                    {
                        "name": "Create user", "method": "POST", "url": "{{base}}/users?trace={{$uuid}}",
                        "headers": [
                            {"name": "Authorization", "value": "Bearer {{token}}", "is_enabled": true},
                            {"name": "X-Client", "value": "{{client}}", "is_enabled": false}
                        ],
                        "body": base64(r#"{"name": "{{name}}"}"#), "body_type": "json",
                        "checks": [{"selector": "status", "op": "equals", "expected": "201"}],
                        "captures": [{"variable": "id", "selector": {"json": "$.id"}}],
                        "options": {"timeout": 5.0}
                    },
                    {
                        "name": "Literal", "method": "GET", "url": "{{base}}/me",
                        "auth": {"type": "bearer_token", "bearer_token": {"token": SECRET}},
                        "headers": [{"name": "authorization", "value": format!("Bearer {SECRET}"), "is_enabled": true}],
                        "cookies": [{"name": "sid", "value": SECRET, "is_enabled": true}],
                        "body": "", "body_type": "none"
                    }
                ]
            }],
            "requests": [{"name": "Ping", "method": "GET", "url": "{{base}}/ping", "body": "", "body_type": "none"}]
        },
        "environments": [
            {"name": "Dev", "variables": {"token": SECRET, "name": "dev-user"}},
            {"name": "Prod", "variables": {"token": SECRET}, "color": "red"}
        ],
        "globals": {"client": SECRET, "token": SECRET}
    }))
    .unwrap()
}

fn base64(text: &str) -> String {
    imposter_core::base64_bytes::encode(text.as_bytes())
}

async fn profile_of(document: &NativeDocument) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("api.imposter.json");
    std::fs::write(&file, native::encode(document)).unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    let preview = read_preview(&store.handle(), file, Vec::new())
        .await
        .unwrap();
    import(&store.handle(), preview, false).await.unwrap();
    (dir, store)
}

fn path(text: &str) -> RequestPath {
    RequestPath::parse(text).unwrap()
}

#[tokio::test]
async fn the_profile_and_the_file_list_the_same_requests() {
    let document = document();
    let (_dir, store) = profile_of(&document).await;
    let from_file = list_file(&document);
    let from_profile = list_profile(&store.handle()).await.unwrap();
    assert_eq!(from_file.requests, from_profile.requests);
    let paths: Vec<&str> = from_file.requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "Shop API/Ping",
            "Shop API/Users/Create user",
            "Shop API/Users/Literal"
        ]
    );
    let create = &from_file.requests[1];
    assert_eq!(
        (
            create.method.as_str(),
            create.checks,
            create.captures,
            create.scripts
        ),
        ("POST", 1, 1, true),
        "the collection's script runs around it"
    );

    let filtered = |filter: &str| -> Vec<String> {
        from_file
            .clone()
            .filtered(filter)
            .requests
            .into_iter()
            .map(|r| r.path)
            .collect()
    };
    assert_eq!(filtered("method:POST"), ["Shop API/Users/Create user"]);
    assert_eq!(filtered("users -literal"), ["Shop API/Users/Create user"]);
    assert_eq!(
        filtered("shop").len(),
        3,
        "the collection's name is matched"
    );
    assert_eq!(filtered("url:ping"), ["Shop API/Ping"]);
}

#[tokio::test]
async fn a_request_shows_its_placeholders_and_where_each_comes_from() {
    let document = document();
    let (_dir, store) = profile_of(&document).await;
    let request = path("Shop API / Users / Create user");
    let profile_dev = show_profile(&store.handle(), &request, Some("Dev"), 64 * 1024)
        .await
        .unwrap();
    let file_dev = show_file(
        &document,
        &path("Users/Create user"),
        Some("Dev"),
        64 * 1024,
    )
    .unwrap();
    let mut file_as_profile = file_dev.clone();
    file_as_profile.source = profile_dev.source;
    assert_eq!(
        file_as_profile, profile_dev,
        "both sources show the same request"
    );
    assert_eq!(file_dev.path, "Shop API/Users/Create user");

    assert_eq!(profile_dev.url, "{{base}}/users?trace={{$uuid}}");
    assert_eq!(profile_dev.headers[0].value, "Bearer {{token}}");
    assert!(!profile_dev.headers[1].enabled);
    assert_eq!(
        profile_dev.body.text.as_deref(),
        Some(r#"{"name": "{{name}}"}"#)
    );
    assert_eq!(profile_dev.checks[0].describe, "status = 201");
    assert_eq!(profile_dev.options["timeout"], json!(5.0));
    assert_eq!(
        profile_dev.inherited_scripts[0].level,
        "Collection \"Shop API\""
    );
    let sources: Vec<(String, String)> = profile_dev
        .variables
        .iter()
        .map(|v| (v.name.clone(), v.defined_in.clone()))
        .collect();
    assert_eq!(
        sources,
        [
            ("$uuid".into(), "generated".into()),
            ("base".into(), "Shop API".into()),
            ("client".into(), "Global".into()),
            ("name".into(), "Dev".into()),
            ("token".into(), "Dev".into()),
        ]
    );
    assert_eq!(profile_dev.variables[4].overrides, ["Global"]);

    let without = show_profile(&store.handle(), &request, None, 64 * 1024)
        .await
        .unwrap();
    let name = without.variables.iter().find(|v| v.name == "name").unwrap();
    assert_eq!(
        name.defined_in, "undefined",
        "only the named environment applies"
    );
    let prod = show_profile(&store.handle(), &request, Some("Prod"), 4)
        .await
        .unwrap();
    assert!(prod.body.cut, "a short --max-body cuts the body");
    assert_eq!(
        prod.environment.as_deref(),
        Some("Prod"),
        "showing production reveals only names"
    );
}

#[tokio::test]
async fn no_value_or_literal_credential_is_ever_shown() {
    let document = document();
    let (_dir, store) = profile_of(&document).await;
    let literal = show_profile(
        &store.handle(),
        &path("Shop API/Users/Literal"),
        Some("Dev"),
        1024,
    )
    .await
    .unwrap();
    assert_eq!(
        literal.auth,
        AuthView::Bearer {
            token: "«redacted»".into()
        }
    );
    assert_eq!(literal.headers[0].value, "«redacted»");
    assert_eq!(literal.cookies[0].value, "«redacted»");

    let outputs = [
        serde_json::to_string(&literal).unwrap(),
        serde_json::to_string(
            &show_profile(
                &store.handle(),
                &path("Shop API/Users/Create user"),
                Some("Dev"),
                1024,
            )
            .await
            .unwrap(),
        )
        .unwrap(),
        serde_json::to_string(&list_profile(&store.handle()).await.unwrap()).unwrap(),
        serde_json::to_string(&environments_profile(&store.handle()).await.unwrap()).unwrap(),
        serde_json::to_string(&environments_file(&document)).unwrap(),
    ];
    for output in outputs {
        assert!(!output.contains(SECRET), "{output}");
        assert!(!output.contains("dev-user"), "{output}");
    }
}

#[tokio::test]
async fn environments_list_names_colours_and_variable_names() {
    let document = document();
    let (_dir, store) = profile_of(&document).await;
    let from_profile = environments_profile(&store.handle()).await.unwrap();
    let mut from_file = environments_file(&document);
    from_file.source = from_profile.source;
    assert_eq!(from_file, from_profile);
    let names: Vec<(&str, Option<imposter_core::model::EnvColor>)> = from_profile
        .environments
        .iter()
        .map(|e| (e.name.as_str(), e.color))
        .collect();
    assert_eq!(
        names,
        [
            ("Dev", None),
            ("Prod", Some(imposter_core::model::EnvColor::Red))
        ]
    );
    assert_eq!(from_profile.environments[0].variables, ["token", "name"]);
    assert_eq!(from_profile.globals, ["client", "token"]);
    assert_eq!(from_profile.collections[0].variables, ["base"]);
}
