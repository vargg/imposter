//! `imposter run` in process: a folder as one run with values carried forward, progress on
//! stderr, the report on stdout, `--junit` and `--out`, stopping and `--keep-going`, the
//! exit statuses, and never a secret anywhere.
#![cfg(test)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use imposter_cli::{Io, main};
use imposter_testkit::{Received, Server, response};
use serde_json::{Value, json};

const SECRET: &str = "tok-SECRET-42";

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
        let path = received.target().split('?').next().unwrap_or_default();
        match (received.method(), path) {
            ("POST", "/login") => response("200 OK", &json, br#"{"token": "run-token"}"#),
            ("POST", "/items") => {
                let auth = received.header("authorization").unwrap_or_default();
                if auth == "Bearer run-token" {
                    response("201 Created", &json, br#"{"id": 7}"#)
                } else {
                    response("401 Unauthorized", &[], b"")
                }
            }
            ("GET", "/items/7") => response("200 OK", &json, br#"{"id": 7}"#),
            _ => response("500 Internal Server Error", &[], b""),
        }
    }));
    Serving {
        server,
        _runtime: runtime,
    }
}

fn request(name: &str, method: &str, url: &str, extra: Value) -> Value {
    let mut request =
        json!({"name": name, "method": method, "url": url, "body": "", "body_type": "none"});
    if let (Some(map), Value::Object(extra)) = (request.as_object_mut(), extra) {
        map.extend(extra);
    }
    request
}

/// "Flow": Login (captures the token, no checks), Create (uses it, captures id), Read
/// (uses the id). "Broken": a failing request, then one more. "Plain": no checks at all.
fn write_document(dir: &Path, base: &str) -> PathBuf {
    let status = |code: &str| json!([{"selector": "status", "op": "equals", "expected": code}]);
    let document = json!({
        "format": "imposter", "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": base},
            "folders": [
                {"name": "Flow", "position": 0, "requests": [
                    request("Login", "POST", "{{base}}/login?key={{key}}", json!({
                        "position": 0,
                        "captures": [{"variable": "token", "selector": {"json": "$.token"}}]
                    })),
                    request("Create", "POST", "{{base}}/items", json!({
                        "position": 1,
                        "auth": {"type": "bearer_token", "bearer_token": {"token": "{{token}}"}},
                        "checks": status("201"),
                        "captures": [{"variable": "id", "selector": {"json": "$.id"}}]
                    })),
                    request("Read", "GET", "{{base}}/items/{{id}}", json!({"position": 2, "checks": status("200")}))
                ]},
                {"name": "Broken", "position": 1, "requests": [
                    request("Fails", "GET", "{{base}}/fail", json!({"position": 0, "checks": status("200")})),
                    request("After", "GET", "{{base}}/items/7", json!({"position": 1, "checks": status("200")}))
                ]},
                {"name": "Plain", "position": 2, "requests": [
                    request("Ping", "GET", "{{base}}/items/7", json!({}))
                ]},
                {"name": "Empty", "position": 3}
            ]
        }
    });
    let path = dir.join("api.imposter.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

struct Ran {
    status: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Ran {
    let env: HashMap<String, String> = [("API_KEY".to_owned(), SECRET.to_owned())].into();
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

fn report(ran: &Ran) -> Value {
    serde_json::from_str(&ran.stdout).unwrap_or_else(|e| panic!("{e}: {}", ran.stdout))
}

#[test]
fn a_folder_runs_as_one_run_and_writes_junit() {
    let serving = serve();
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url);
    let junit = dir.path().join("report.xml");
    let out = dir.path().join("report.json");
    let ran = run(&[
        "run",
        "Flow",
        "--file",
        file.to_str().unwrap(),
        "--secret",
        "key=API_KEY",
        "--junit",
        junit.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(ran.status, 0, "{}\n{}", ran.stderr, ran.stdout);
    let report = report(&ran);
    assert_eq!(report["format"], "imposter-run");
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["definition"]["request"], "Flow");
    assert_eq!(report["definition"]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(report["totals"]["checks_passed"], 2);
    let lines: Vec<&str> = ran.stderr.lines().collect();
    assert_eq!(lines.len(), 4, "{}", ran.stderr);
    assert!(
        lines[0].starts_with("ok      API/Flow/Login · No checks"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("ok      API/Flow/Create · Passed"),
        "{}",
        lines[1]
    );
    assert!(
        lines[3].starts_with("Passed: 3 requests, 2 checks passed"),
        "{}",
        lines[3]
    );
    let targets: Vec<String> = serving
        .server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets[2], "/items/7", "the captured id reached Read");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), ran.stdout);
    let xml = std::fs::read_to_string(&junit).unwrap();
    assert!(
        xml.contains("<testcase classname=\"API/Flow\" name=\"Create\""),
        "{xml}"
    );
    assert!(!xml.contains(SECRET));
}

#[test]
fn a_failure_stops_the_run_and_keep_going_runs_the_rest() {
    let serving = serve();
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url);
    let file = file.to_str().unwrap();
    let stopped = run(&["run", "API / Broken", "--file", file]);
    assert_eq!(stopped.status, 1);
    assert!(
        stopped.stderr.contains("FAILED  API/Broken/Fails · Failed"),
        "{}",
        stopped.stderr
    );
    assert!(
        stopped
            .stderr
            .contains("skipped API/Broken/After (an earlier request did not pass)"),
        "{}",
        stopped.stderr
    );
    assert_eq!(report(&stopped)["totals"]["skipped"], 1);

    let kept = run(&["run", "Broken", "--file", file, "--keep-going"]);
    assert_eq!(kept.status, 1, "still failed");
    assert_eq!(report(&kept)["totals"]["skipped"], 0);
    assert_eq!(report(&kept)["totals"]["passed"], 1);
}

#[test]
fn every_run_outcome_has_its_exit_status() {
    let serving = serve();
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url);
    let file = file.to_str().unwrap();
    for (args, status, outcome) in [
        (&["run", "Plain", "--file", file][..], 7, "no_checks"),
        (&["run", "Empty", "--file", file][..], 3, "not_sent"),
        (&["run", "Nope", "--file", file][..], 3, "not_sent"),
        (&["run", "Flow/Read", "--file", file][..], 3, "not_sent"),
        (
            &["run", "Flow", "--file", file, "--input", "key=NOT_SET"][..],
            3,
            "not_sent",
        ),
    ] {
        let ran = run(args);
        assert_eq!(ran.status, status, "{args:?}: {}", ran.stderr);
        assert_eq!(report(&ran)["outcome"], outcome, "{args:?}");
    }
    let nope = run(&["run", "Nope", "--file", file]);
    assert!(
        nope.stderr
            .contains("Collections and folders: API, API/Flow, API/Broken"),
        "{}",
        nope.stderr
    );
    let usage = run(&["run"]);
    assert_eq!(usage.status, 2);
    assert!(
        usage.stderr.starts_with("error: run takes one TARGET"),
        "{}",
        usage.stderr
    );
    let unwritable = run(&[
        "run",
        "Plain",
        "--file",
        file,
        "--junit",
        dir.path().join("missing/report.xml").to_str().unwrap(),
    ]);
    assert_eq!(unwritable.status, 2);
    assert!(
        unwritable.stderr.contains("error: could not write"),
        "{}",
        unwritable.stderr
    );
}
