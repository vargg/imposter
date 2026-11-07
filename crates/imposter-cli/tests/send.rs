//! `imposter send` end to end, in process: a real collection file, a loopback server, and the
//! exit status, stdout and stderr the command line produces.
#![cfg(test)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use imposter_cli::{Io, main};
use imposter_core::run_result::{RunOutcome, RunResult};
use imposter_testkit::{Received, Server, response};
use serde_json::{Value, json};

const SECRET: &str = "tok-SECRET-42";

/// A server on its own runtime, since the command line starts its own.
struct Serving {
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn serve(respond: impl Fn(usize, &Received) -> Vec<u8> + Send + Sync + 'static) -> Serving {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime.block_on(Server::start(respond));
    Serving {
        server,
        _runtime: runtime,
    }
}

fn request(name: &str, url: &str, extra: Value) -> Value {
    let mut request =
        json!({"name": name, "method": "GET", "url": url, "body": "", "body_type": "json"});
    if let (Some(request), Value::Object(extra)) = (request.as_object_mut(), extra) {
        request.extend(extra);
    }
    request
}

fn status_check(expected: &str) -> Value {
    json!([{"selector": "status", "op": "equals", "expected": expected}])
}

fn write_document(dir: &Path, base: &str, refused: &str) -> PathBuf {
    let document = json!({
        "format": "imposter",
        "version": 3,
        "collection": {
            "name": "API",
            "variables": {"base": base},
            "requests": [
                request("Pass", "{{base}}/pass", json!({"checks": status_check("200")})),
                request("Fail", "{{base}}/fail", json!({"checks": status_check("201")})),
                request("Undecided", "{{base}}/undecided", json!({
                    "checks": [{"selector": {"json": "nope"}, "op": "exists"}]
                })),
                request("Unchecked", "{{base}}/unchecked", json!({})),
                request("Inputs", "{{base}}/inputs?id={{id}}", json!({
                    "checks": [{"selector": "status", "op": "equals", "expected": "{{expected}}"}]
                })),
                request("After", "{{base}}/after", json!({"scripts": {"after": "throw \"broken\";"}})),
                request("Before", "{{base}}/before", json!({"scripts": {"before": "throw \"no key\";"}})),
                request("Unresolved", "{{base}}/{{missing}}", json!({})),
                request("Refused", refused, json!({"checks": status_check("200")})),
                request("Echo", "{{base}}/echo?key={{token}}", json!({
                    "auth": {"type": "bearer_token", "bearer_token": {"token": "{{token}}"}},
                    "headers": [{"name": "X-Key", "value": "{{token}}", "is_enabled": true}],
                    "checks": [{"selector": {"json": "$.echo"}, "op": "equals", "expected": "x"}],
                    "scripts": {"after": "print(vars.get(\"token\")); test(\"t\", false);"}
                })),
            ],
        },
        "environments": [{"name": "Dev", "variables": {"expected": "200"}}],
    });
    let path = dir.join("api.imposter.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    path
}

struct Ran {
    status: u8,
    stdout: String,
    stderr: String,
}

impl Ran {
    fn result(&self) -> RunResult {
        RunResult::read(self.stdout.as_bytes()).unwrap()
    }
}

fn run_with(args: &[&str], var: &dyn Fn(&str) -> Option<String>) -> Ran {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let status = main(
        args.iter().map(OsString::from).collect(),
        &mut Io {
            stdout: &mut stdout,
            stderr: &mut stderr,
            var,
        },
    );
    Ran {
        status,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn refused_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/refused")
}

fn echo(_: usize, received: &Received) -> Vec<u8> {
    let auth = received
        .header("authorization")
        .unwrap_or_default()
        .to_owned();
    let body = format!(
        r#"{{"echo":"{}","target":"{}"}}"#,
        received.header("x-key").unwrap_or_default(),
        received.target()
    );
    response(
        "200 OK",
        &[("Content-Type", "application/json"), ("X-Auth-Echo", &auth)],
        body.as_bytes(),
    )
}

#[test]
fn every_outcome_has_its_exit_status_and_result() {
    let serving = serve(|_, _| response("200 OK", &[], b"{}"));
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url, &refused_url());
    let file = file.to_str().unwrap();
    let env = vars(&[("USER_ID", "u1"), ("EXPECTED", "200")]);
    let var = |name: &str| env.get(name).cloned();
    for (request, extra, outcome) in [
        ("Pass", &[][..], RunOutcome::Passed),
        ("Fail", &[][..], RunOutcome::Failed),
        ("Undecided", &[][..], RunOutcome::NotChecked),
        ("Unchecked", &[][..], RunOutcome::NoChecks),
        ("After", &[][..], RunOutcome::ScriptFailed),
        ("Before", &[][..], RunOutcome::ScriptFailed),
        ("Unresolved", &[][..], RunOutcome::NotSent),
        ("Refused", &[][..], RunOutcome::TransportError),
        (
            "Inputs",
            &["--env", "Dev", "--input", "id=USER_ID"][..],
            RunOutcome::Passed,
        ),
        (
            "Inputs",
            &["--input", "id=USER_ID", "--input", "expected=EXPECTED"][..],
            RunOutcome::Passed,
        ),
        (
            "Inputs",
            &["--input", "id=NOT_SET", "--env", "Dev"][..],
            RunOutcome::NotSent,
        ),
        ("Inputs", &["--env", "Staging"][..], RunOutcome::NotSent),
        ("Twins", &[][..], RunOutcome::NotSent),
    ] {
        let mut args = vec!["send", file, request];
        args.extend_from_slice(extra);
        let ran = run_with(&args, &var);
        let result = ran.result();
        assert_eq!(
            result.outcome, outcome,
            "{request} {extra:?}: {}",
            ran.stdout
        );
        assert_eq!(ran.status, outcome.exit_status(), "{request} {extra:?}");
        assert_eq!(ran.stderr, format!("{}\n", result.summary), "{request}");
        assert_eq!(result.definition.request, request);
        assert_eq!(result.definition.sha256.len(), 64);
    }
    let targets: Vec<String> = serving
        .server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert!(
        !targets.iter().any(|t| t == "/before"),
        "a failed before-script sends nothing"
    );
    assert!(targets.contains(&"/inputs?id=u1".to_owned()));
}

#[test]
fn unsent_runs_say_why_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), "http://127.0.0.1:1", "http://127.0.0.1:1");
    let file = file.to_str().unwrap();
    let none = |_: &str| None;
    let cases = [
        (
            vec!["send", file, "Inputs", "--input", "id=USER_ID"],
            "Not sent: Input id: the process variable USER_ID is not set",
        ),
        (
            vec!["send", file, "Nope"],
            "Not sent: No request at Nope. Requests in this file: Pass, Fail,",
        ),
        (
            vec!["send", file, "a//b"],
            "Not sent: \"a//b\" is not a request path",
        ),
        (
            vec!["send", "/no/such/file.json", "Pass"],
            "Not sent: Cannot read the file:",
        ),
        (
            vec!["send", file, "Before"],
            "Script failed, nothing sent: Before-sending script, line 1: no key",
        ),
        (
            vec!["send", file, "Unresolved"],
            "Not sent: Unresolved variables: missing",
        ),
    ];
    for (args, start) in cases {
        let ran = run_with(&args, &none);
        assert!(ran.stderr.starts_with(start), "{args:?}: {}", ran.stderr);
    }
    let garbage = dir.path().join("garbage.json");
    std::fs::write(&garbage, "{\"format\": \"imposter\", \"version\": 1}").unwrap();
    let ran = run_with(&["send", garbage.to_str().unwrap(), "Pass"], &none);
    assert_eq!(ran.result().outcome, RunOutcome::NotSent);
    assert_eq!(
        ran.result().definition.sha256.len(),
        64,
        "a file that was read has its revision even when it is not a collection"
    );
}

#[test]
fn secrets_never_reach_stdout_stderr_or_the_result_file() {
    let serving = serve(echo);
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url, &refused_url());
    let out = dir.path().join("result.json");
    let env = vars(&[("API_TOKEN", SECRET), ("SHORT", "abc")]);
    let var = |name: &str| env.get(name).cloned();
    let ran = run_with(
        &[
            "send",
            file.to_str().unwrap(),
            "Echo",
            "--secret",
            "token=API_TOKEN",
            "--out",
            out.to_str().unwrap(),
        ],
        &var,
    );
    let received = serving.server.requests();
    assert_eq!(
        received[0].header("authorization"),
        Some(format!("Bearer {SECRET}").as_str()),
        "the secret is sent"
    );
    let written = std::fs::read_to_string(&out).unwrap();
    assert_eq!(written, ran.stdout, "the file holds the same result");
    for text in [&ran.stdout, &ran.stderr] {
        assert!(!text.contains(SECRET), "{text}");
        assert!(!text.contains("tok-SECRET"), "{text}");
    }
    let result = ran.result();
    assert_eq!(result.outcome, RunOutcome::Failed);
    assert_eq!(result.inputs[0].from, "API_TOKEN");
    assert_eq!(result.checks[0].actual.as_deref(), Some("«redacted»"));

    let short = run_with(
        &[
            "send",
            file.to_str().unwrap(),
            "Echo",
            "--secret",
            "token=SHORT",
        ],
        &var,
    );
    assert_eq!(short.result().outcome, RunOutcome::NotSent);
    assert!(short.stderr.contains("too short"), "{}", short.stderr);
    assert_eq!(
        serving.server.requests().len(),
        1,
        "a refused secret sends nothing"
    );
}

#[test]
fn an_unwritable_result_file_is_reported_after_the_result() {
    let serving = serve(|_, _| response("200 OK", &[], b""));
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url, &refused_url());
    let out = dir.path().join("missing-dir").join("result.json");
    let ran = run_with(
        &[
            "send",
            file.to_str().unwrap(),
            "Pass",
            "--out",
            out.to_str().unwrap(),
            "--max-body",
            "0",
        ],
        &|_| None,
    );
    assert_eq!(ran.status, 2);
    assert!(
        ran.stderr.contains("the result could not be written"),
        "{}",
        ran.stderr
    );
    assert_eq!(
        ran.result().outcome,
        RunOutcome::Passed,
        "stdout still has the result"
    );
    assert_eq!(
        ran.result().response.unwrap().body,
        None,
        "--max-body 0 shows no body"
    );
}

#[test]
fn help_version_and_usage_errors() {
    let help = run_with(&["--help"], &|_| None);
    assert_eq!(help.status, 0);
    assert!(help.stdout.contains("imposter send <FILE> <REQUEST>"));
    let version = run_with(&["--version"], &|_| None);
    assert_eq!(
        version.stdout,
        format!("imposter {}\n", env!("CARGO_PKG_VERSION"))
    );
    let wrong = run_with(&["send", "only-a-file"], &|_| None);
    assert_eq!(wrong.status, 2);
    assert_eq!(wrong.stdout, "");
    assert_eq!(
        wrong.stderr,
        "error: send takes a FILE and a REQUEST\nRun imposter --help for usage.\n"
    );
}

/// The run reads its file and inputs once: a check's expected value changed in the file and
/// the process environment while the response is on its way does not change the verdict.
#[test]
fn an_edit_during_the_run_does_not_change_it() {
    let (arrived, wait_arrived) = mpsc::channel::<()>();
    let (release, wait_release) = mpsc::channel::<()>();
    let arrived = Mutex::new(arrived);
    let wait_release = Mutex::new(wait_release);
    let serving = serve(move |_, _| {
        arrived.lock().unwrap().send(()).unwrap();
        wait_release.lock().unwrap().recv().unwrap();
        response("200 OK", &[], b"")
    });
    let dir = tempfile::tempdir().unwrap();
    let file = write_document(dir.path(), &serving.server.url, &refused_url());
    let env = Arc::new(Mutex::new(vars(&[("USER_ID", "u1"), ("EXPECTED", "200")])));
    let shared = env.clone();
    let path = file.to_str().unwrap().to_owned();
    let running = std::thread::spawn(move || {
        let var = move |name: &str| shared.lock().unwrap().get(name).cloned();
        run_with(
            &[
                "send",
                &path,
                "Inputs",
                "--input",
                "id=USER_ID",
                "--input",
                "expected=EXPECTED",
            ],
            &var,
        )
    });
    wait_arrived.recv().unwrap();
    let edited = std::fs::read_to_string(&file)
        .unwrap()
        .replace("{{expected}}", "500");
    std::fs::write(&file, edited).unwrap();
    env.lock().unwrap().insert("EXPECTED".into(), "500".into());
    release.send(()).unwrap();
    let ran = running.join().unwrap();
    assert_eq!(ran.result().outcome, RunOutcome::Passed, "{}", ran.stdout);
}
