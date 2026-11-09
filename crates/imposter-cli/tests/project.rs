//! `--project DIR`: a collections folder, as the app writes it, runs from a checkout with no
//! profile at all; the result names the folder and a revision over its files, which an
//! edit changes; a folder of several collections is chosen from by the path.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // copies and edits the folder

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use imposter_app::core::model::{RequestTemplate, Variables};
use imposter_app::store::{Store, collections, environments};
use imposter_cli::{Io, main};
use imposter_testkit::{Received, Server, response};
use serde_json::{Value, json};

struct Serving {
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn serve() -> Serving {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime.block_on(Server::start(|_, received: &Received| {
        let json = [("Content-Type", "application/json")];
        match received.target().split('?').next().unwrap_or_default() {
            "/login" => response("200 OK", &json, br#"{"token": "t-1"}"#),
            _ => response("200 OK", &json, br#"{"ok": true}"#),
        }
    }));
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
    Ran {
        status,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn json(ran: &Ran) -> Value {
    serde_json::from_str(&ran.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}\n{}", ran.stdout, ran.stderr))
}

/// Writes, through the store as the app does, "API" (a Flow folder: Login captures a
/// token, Use sends it) and "Admin" (one request), with a Dev environment.
fn written_by_the_app(data: &Path, base: &str) {
    let store = Store::open(&data.join("imposter.db")).unwrap();
    let base = base.to_owned();
    store
        .call_blocking(move |db| {
            let template = |value: Value| serde_json::from_value::<RequestTemplate>(value).unwrap();
            environments::create_environment(
                db,
                "Dev",
                &[("base".to_owned(), base.clone())].into_iter().collect(),
            )?;
            environments::set_globals(
                db,
                &[("stats_path".to_owned(), "stats".to_owned())]
                    .into_iter()
                    .collect(),
            )?;
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            let flow = collections::create_folder(db, api.id, None, "Flow", "", 0)?;
            collections::create_request(db, api.id, Some(flow.id), &template(json!({
                "name": "Login", "method": "POST", "url": "{{base}}/login", "position": 0,
                "body": "", "body_type": "none",
                "captures": [{"variable": "token", "selector": {"json": "$.token"}}],
            })))?;
            collections::create_request(db, api.id, Some(flow.id), &template(json!({
                "name": "Use", "method": "GET", "url": "{{base}}/use?t={{token}}", "position": 1,
                "body": "", "body_type": "none",
                "checks": [{"selector": "status", "op": "equals", "expected": "200"}],
            })))?;
            let admin = collections::create_collection(db, "Admin", "", &Variables::new())?;
            collections::create_request(db, admin.id, None, &template(json!({
                "name": "Stats", "method": "GET", "url": "{{base}}/{{stats_path}}",
                "body": "", "body_type": "none",
            })))?;
            Ok(())
        })
        .unwrap();
    store.close().unwrap();
}

fn copy(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn a_checkout_runs_with_no_profile() {
    let serving = serve();
    let author = tempfile::tempdir().unwrap();
    written_by_the_app(author.path(), &serving.server.url);
    // A teammate's checkout: the folder alone, and a data directory with nothing in it.
    let checkout = tempfile::tempdir().unwrap();
    copy(&author.path().join("collections"), checkout.path());
    let empty = tempfile::tempdir().unwrap();
    let dir = checkout.path().to_str().unwrap();

    let ran = run(
        &["run", "API / Flow", "--project", dir, "--env", "Dev"],
        empty.path(),
    );
    assert_eq!(ran.status, 0, "{}\n{}", ran.stderr, ran.stdout);
    let report = json(&ran);
    assert_eq!(report["definition"]["source"], "project");
    assert_eq!(report["definition"]["file"], dir);
    let revision = report["definition"]["sha256"].as_str().unwrap().to_owned();
    assert_eq!(revision.len(), 64);
    let targets: Vec<String> = serving
        .server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(
        targets,
        ["/login", "/use?t=t-1"],
        "the captured token carried"
    );
    assert!(
        !empty.path().join("imposter.db").exists(),
        "no profile was created"
    );

    let sent = run(
        &["send", "Admin/Stats", "--project", dir, "--env", "Dev"],
        empty.path(),
    );
    assert_eq!(json(&sent)["definition"]["source"], "project");
    assert_eq!(
        serving.server.requests().last().unwrap().target(),
        "/stats",
        "the folder's shared globals apply"
    );

    let listed = json(&run(&["list", "--project", dir], empty.path()));
    let paths: Vec<&str> = listed["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["API/Flow/Login", "API/Flow/Use", "Admin/Stats"]);
    assert_eq!(listed["source"], "project");
    let shown = json(&run(
        &["show", "API/Flow/Use", "--project", dir, "--env", "Dev"],
        empty.path(),
    ));
    assert_eq!(shown["source"], "project");
    let envs = json(&run(&["envs", "--project", dir], empty.path()));
    assert_eq!(envs["environments"][0]["name"], "Dev");
    assert_eq!(envs["collections"].as_array().unwrap().len(), 2);

    // An edit to any file is a new revision.
    let use_file = checkout.path().join("api/flow/use.yaml");
    let text = std::fs::read_to_string(&use_file).unwrap();
    std::fs::write(
        &use_file,
        text.replace("expected: \"200\"", "expected: \"201\""),
    )
    .unwrap();
    let edited = run(
        &["run", "API/Flow", "--project", dir, "--env", "Dev"],
        empty.path(),
    );
    assert_eq!(edited.status, 1, "the edited check fails");
    assert_ne!(json(&edited)["definition"]["sha256"], revision.as_str());
}

#[test]
fn the_folder_must_be_one_and_its_collection_named() {
    let serving = serve();
    let author = tempfile::tempdir().unwrap();
    written_by_the_app(author.path(), &serving.server.url);
    let dir = author.path().join("collections");
    let dir = dir.to_str().unwrap();

    let both = run(
        &["send", "API/Flow/Use", "--project", dir, "--file", "x.json"],
        author.path(),
    );
    assert_eq!(both.status, 2);
    assert!(
        both.stderr.contains("--file and --project"),
        "{}",
        both.stderr
    );

    let unnamed = run(&["send", "Stats", "--project", dir], author.path());
    assert_eq!(unnamed.status, 3, "two collections: the path must name one");
    assert!(
        unnamed.stdout.contains("API, Admin") || unnamed.stdout.contains("Admin, API"),
        "{}",
        unnamed.stdout
    );

    let missing = run(
        &[
            "list",
            "--project",
            author.path().join("nowhere").to_str().unwrap(),
        ],
        author.path(),
    );
    assert_ne!(missing.status, 0);
    assert!(
        missing.stderr.contains("holds no collection"),
        "{}",
        missing.stderr
    );

    // Without --project, the profile's folder is read, as the app reads it.
    let from_profile = run(&["send", "Admin/Stats", "--env", "Dev"], author.path());
    assert_eq!(json(&from_profile)["definition"]["source"], "profile");
}

/// A checkout whose request uploads `fixtures/data.bin`: the path is the folder's, so the
/// run works from any working directory (this test's is the crate's).
#[test]
fn relative_files_are_found_in_the_checkout() {
    let serving = serve();
    let checkout = tempfile::tempdir().unwrap();
    let api = checkout.path().join("api");
    std::fs::create_dir_all(checkout.path().join("fixtures")).unwrap();
    std::fs::create_dir_all(&api).unwrap();
    std::fs::write(
        checkout.path().join("fixtures/data.bin"),
        b"from-the-checkout",
    )
    .unwrap();
    std::fs::write(
        api.join("imposter.yaml"),
        format!(
            "format: imposter-project\nversion: 1\nid: c1\nname: API\nvariables:\n  base: {}\n",
            serving.server.url
        ),
    )
    .unwrap();
    std::fs::write(
        api.join("upload.yaml"),
        "id: r1\nname: Upload\nmethod: POST\nurl: \"{{base}}/upload\"\nbody:\n  file: fixtures/data.bin\n",
    )
    .unwrap();
    let empty = tempfile::tempdir().unwrap();
    let ran = run(
        &[
            "send",
            "API/Upload",
            "--project",
            checkout.path().to_str().unwrap(),
        ],
        empty.path(),
    );
    assert_eq!(ran.status, 7, "no checks, but sent: {}", ran.stdout);
    assert_eq!(serving.server.requests()[0].body, b"from-the-checkout");
}
