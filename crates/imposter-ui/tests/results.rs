//! Result files the command line writes, opened in History: the desktop shows the same
//! outcome and verdicts the command line reported.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app and the command line from outside

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use egui_kittest::kittest::Queryable;
use imposter_app::core::run_result::{RunOutcome, RunResult};
use imposter_testkit::{Server, response};
use imposter_ui::app::Screen;
use serde_json::json;
use support::{click, settle, start};

fn collection(dir: &Path, base: &str) -> PathBuf {
    let request = |name: &str, url: &str, checks: serde_json::Value| json!({"name": name, "method": "GET", "url": url, "body": "", "body_type": "json", "checks": checks});
    let document = json!({
        "format": "imposter",
        "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": base},
            "folders": [{"name": "Users", "requests": [
                request("Pass", "{{base}}/users", json!([{"selector": "status", "op": "equals", "expected": "200"}])),
                request("Fail", "{{base}}/users", json!([{"selector": "status", "op": "equals", "expected": "201"}])),
                request("Undecided", "{{base}}/users", json!([{"selector": {"json": "nope"}, "op": "exists"}])),
                request("Unresolved", "{{base}}/{{missing}}", json!([])),
            ]}],
        },
    });
    let path = dir.join("api.imposter.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

/// Run `imposter send … --out` in process, as a CI job would, and return the exit status.
fn headless(command: &str, file: &Path, request: &str, out: &Path) -> u8 {
    let args: Vec<OsString> = [
        command,
        "--file",
        file.to_str().unwrap(),
        request,
        "--out",
        out.to_str().unwrap(),
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    imposter_cli::main(
        args,
        &mut imposter_cli::Io {
            stdout: &mut stdout,
            stderr: &mut stderr,
            var: &|_| None,
        },
    )
}

fn open_result(app: &mut support::App, path: &Path) {
    app.harness
        .state_mut()
        .script_file_dialogs([Some(path.to_owned())]);
    click(&mut app.harness, "History actions");
    click(&mut app.harness, "Open result file…");
    settle(&mut app.harness);
}

#[test]
fn results_written_by_the_command_line_open_in_history_with_the_same_verdicts() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"id": 7}"#,
        )
    }));
    let dir = tempfile::tempdir().unwrap();
    let file = collection(dir.path(), &server.url);
    let mut app = start();
    click(&mut app.harness, "History");
    settle(&mut app.harness);

    for (request, outcome) in [
        ("Users/Pass", RunOutcome::Passed),
        ("Users/Fail", RunOutcome::Failed),
        ("Users/Undecided", RunOutcome::NotChecked),
        ("Users/Unresolved", RunOutcome::NotSent),
    ] {
        let out = dir
            .path()
            .join(format!("{}.json", request.replace('/', "-")));
        let status = headless("send", &file, request, &out);
        assert_eq!(status, outcome.exit_status(), "{request}");
        let result = RunResult::read(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(result.outcome, outcome);

        open_result(&mut app, &out);
        assert_eq!(app.harness.state().state.screen, Screen::History);
        let opened = app.harness.state().state.history.opened.clone().unwrap();
        assert_eq!(
            opened.current(),
            Some(&result),
            "the desktop shows the file as written"
        );
        app.harness.get_by_label(&result.summary);
        let revision = result.definition.sha256.as_deref().unwrap();
        app.harness
            .get_by_label(&format!("Revision {}", &revision[..12]));
        app.harness.get_by_label_contains(&format!(
            "From imposter send · {request} · no environment · run "
        ));
        let shown = app
            .harness
            .state()
            .state
            .history
            .response
            .response
            .as_ref()
            .map(|r| {
                r.checks
                    .iter()
                    .map(|c| c.verdict.clone())
                    .collect::<Vec<_>>()
            });
        let written: Vec<_> = result.checks.iter().map(|c| c.verdict.clone()).collect();
        match outcome {
            RunOutcome::NotSent => {
                assert_eq!(shown, None, "nothing arrived");
                app.harness.get_by_label("Unresolved variables: missing");
            }
            _ => assert_eq!(shown, Some(written), "{request}"),
        }
    }

    click(&mut app.harness, "Close result");
    assert_eq!(app.harness.state().state.history.opened, None);
    app.harness
        .get_by_label("Select an execution to see what was sent and received.");
}

#[test]
fn a_run_report_shows_its_source_and_lets_the_reader_choose_a_failed_check() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let dir = tempfile::tempdir().unwrap();
    let file = collection(dir.path(), &server.url);
    let out = dir.path().join("run.json");
    assert_eq!(
        headless("run", &file, "API", &out),
        RunOutcome::Failed.exit_status()
    );
    let mut report =
        imposter_app::core::run_report::RunReport::read(&std::fs::read(&out).unwrap()).unwrap();
    let mut app = start();
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    open_result(&mut app, &out);
    app.harness.get_by_label(&report.summary);
    app.harness
        .get_by_label_contains("From imposter run · API · no environment");
    click(&mut app.harness, "Result request");
    let label = match &report.steps[1] {
        imposter_app::core::run_report::Step::Ran(result) => {
            format!("API/Users/Fail · {}", result.summary)
        }
        skipped @ imposter_app::core::run_report::Step::Skipped { .. } => panic!("{skipped:?}"),
    };
    click(&mut app.harness, &label);
    settle(&mut app.harness);
    let opened = app.harness.state().state.history.opened.as_ref().unwrap();
    assert_eq!(opened.current().unwrap().outcome, RunOutcome::Failed);

    report.run.tool = "imposter mcp test-version".to_owned();
    report.definition.environment = Some("Dev".to_owned());
    std::fs::write(&out, report.to_json()).unwrap();
    open_result(&mut app, &out);
    app.harness
        .get_by_label_contains("From imposter mcp · API · Dev");
}

#[test]
fn a_file_that_is_not_a_result_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let file = collection(dir.path(), "http://127.0.0.1:1");
    let mut app = start();
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    open_result(&mut app, &file);
    assert_eq!(app.harness.state().state.history.opened, None);
    app.harness
        .get_by_label_contains("Cannot open api.imposter.json: Not an Imposter result");
}
