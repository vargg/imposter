//! `imposter list`, `show` and `envs` in process: JSON and text, from the profile and a
//! file, a filter, wrong paths, and never a value or a literal credential on stdout or
//! stderr.
#![cfg(test)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use imposter_app::core::model::{EnvColor, RequestTemplate, Variables};
use imposter_app::store::{Store, collections, environments};
use imposter_cli::{Io, main};
use serde_json::{Value, json};

const SECRET: &str = "tok-SECRET-42";

struct Ran {
    status: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], data: &Path) -> Ran {
    let env: HashMap<String, String> = [(
        "IMPOSTER_DATA_DIR".to_owned(),
        data.to_str().unwrap().to_owned(),
    )]
    .into();
    let var = move |name: &str| env.get(name).cloned();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let status = main(
        args.iter().map(OsString::from).collect(),
        &mut Io {
            stdout: &mut stdout,
            stderr: &mut stderr,
            var: &var,
        },
    );
    let ran = Ran {
        status,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    };
    for text in [&ran.stdout, &ran.stderr] {
        assert!(!text.contains(SECRET), "{args:?}: {text}");
    }
    ran
}

fn json_of(ran: &Ran) -> Value {
    serde_json::from_str(&ran.stdout).unwrap_or_else(|e| panic!("{e}: {}", ran.stdout))
}

/// "Shop API" with a request using placeholders and one with a literal token; environments
/// and globals hold the secret as a value.
fn profile(dir: &Path) {
    let store = Store::open(&dir.join("imposter.db")).unwrap();
    store
        .call_blocking(|db| {
            let vars = |pairs: &[(&str, &str)]| -> Variables {
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect()
            };
            environments::set_globals(db, &vars(&[("token", SECRET)]))?;
            environments::create_environment(db, "Dev", &vars(&[("token", SECRET)]))?;
            let prod = environments::create_environment(db, "Prod", &vars(&[("token", SECRET)]))?;
            environments::set_environment_color(db, prod.id, Some(EnvColor::Red))?;
            let shop = collections::create_collection(
                db,
                "Shop API",
                "",
                &vars(&[("base", "http://127.0.0.1:1")]),
            )?;
            let users = collections::create_folder(db, shop.id, None, "Users", "", 0)?;
            for template in [
                json!({
                    "name": "Create user", "method": "POST", "url": "{{base}}/users",
                    "auth": {"type": "bearer_token", "bearer_token": {"token": "{{token}}"}},
                    "body": "", "body_type": "none",
                    "checks": [{"selector": "status", "op": "equals", "expected": "201"}],
                }),
                json!({
                    "name": "Literal", "method": "GET", "url": "{{base}}/me",
                    "auth": {"type": "bearer_token", "bearer_token": {"token": SECRET}},
                    "body": "", "body_type": "none",
                }),
            ] {
                let template: RequestTemplate = serde_json::from_value(template).unwrap();
                collections::create_request(db, shop.id, Some(users.id), &template)?;
            }
            Ok(())
        })
        .unwrap();
    store.close().unwrap();
}

#[test]
fn list_finds_saved_requests_and_filters_them() {
    let data = tempfile::tempdir().unwrap();
    profile(data.path());
    let all = run(&["list"], data.path());
    assert_eq!(all.status, 0, "{}", all.stderr);
    let listing = json_of(&all);
    assert_eq!(listing["source"], "profile");
    assert_eq!(listing["requests"][0]["path"], "Shop API/Users/Create user");
    assert_eq!(listing["requests"][0]["checks"], 1);

    let posts = json_of(&run(&["list", "method:POST"], data.path()));
    assert_eq!(posts["requests"].as_array().unwrap().len(), 1);
    let words = json_of(&run(&["list", "users", "-literal"], data.path()));
    assert_eq!(
        words["requests"].as_array().unwrap().len(),
        1,
        "several words read as one filter"
    );

    let text = run(&["list", "--text"], data.path());
    assert_eq!(
        text.stdout,
        "POST  Shop API/Users/Create user  {{base}}/users  (1 check)\n\
         GET   Shop API/Users/Literal      {{base}}/me\n"
    );
    let none = run(&["list", "nothing-like-this", "--text"], data.path());
    assert_eq!(none.stdout, "No saved requests match.\n");
}

#[test]
fn show_gives_placeholders_and_variable_sources_never_values() {
    let data = tempfile::tempdir().unwrap();
    profile(data.path());
    let shown = run(
        &["show", "Shop API / Users / Create user", "--env", "Dev"],
        data.path(),
    );
    assert_eq!(shown.status, 0, "{}", shown.stderr);
    let view = json_of(&shown);
    assert_eq!(
        view["auth"],
        json!({"type": "bearer", "token": "{{token}}"})
    );
    assert_eq!(
        view["variables"],
        json!([
            {"name": "base", "defined_in": "Shop API"},
            {"name": "token", "defined_in": "Dev", "overrides": ["Global"]}
        ])
    );
    let literal = json_of(&run(&["show", "Shop API/Users/Literal"], data.path()));
    assert_eq!(literal["auth"]["token"], "«redacted»");

    let prod = run(
        &[
            "show",
            "Shop API/Users/Create user",
            "--env",
            "Prod",
            "--text",
        ],
        data.path(),
    );
    assert_eq!(prod.status, 0, "inspecting production reveals only names");
    assert!(
        prod.stdout.starts_with("POST {{base}}/users\n"),
        "{}",
        prod.stdout
    );
    assert!(
        prod.stdout.contains("  token  Prod (overrides Global)"),
        "{}",
        prod.stdout
    );
    assert!(
        prod.stdout.contains("Checks:\n  status = 201\n"),
        "{}",
        prod.stdout
    );
}

#[test]
fn envs_list_scopes_by_variable_name() {
    let data = tempfile::tempdir().unwrap();
    profile(data.path());
    let envs = json_of(&run(&["envs"], data.path()));
    assert_eq!(
        envs["environments"][1],
        json!({"name": "Prod", "color": "red", "variables": ["token"]})
    );
    assert_eq!(envs["globals"], json!(["token"]));
    let text = run(&["envs", "--text"], data.path());
    assert_eq!(
        text.stdout,
        "Environments:\n  Dev: token\n  Prod (red: production): token\n\
         Globals:\n  Global: token\nCollections:\n  Shop API: base\n"
    );
}

#[test]
fn a_file_works_like_the_profile_and_errors_name_what_exists() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("api.imposter.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({
            "format": "imposter", "version": 2,
            "collection": {"name": "API", "requests": [
                {"name": "Ping", "method": "GET", "url": "{{base}}/ping", "body": "", "body_type": "none"}
            ]},
        }))
        .unwrap(),
    )
    .unwrap();
    let file = file.to_str().unwrap();
    let listing = json_of(&run(&["list", "--file", file], data.path()));
    assert_eq!(listing["source"], "file");
    assert_eq!(listing["requests"][0]["path"], "API/Ping");
    let view = json_of(&run(&["show", "Ping", "--file", file], data.path()));
    assert_eq!(view["path"], "API/Ping");
    assert_eq!(
        view["variables"],
        json!([{"name": "base", "defined_in": "undefined"}])
    );

    let wrong = run(&["show", "Nope", "--file", file], data.path());
    assert_eq!((wrong.status, wrong.stdout.as_str()), (3, ""));
    assert_eq!(wrong.stderr, "error: No request at Nope. Requests: Ping\n");
    let missing = run(&["envs"], data.path());
    assert_eq!(missing.status, 3, "no profile in this directory");
    assert!(
        missing.stderr.contains("open the app once"),
        "{}",
        missing.stderr
    );
    for (args, message) in [
        (&["show"][..], "show takes one REQUEST"),
        (&["envs", "extra"][..], "envs takes no arguments"),
        (&["list", "--env", "Dev"][..], "invalid option '--env'"),
    ] {
        let ran = run(args, data.path());
        assert_eq!(ran.status, 2, "{args:?}");
        assert!(
            ran.stderr.starts_with(&format!("error: {message}")),
            "{}",
            ran.stderr
        );
    }
}

#[test]
fn show_as_text_covers_every_section() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("api.imposter.json");
    let body = imposter_app::core::base64_bytes::encode(b"{\"name\": \"{{name}}\"}");
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({
            "format": "imposter", "version": 3,
            "collection": {
                "name": "API",
                "scripts": {"after": "test(\"ok\", true);"},
                "requests": [{
                    "name": "Create", "method": "POST", "url": "{{base}}/users",
                    "description": "Makes a user",
                    "auth": {"type": "basic", "basic": {"username": "ann", "password": "{{pass}}"}},
                    "queries": [{"name": "dry", "value": "1", "is_enabled": false}],
                    "headers": [{"name": "X-Trace", "value": "{{$uuid}}", "is_enabled": true}],
                    "cookies": [{"name": "sid", "value": "{{sid}}", "is_enabled": true}],
                    "body": body, "body_type": "json",
                    "options": {"timeout": 5.0},
                    "checks": [{"selector": "status", "op": "equals", "expected": "201", "enabled": false}],
                    "captures": [{"variable": "id", "selector": {"json": "$.id"}}],
                    "scripts": {"before": "vars.set(\"a\", \"1\");"}
                }]
            },
            "environments": [{"name": "Stage", "color": "yellow", "variables": {}}]
        }))
        .unwrap(),
    )
    .unwrap();
    let file = file.to_str().unwrap();
    let text = run(&["show", "Create", "--file", file, "--text"], data.path()).stdout;
    for line in [
        "POST {{base}}/users\n",
        "Path: API/Create\n",
        "Environment: none (globals and collection only)\n",
        "Description: Makes a user\n",
        "Query:\n  dry=1  (off)\n",
        "Headers:\n  X-Trace: {{$uuid}}\n",
        "Cookies:\n  sid={{sid}}\n",
        "Auth: Basic ann / {{pass}}\n",
        "Body (json, 20 bytes):\n  {\"name\": \"{{name}}\"}\n",
        "Settings: timeout=5.0\n",
        "Checks:\n  status = 201  (off)\n",
        "Captures:\n  id <- $.id\n",
        "Collection \"API\", after the response:\n  test(\"ok\", true);\n",
        "This request, before sending:\n  vars.set(\"a\", \"1\");\n",
        "  $uuid  generated\n",
        "  pass   undefined\n",
    ] {
        assert!(text.contains(line), "{line:?} in:\n{text}");
    }
    let cut = run(
        &[
            "show",
            "Create",
            "--file",
            file,
            "--text",
            "--max-body",
            "4",
        ],
        data.path(),
    )
    .stdout;
    assert!(cut.contains("  {\"na\n  …\n"), "{cut}");
    let envs = run(&["envs", "--file", file, "--text"], data.path()).stdout;
    assert!(envs.contains("  Stage (yellow): no variables\n"), "{envs}");
}
