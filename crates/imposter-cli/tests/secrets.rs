//! Secrets on the command line: a profile run uses the named environment's stored secrets
//! and never prints one, nor a secret it captured; `--secret` fills `{{secret:name}}` from
//! a file; `envs` and `show` name secrets, never their values.
#![cfg(test)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use imposter_app::core::variables::SecretScope;
use imposter_cli::{Io, main};
use imposter_testkit::{Received, Server, response};
use serde_json::{Value, json};

const STORED: &str = "stored-dev-secret-1";
const GLOBAL: &str = "stored-global-secret-1";
const ISSUED: &str = "issued-by-login-77";
const GIVEN: &str = "given-on-the-line-9";

struct Serving {
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn reply(_: usize, received: &Received) -> Vec<u8> {
    let json = [("Content-Type", "application/json")];
    let path = received.target().split('?').next().unwrap_or_default();
    // Every answer echoes what it received, so a secret that went out comes back.
    let echo = format!(
        r#"{{"target": "{}", "auth": "{}", "token": "{ISSUED}"}}"#,
        received.target(),
        received.header("authorization").unwrap_or_default()
    );
    match path {
        "/fail" => response("500 Internal Server Error", &json, echo.as_bytes()),
        _ => response("200 OK", &json, echo.as_bytes()),
    }
}

fn serve() -> Serving {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime.block_on(Server::start(reply));
    Serving {
        server,
        _runtime: runtime,
    }
}

struct Ran {
    status: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], env: &HashMap<String, String>) -> Ran {
    let var = |name: &str| env.get(name).cloned();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let status = main(
        args.iter().map(OsString::from).collect(),
        &mut Io {
            stdout: &mut stdout,
            stderr: &mut stderr,
            var: &var,
        },
    );
    Ran {
        status,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn assert_clean(ran: &Ran, secrets: &[&str]) {
    for secret in secrets {
        assert!(
            !ran.stdout.contains(secret),
            "{secret} on stdout: {}",
            ran.stdout
        );
        assert!(
            !ran.stderr.contains(secret),
            "{secret} on stderr: {}",
            ran.stderr
        );
    }
}

fn template(value: Value) -> imposter_app::core::model::RequestTemplate {
    serde_json::from_value(value).unwrap()
}

/// "API / Flow": Login captures a token as a secret, Use sends it and the stored one.
/// Dev holds `token`; the globals hold `token` and `key`.
fn profile(dir: &Path, base: &str) {
    use imposter_app::core::model::Variables;
    use imposter_app::store::{Store, collections, environments, secrets};
    let store = Store::open(&dir.join("imposter.db")).unwrap();
    let base = base.to_owned();
    store
        .call_blocking(move |db| {
            let one = |name: &str, value: &str| -> Variables {
                [(name.to_owned(), value.to_owned())].into_iter().collect()
            };
            let dev = environments::create_environment(db, "Dev", &Variables::new())?;
            secrets::write(db, SecretScope::Environment(dev.id), &one("token", STORED))?;
            let mut global = one("token", GLOBAL);
            global.insert("key".into(), "global-key-123".into());
            secrets::write(db, SecretScope::Global, &global)?;
            let api = collections::create_collection(db, "API", "", &one("base", &base))?;
            let flow = collections::create_folder(db, api.id, None, "Flow", "", 0)?;
            collections::create_request(
                db,
                api.id,
                Some(flow.id),
                &template(json!({
                    "name": "Login", "method": "POST", "url": "{{base}}/login",
                    "body": "", "body_type": "none", "position": 0,
                    "captures": [{"variable": "issued", "selector": {"json": "$.token"}, "secret": true}],
                })),
            )?;
            collections::create_request(
                db,
                api.id,
                Some(flow.id),
                &template(json!({
                    "name": "Use", "method": "GET", "position": 1,
                    "url": "{{base}}/use?t={{secret:issued}}",
                    "body": "", "body_type": "none",
                    "auth": {"type": "bearer_token", "bearer_token": {"token": "{{secret:token}}"}},
                    "checks": [{"selector": "status", "op": "equals", "expected": "200"}],
                })),
            )?;
            Ok(())
        })
        .unwrap();
    store.close().unwrap();
}

#[test]
fn a_profile_run_uses_stored_secrets_and_prints_none() {
    let serving = serve();
    let data = tempfile::tempdir().unwrap();
    profile(data.path(), &serving.server.url);
    let env: HashMap<String, String> = [(
        "IMPOSTER_DATA_DIR".to_owned(),
        data.path().to_str().unwrap().to_owned(),
    )]
    .into();

    let ran = run(&["run", "API / Flow", "--env", "Dev"], &env);
    assert_eq!(ran.status, 0, "{}\n{}", ran.stderr, ran.stdout);
    let targets: Vec<String> = serving
        .server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(
        targets[1],
        format!("/use?t={ISSUED}"),
        "the captured secret"
    );
    assert_eq!(
        serving.server.requests()[1].header("authorization"),
        Some(format!("Bearer {STORED}").as_str()),
        "Dev's secret beats the global one"
    );
    assert_clean(&ran, &[STORED, GLOBAL, ISSUED]);

    let sent = run(&["send", "API/Flow/Login", "--env", "Dev"], &env);
    assert_eq!(sent.status, 7, "no checks: {}", sent.stdout);
    assert_clean(&sent, &[STORED, GLOBAL, ISSUED]);
    let result: Value = serde_json::from_str(&sent.stdout).unwrap();
    assert_eq!(
        result["captures"][0]["taken"]["value"], "«redacted»",
        "{result}"
    );

    let shown = run(&["show", "API/Flow/Use", "--env", "Dev"], &env);
    assert_eq!(shown.status, 0, "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let variables = shown["variables"].as_array().unwrap();
    let defined = |name: &str| {
        variables
            .iter()
            .find(|v| v["name"] == name)
            .map(|v| v["defined_in"].clone())
    };
    assert_eq!(defined("secret:token"), Some(json!("Dev secrets")));
    assert_eq!(defined("secret:issued"), Some(json!("undefined")));

    let envs = run(&["envs"], &env);
    assert_clean(&envs, &[STORED, GLOBAL, "global-key-123"]);
    let envs: Value = serde_json::from_str(&envs.stdout).unwrap();
    assert_eq!(envs["environments"][0]["secrets"], json!(["token"]));
    assert_eq!(envs["global_secrets"], json!(["token", "key"]));
    let text = run(&["envs", "--text"], &env);
    assert!(
        text.stdout.contains("    secrets: token, key"),
        "{}",
        text.stdout
    );

    // History holds the run, without the values.
    let store = imposter_app::store::Store::open(&data.path().join("imposter.db")).unwrap();
    let payloads: Vec<String> = store
        .call_blocking(|db| {
            let mut statement = db.prepare("SELECT payload FROM executions")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    store.close().unwrap();
    assert_eq!(payloads.len(), 3);
    for payload in payloads {
        for secret in [STORED, ISSUED] {
            assert!(!payload.contains(secret), "{secret} in History");
        }
    }
}

#[test]
fn secret_fills_a_secret_reference_in_a_file() {
    let serving = serve();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("api.imposter.json");
    let document = json!({
        "format": "imposter", "version": 2,
        "collection": {
            "name": "API", "variables": {"base": serving.server.url},
            "requests": [{
                "name": "Use", "method": "GET", "url": "{{base}}/use?t={{secret:token}}&v={{token}}",
                "body": "", "body_type": "none",
                "checks": [{"selector": "status", "op": "equals", "expected": "200"}],
            }],
        },
    });
    std::fs::write(&file, serde_json::to_vec(&document).unwrap()).unwrap();
    let env: HashMap<String, String> = [("TOKEN".to_owned(), GIVEN.to_owned())].into();
    let ran = run(
        &[
            "send",
            "Use",
            "--file",
            file.to_str().unwrap(),
            "--secret",
            "token=TOKEN",
        ],
        &env,
    );
    assert_eq!(ran.status, 0, "{}", ran.stdout);
    assert_eq!(
        serving.server.requests()[0].target(),
        format!("/use?t={GIVEN}&v={GIVEN}"),
        "both spellings, as before"
    );
    assert_clean(&ran, &[GIVEN]);

    let missing = run(&["send", "Use", "--file", file.to_str().unwrap()], &env);
    assert_eq!(missing.status, 3, "a file has no stored secrets");
    assert!(
        missing.stdout.contains("secret:token"),
        "{}",
        missing.stdout
    );
}
