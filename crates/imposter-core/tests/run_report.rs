//! The report of a run of several requests: the pass rule, its precedence, totals, an
//! empty run, skipped steps, and the JUnit XML it gives.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reading the golden file is the test harness, not core code

use std::path::Path;

use imposter_core::check::{Check, CheckResult, Op, Verdict};
use imposter_core::execution::{
    ExecutionRecord, Outcome, Response, ScriptReport, ScriptTest, Timestamp, Via,
};
use imposter_core::model::{Request, RequestTemplate};
use imposter_core::redact::Redactor;
use imposter_core::run_report::{Meta, RunReport, Step};
use imposter_core::run_result::{
    self, Definition, Input, Limits, RunInfo, RunOutcome, RunResult, Source,
};
use imposter_core::select::Selector;
use imposter_core::settings::AppSettings;

const SECRET: &str = "tok-SECRET-42";

fn info(id: &str) -> RunInfo {
    RunInfo {
        id: id.into(),
        tool: "imposter 0.0.0".into(),
        started: Timestamp(1_793_000_000_000).to_rfc3339(),
        elapsed_ms: 42,
    }
}

fn definition(target: &str) -> Definition {
    Definition {
        source: Source::Profile,
        file: None,
        sha256: None,
        request: target.into(),
        environment: Some("Dev".into()),
    }
}

fn limits() -> Limits {
    Limits {
        capture_bytes: 5_242_880,
        excerpt_bytes: 64,
    }
}

fn result_meta(path: &str) -> run_result::Meta {
    run_result::Meta {
        run: info("step"),
        definition: definition(path),
        inputs: Vec::new(),
        limits: limits(),
    }
}

/// A request at `path` that got `status`, with checks of `verdicts` and script tests.
fn sent(path: &str, status: u16, verdicts: &[Verdict], tests: &[bool]) -> Step {
    let record = ExecutionRecord {
        timestamp: Timestamp(0),
        request_id: None,
        template: Request::default(),
        resolved: Request {
            template: RequestTemplate {
                method: "GET".into(),
                url: "http://api.test/x".into(),
                ..RequestTemplate::default()
            },
            ..Request::default()
        },
        settings: AppSettings::default(),
        environment_id: None,
        environment: "Dev".into(),
        response: Response {
            status,
            reason: "OK".into(),
            elapsed: 0.012,
            outcome: Outcome::Response,
            checks: verdicts
                .iter()
                .map(|verdict| CheckResult {
                    check: Check::new(Selector::Status, Op::Equals, "200"),
                    verdict: verdict.clone(),
                    actual: matches!(verdict, Verdict::Failed)
                        .then(|| format!("{status} {SECRET}")),
                })
                .collect(),
            script: (!tests.is_empty()).then(|| ScriptReport {
                tests: tests
                    .iter()
                    .map(|passed| ScriptTest {
                        name: "t".into(),
                        passed: *passed,
                        message: if *passed {
                            String::new()
                        } else {
                            "expected <1>".into()
                        },
                    })
                    .collect(),
                error: String::new(),
            }),
            ..Response::default()
        },
        via: Via::CommandLine,
    };
    Step::Ran(Box::new(RunResult::sent(
        &record,
        result_meta(path),
        &Redactor::default(),
    )))
}

fn unsent(path: &str, outcome: RunOutcome, error: &str) -> Step {
    Step::Ran(Box::new(RunResult::unsent(
        outcome,
        error,
        result_meta(path),
    )))
}

fn skipped(path: &str) -> Step {
    Step::Skipped {
        path: path.into(),
        reason: "an earlier request did not pass".into(),
    }
}

fn report(steps: Vec<Step>) -> RunReport {
    RunReport::new(
        steps,
        Meta {
            run: info("run-1"),
            definition: definition("API / Flow"),
            inputs: vec![Input {
                variable: "token".into(),
                from: "API_TOKEN".into(),
                secret: true,
            }],
            limits: limits(),
        },
    )
}

#[test]
fn a_run_passes_when_checked_requests_pass_and_some_check_ran() {
    let login = sent("API/Flow/Login", 200, &[], &[]);
    let create = sent("API/Flow/Create", 201, &[Verdict::Passed], &[true]);
    let passed = report(vec![login.clone(), create]);
    assert_eq!(
        passed.outcome,
        RunOutcome::Passed,
        "a login without checks is fine"
    );
    assert_eq!(
        passed.summary,
        "Passed: 2 requests, 2 checks passed · 42 ms"
    );
    assert_eq!((passed.totals.passed, passed.totals.checks_passed), (2, 2));

    let unchecked = report(vec![login.clone(), sent("API/Flow/Get", 200, &[], &[])]);
    assert_eq!(unchecked.outcome, RunOutcome::NoChecks);
    assert_eq!(unchecked.outcome.exit_status(), 7);

    let empty = report(Vec::new());
    assert_eq!(
        empty.outcome,
        RunOutcome::NotSent,
        "nothing to run is not success"
    );
    assert_eq!(empty.summary, "Not sent: nothing to run");
}

#[test]
fn the_outcome_that_says_least_was_verified_wins() {
    let ok = || sent("API/Ok", 200, &[Verdict::Passed], &[]);
    let cases = [
        (
            sent("API/Bad", 200, &[Verdict::Failed], &[]),
            RunOutcome::Failed,
        ),
        (sent("API/Test", 200, &[], &[false]), RunOutcome::Failed),
        (
            sent("API/Cut", 200, &[Verdict::Unchecked("cut".into())], &[]),
            RunOutcome::NotChecked,
        ),
        (
            unsent("API/Gone", RunOutcome::TransportError, "refused"),
            RunOutcome::TransportError,
        ),
        (
            unsent("API/Var", RunOutcome::NotSent, "Unresolved variables: id"),
            RunOutcome::NotSent,
        ),
        (
            unsent("API/Stop", RunOutcome::Cancelled, "Cancelled"),
            RunOutcome::Cancelled,
        ),
    ];
    for (bad, expected) in cases {
        let run = report(vec![ok(), bad, skipped("API/Later")]);
        assert_eq!(run.outcome, expected, "{}", run.summary);
        assert_eq!((run.totals.failed, run.totals.skipped), (1, 1));
    }
    let both = report(vec![
        sent("API/Bad", 200, &[Verdict::Failed], &[]),
        unsent("API/Gone", RunOutcome::TransportError, "refused"),
    ]);
    assert_eq!(both.outcome, RunOutcome::TransportError);
    let failed = report(vec![
        ok(),
        sent("API/Bad", 200, &[Verdict::Failed], &[]),
        skipped("API/Later"),
    ]);
    assert!(
        failed
            .summary
            .starts_with("Failed at API/Bad: 1 of 1 checks failed · 200 OK"),
        "{}",
        failed.summary
    );
    assert!(
        failed.summary.ends_with(" · 2 of 3 requests ran"),
        "{}",
        failed.summary
    );
}

#[test]
fn the_report_redacts_and_gives_junit_matching_its_golden_file() {
    let mut redactor = Redactor::default();
    redactor.secret(SECRET);
    let run = report(vec![
        sent("API/Flow/Login", 200, &[], &[]),
        sent(
            "API/Flow/Create <&>",
            201,
            &[Verdict::Passed, Verdict::Failed],
            &[false],
        ),
        unsent(
            "API/Flow/Get",
            RunOutcome::NotSent,
            "Unresolved variables: id",
        ),
        skipped("API/Flow/Delete"),
    ])
    .redacted(&redactor);
    let json = run.to_json();
    assert!(!json.contains(SECRET), "{json}");
    let xml = run.junit();
    assert!(!xml.contains(SECRET), "{xml}");
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/results/run.junit.xml");
    assert_eq!(xml, std::fs::read_to_string(golden).unwrap());
    let read: RunReport = serde_json::from_str(&json).unwrap();
    assert_eq!(read, run, "a report reads back");
}
