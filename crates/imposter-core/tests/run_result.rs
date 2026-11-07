//! The headless result schema: its exact JSON, its redaction and reading it back.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reading the golden file is the test harness, not core code

use std::path::Path;

use imposter_core::capture::{Captured, Taken};
use imposter_core::check::{Check, CheckResult, Op, Verdict};
use imposter_core::execution::{
    ExecutionRecord, Outcome, Response, ScriptReport, ScriptTest, Timestamp,
};
use imposter_core::model::{BodyType, ParamItem, Request, RequestAuth, RequestTemplate};
use imposter_core::redact::Redactor;
use imposter_core::run_result::{
    Definition, Input, Limits, Meta, ReadError, RunInfo, RunOutcome, RunResult,
};
use imposter_core::select::Selector;
use imposter_core::settings::AppSettings;

const SECRET: &str = "tok-SECRET-42";

fn meta() -> Meta {
    Meta {
        run: RunInfo {
            id: "0b5c7f0e-4f0a-4d55-9a57-2b8f1d2c3e4f".into(),
            tool: "imposter 0.0.0".into(),
            started: Timestamp(1_793_000_000_000).to_rfc3339(),
            elapsed_ms: 15,
        },
        definition: Definition {
            file: "api.imposter.json".into(),
            sha256: "ab".repeat(32),
            request: "Users/Create user".into(),
            environment: Some("Dev".into()),
        },
        inputs: vec![Input {
            variable: "token".into(),
            from: "API_TOKEN".into(),
            secret: true,
        }],
        limits: Limits {
            capture_bytes: 5 * 1024 * 1024,
            excerpt_bytes: 64,
        },
    }
}

/// A run whose secret shows up everywhere a server or script could put it.
fn record() -> ExecutionRecord {
    let template = RequestTemplate {
        name: "Create user".into(),
        method: "POST".into(),
        url: format!("https://ann:pw@api.test/users?key={SECRET}"),
        headers: vec![ParamItem::new("X-Trace", "t-1")],
        auth: RequestAuth::bearer(SECRET),
        body: format!(r#"{{"token":"{SECRET}"}}"#).into_bytes(),
        body_type: BodyType::Json,
        ..RequestTemplate::default()
    };
    ExecutionRecord {
        timestamp: Timestamp(1_793_000_000_000),
        request_id: None,
        template: Request::default(),
        resolved: Request {
            template,
            ..Request::default()
        },
        settings: AppSettings::default(),
        environment_id: None,
        environment: "Dev".into(),
        response: Response {
            status: 201,
            reason: "Created".into(),
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("set-cookie".into(), "sid=abc".into()),
                ("x-echo".into(), SECRET.into()),
            ],
            body: format!(
                r#"{{"id":7,"echo":"{SECRET}","padding":"0123456789abcdefghijklmnopqrstuvwxyz"}}"#
            )
            .into_bytes(),
            elapsed: 0.0123,
            outcome: Outcome::Response,
            checks: vec![
                CheckResult {
                    check: Check::new(Selector::Status, Op::Equals, "201"),
                    verdict: Verdict::Passed,
                    actual: None,
                },
                CheckResult {
                    check: Check::new(Selector::Json("$.echo".into()), Op::Equals, "x"),
                    verdict: Verdict::Failed,
                    actual: Some(SECRET.into()),
                },
            ],
            captures: vec![Captured {
                variable: "echo".into(),
                taken: Taken::Value(SECRET.into()),
                target: "this run".into(),
            }],
            script: Some(ScriptReport {
                tests: vec![ScriptTest {
                    name: "created".into(),
                    passed: true,
                    message: String::new(),
                }],
                error: String::new(),
            }),
            console: vec![format!("printed {SECRET}")],
            ..Response::default()
        },
    }
}

fn redactor() -> Redactor {
    let mut redactor = Redactor::default();
    redactor.secret(SECRET);
    let record = record();
    redactor.credentials(&record.resolved.template, &record.settings);
    redactor
}

#[test]
fn a_sent_result_matches_its_golden_file_with_every_secret_removed() {
    let redactor = redactor();
    let result = RunResult::sent(&record(), meta(), &redactor).redacted(&redactor);
    let json = result.to_json();
    assert!(!json.contains(SECRET), "{json}");
    assert!(!json.contains("ann:pw"), "{json}");
    assert!(!json.contains("sid=abc"), "{json}");
    assert!(
        !json.contains("printed"),
        "the console is never part of a result"
    );
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/results/sent.imposter-result.json");
    assert_eq!(json, std::fs::read_to_string(&golden).unwrap());
    assert_eq!(result.outcome, RunOutcome::Failed);
    assert_eq!(result.outcome.exit_status(), 1);
}

#[test]
fn a_result_reads_back_and_shows_as_a_response() {
    let redactor = redactor();
    let result = RunResult::sent(&record(), meta(), &redactor).redacted(&redactor);
    let read = RunResult::read(result.to_json().as_bytes()).unwrap();
    assert_eq!(read, result);
    let response = read.response().unwrap();
    assert_eq!(response.status, 201);
    assert!(response.truncated, "the excerpt is cut at 64 bytes");
    assert_eq!(response.checks.len(), 2);
    assert!(!String::from_utf8_lossy(&response.body).contains(SECRET));

    let mut newer = serde_json::to_value(&result).unwrap();
    newer["version"] = 2.into();
    assert!(matches!(
        RunResult::read(newer.to_string().as_bytes()),
        Err(ReadError::Version(2))
    ));
    assert!(matches!(
        RunResult::read(br#"{"format": "imposter"}"#),
        Err(ReadError::Json(_))
    ));
}

#[test]
fn unsent_and_failed_runs_say_why() {
    let unsent = RunResult::unsent(
        RunOutcome::NotSent,
        format!("Input token: {SECRET} is too short"),
        meta(),
    )
    .redacted(&redactor());
    assert_eq!(
        unsent.summary,
        "Not sent: Input token: «redacted» is too short"
    );
    assert_eq!(unsent.response(), None);
    assert_eq!(unsent.outcome.exit_status(), 3);

    let mut refused = record();
    refused.response = Response::failure(
        Outcome::TransportError,
        "Connect",
        "Connection refused (os error 61)",
    );
    refused.response.elapsed = 0.003;
    let result = RunResult::sent(&refused, meta(), &redactor());
    assert_eq!(result.outcome, RunOutcome::TransportError);
    assert_eq!(result.summary, "Transport error: Connect · 3 ms");
    assert_eq!(result.error, "Connection refused (os error 61)");
    assert_eq!(result.response, None);
    assert!(result.request.is_some(), "the request it tried is kept");
}
