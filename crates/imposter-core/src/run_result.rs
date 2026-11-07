//! The result of a headless run: one schema for the command line's JSON, a result file and
//! the desktop's view of it. It leads with a summary and an outcome, records what was run
//! (file revision, request path, environment, input names), and keeps the observed
//! exchange bounded. Secrets are removed before it leaves the process ([`RunResult::redacted`]).

use serde::{Deserialize, Serialize};

use crate::capture::Captured;
use crate::check::{CheckResult, Verdict};
use crate::execution::{ExecutionRecord, Outcome, Response, ScriptReport};
use crate::prepare::{PreparedBody, prepare};
use crate::redact::Redactor;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[serde(rename = "imposter-result")]
    ImposterResult,
}

/// How a run ended. Each has its own exit status, so a caller can tell them apart without
/// reading the JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// A response arrived and every check and script test passed.
    Passed,
    /// A check or a script test failed.
    Failed,
    /// No check failed, but at least one could not be decided (a cut body, a bad path).
    NotChecked,
    /// A response arrived, but nothing checked it: an execution, not verified behavior.
    NoChecks,
    /// Nothing was sent: the file, the request path, an input or the request was wrong.
    NotSent,
    /// No response: the connection, TLS or a timeout failed.
    TransportError,
    /// A script failed or was stopped: before sending (nothing went out) or after the
    /// response.
    ScriptFailed,
    Cancelled,
}

impl RunOutcome {
    /// Every outcome with its exit status, in the order the help lists them.
    pub const ALL: [Self; 8] = [
        Self::Passed,
        Self::Failed,
        Self::NotSent,
        Self::TransportError,
        Self::ScriptFailed,
        Self::NotChecked,
        Self::NoChecks,
        Self::Cancelled,
    ];

    /// The process exit status. 2 is left for usage errors, which have no result; 130 is
    /// the shell's status for an interrupt.
    pub fn exit_status(self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::Failed => 1,
            Self::NotSent => 3,
            Self::TransportError => 4,
            Self::ScriptFailed => 5,
            Self::NotChecked => 6,
            Self::NoChecks => 7,
            Self::Cancelled => 130,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotChecked => "not_checked",
            Self::NoChecks => "no_checks",
            Self::NotSent => "not_sent",
            Self::TransportError => "transport_error",
            Self::ScriptFailed => "script_failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// What a response says about the run. When several apply, the one that says the least
    /// was verified wins: a cancel, then a transport error, a script failure, a failed
    /// check, an undecided one, no checks at all.
    pub fn of(response: &Response) -> Self {
        match response.outcome {
            Outcome::Cancelled => return Self::Cancelled,
            Outcome::TransportError => return Self::TransportError,
            Outcome::Response => {}
        }
        let tests = response
            .script
            .as_ref()
            .map_or(&[][..], |s| s.tests.as_slice());
        if response
            .script
            .as_ref()
            .is_some_and(|s| !s.error.is_empty())
        {
            Self::ScriptFailed
        } else if response.checks.iter().any(|c| c.verdict == Verdict::Failed)
            || tests.iter().any(|t| !t.passed)
        {
            Self::Failed
        } else if response
            .checks
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Unchecked(_)))
        {
            Self::NotChecked
        } else if response.checks.is_empty() && tests.is_empty() {
            Self::NoChecks
        } else {
            Self::Passed
        }
    }
}

/// Where the response came from. Only live runs exist so far; a mock's answer will say so,
/// since it does not verify the live API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunInfo {
    pub id: String,
    /// `imposter 0.6.3`.
    pub tool: String,
    pub started: String,
    pub elapsed_ms: u64,
}

/// Where the definitions that ran were read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The personal profile the app uses.
    Profile,
    /// A native collection file (`--file`).
    File,
}

/// What ran: where it was read, the request and the environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub source: Source,
    /// The file as given, for a file run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// SHA-256 of the file's bytes, in hex, once the file was read: the revision that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub request: String,
    /// `None`: globals and collection variables only.
    pub environment: Option<String>,
}

/// A value given to the run, by name only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub variable: String,
    /// The process variable it was read from.
    pub from: String,
    pub secret: bool,
}

/// The start of a body, as text. A body that is not UTF-8 text is described, not shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Excerpt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Bytes the body had (as captured, for a response).
    pub bytes: usize,
    /// The excerpt is shorter than the body.
    pub cut: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Excerpt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseHead {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub elapsed_ms: f64,
    /// The body was cut at the capture limit when it arrived.
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Excerpt>,
}

/// The bounds this result was produced under, so a reader knows what may be missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Bytes of response body captured; 0 captures everything.
    pub capture_bytes: u64,
    /// Bytes of each body shown in the result; 0 shows none.
    pub excerpt_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub format: Format,
    pub version: u32,
    pub summary: String,
    pub outcome: RunOutcome,
    pub run: RunInfo,
    pub definition: Definition,
    pub inputs: Vec<Input>,
    pub origin: Origin,
    /// Why nothing was sent, why the exchange failed, or why a script stopped.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<Captured>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<SentRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseHead>,
    pub limits: Limits,
    /// What else the reader should know, such as a History write that failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// What the caller knows about a run besides its record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub run: RunInfo,
    pub definition: Definition,
    pub inputs: Vec<Input>,
    pub limits: Limits,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("Not an Imposter result: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Result version {0} is not supported; this app reads version 1")]
    Version(u32),
}

/// The first `limit` bytes of `body` as text, redacted before the cut so a secret that
/// straddles it cannot leave its first half behind.
fn excerpt(body: &[u8], limit: usize, redactor: &Redactor) -> Option<Excerpt> {
    if limit == 0 {
        return None;
    }
    let text = match std::str::from_utf8(body) {
        Ok(text) => Some(text),
        // A capture limit can split a character; up to three bytes short is still text.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&body[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
    .filter(|text| !text.contains('\0'));
    let Some(text) = text else {
        return Some(Excerpt {
            text: None,
            bytes: body.len(),
            cut: false,
            binary: true,
        });
    };
    let redacted = redactor.text(text);
    let mut end = redacted.len().min(limit);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    Some(Excerpt {
        text: Some(redacted[..end].to_owned()),
        bytes: body.len(),
        cut: end < redacted.len(),
        binary: false,
    })
}

fn milliseconds(seconds: f64) -> f64 {
    (seconds * 1000.0 * 10.0).round() / 10.0
}

fn summary(outcome: RunOutcome, response: Option<&Response>, error: &str) -> String {
    let head = |r: &Response| {
        let status = if r.reason.is_empty() {
            r.status.to_string()
        } else {
            format!("{} {}", r.status, r.reason)
        };
        format!("{status} · {} ms", milliseconds(r.elapsed).round())
    };
    let first_line = error.lines().next().unwrap_or_default();
    let Some(response) = response else {
        return match outcome {
            RunOutcome::Cancelled => "Cancelled before anything was sent".to_owned(),
            RunOutcome::ScriptFailed => format!("Script failed, nothing sent: {first_line}"),
            _ => format!("Not sent: {first_line}"),
        };
    };
    let tests = response
        .script
        .as_ref()
        .map_or(&[][..], |s| s.tests.as_slice());
    let total = response.checks.len() + tests.len();
    let failed = response
        .checks
        .iter()
        .filter(|c| c.verdict == Verdict::Failed)
        .count()
        + tests.iter().filter(|t| !t.passed).count();
    let undecided = response
        .checks
        .iter()
        .filter(|c| matches!(c.verdict, Verdict::Unchecked(_)))
        .count();
    match outcome {
        RunOutcome::Passed => format!("Passed: {total} of {total} checks · {}", head(response)),
        RunOutcome::Failed => {
            format!(
                "Failed: {failed} of {total} checks failed · {}",
                head(response)
            )
        }
        RunOutcome::NotChecked => format!(
            "Not checked: {undecided} of {total} checks could not be decided · {}",
            head(response)
        ),
        RunOutcome::NoChecks => format!("No checks: nothing verified · {}", head(response)),
        RunOutcome::ScriptFailed => format!("Script failed: {first_line} · {}", head(response)),
        RunOutcome::TransportError => format!(
            "Transport error: {} · {} ms",
            response.error_kind,
            milliseconds(response.elapsed).round()
        ),
        RunOutcome::Cancelled => "Cancelled".to_owned(),
        RunOutcome::NotSent => format!("Not sent: {first_line}"),
    }
}

impl RunResult {
    /// The result of a run that sent its request (or was cancelled or failed on the way).
    /// Credential headers are already masked here; [`RunResult::redacted`] removes the rest.
    pub fn sent(record: &ExecutionRecord, meta: Meta, redactor: &Redactor) -> Self {
        let response = &record.response;
        let outcome = RunOutcome::of(response);
        let limit = meta.limits.excerpt_bytes;
        let request = prepare(&record.resolved.template, record.settings.attach_cookies)
            .ok()
            .map(|prepared| SentRequest {
                method: prepared.method,
                url: prepared.url,
                headers: prepared
                    .headers
                    .iter()
                    .map(|(n, v)| redactor.header(n, v))
                    .collect(),
                body: match &prepared.body {
                    PreparedBody::Bytes(bytes) if !bytes.is_empty() => {
                        excerpt(bytes, limit, redactor)
                    }
                    PreparedBody::Bytes(_) | PreparedBody::Multipart(_) | PreparedBody::File(_) => {
                        None
                    }
                },
            });
        let error = match (outcome, &response.script) {
            (RunOutcome::TransportError | RunOutcome::Cancelled, _) => response.reason.clone(),
            (_, Some(script)) => script.error.clone(),
            _ => String::new(),
        };
        let head = (response.outcome == Outcome::Response).then(|| ResponseHead {
            status: response.status,
            reason: response.reason.clone(),
            headers: response
                .headers
                .iter()
                .map(|(n, v)| redactor.header(n, v))
                .collect(),
            elapsed_ms: milliseconds(response.elapsed),
            truncated: response.truncated,
            body: excerpt(&response.body, limit, redactor),
        });
        Self {
            format: Format::ImposterResult,
            version: VERSION,
            summary: summary(outcome, Some(response), &error),
            outcome,
            run: meta.run,
            definition: meta.definition,
            inputs: meta.inputs,
            origin: Origin::Live,
            error,
            checks: response.checks.clone(),
            script: response.script.clone(),
            captures: response.captures.clone(),
            request,
            response: head,
            limits: meta.limits,
            notes: Vec::new(),
        }
    }

    /// The result of a run that sent nothing: `outcome` is `NotSent`, `ScriptFailed` (the
    /// before-sending script) or `Cancelled`.
    pub fn unsent(outcome: RunOutcome, error: impl Into<String>, meta: Meta) -> Self {
        let error = error.into();
        Self {
            format: Format::ImposterResult,
            version: VERSION,
            summary: summary(outcome, None, &error),
            outcome,
            run: meta.run,
            definition: meta.definition,
            inputs: meta.inputs,
            origin: Origin::Live,
            error,
            checks: Vec::new(),
            script: None,
            captures: Vec::new(),
            request: None,
            response: None,
            limits: meta.limits,
            notes: Vec::new(),
        }
    }

    /// Every string run through `redactor`: URLs, headers, bodies, check values, script
    /// errors, capture values, the summary, even names.
    ///
    /// # Panics
    /// Never: a result holds only strings, numbers, booleans and string-keyed maps, and
    /// replacing strings keeps every field's type.
    #[must_use]
    pub fn redacted(self, redactor: &Redactor) -> Self {
        let mut value = serde_json::to_value(&self).expect("a result always serializes");
        redactor.json(&mut value);
        serde_json::from_value(value).expect("redacting strings keeps a result's shape")
    }

    /// Pretty JSON with a trailing newline.
    ///
    /// # Panics
    /// Never, as for [`RunResult::redacted`].
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a result always serializes");
        text.push('\n');
        text
    }

    pub fn read(data: &[u8]) -> Result<Self, ReadError> {
        let result: Self = serde_json::from_slice(data)?;
        if result.version != VERSION {
            return Err(ReadError::Version(result.version));
        }
        Ok(result)
    }

    /// The response as the response viewer shows it: the head, the excerpt as its body (cut
    /// when the excerpt is), checks, captures and script tests.
    pub fn response(&self) -> Option<Response> {
        let head = self.response.as_ref()?;
        let body = head.body.as_ref();
        Some(Response {
            status: head.status,
            reason: head.reason.clone(),
            headers: head.headers.clone(),
            body: body
                .and_then(|b| b.text.clone())
                .unwrap_or_default()
                .into_bytes(),
            elapsed: head.elapsed_ms / 1000.0,
            outcome: Outcome::Response,
            truncated: head.truncated || body.is_none_or(|b| b.cut || b.binary),
            checks: self.checks.clone(),
            captures: self.captures.clone(),
            script: self.script.clone(),
            ..Response::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Check, Op};
    use crate::execution::ScriptTest;
    use crate::select::Selector;

    fn check(verdict: Verdict) -> CheckResult {
        CheckResult {
            check: Check::new(Selector::Status, Op::Equals, "200"),
            verdict,
            actual: None,
        }
    }

    fn with(checks: Vec<Verdict>, tests: &[bool], script_error: &str) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            checks: checks.into_iter().map(check).collect(),
            script: (!tests.is_empty() || !script_error.is_empty()).then(|| ScriptReport {
                tests: tests
                    .iter()
                    .map(|passed| ScriptTest {
                        name: "t".into(),
                        passed: *passed,
                        message: String::new(),
                    })
                    .collect(),
                error: script_error.into(),
            }),
            ..Response::default()
        }
    }

    #[test]
    fn outcomes_say_the_least_that_was_verified() {
        use RunOutcome as O;
        let unchecked = || Verdict::Unchecked("cut".into());
        assert_eq!(O::of(&with(vec![Verdict::Passed], &[true], "")), O::Passed);
        assert_eq!(
            O::of(&with(vec![], &[true], "")),
            O::Passed,
            "a script test counts"
        );
        assert_eq!(O::of(&with(vec![], &[], "")), O::NoChecks);
        assert_eq!(O::of(&with(vec![unchecked()], &[], "")), O::NotChecked);
        assert_eq!(
            O::of(&with(vec![unchecked(), Verdict::Failed], &[], "")),
            O::Failed
        );
        assert_eq!(O::of(&with(vec![Verdict::Passed], &[false], "")), O::Failed);
        assert_eq!(
            O::of(&with(
                vec![Verdict::Failed],
                &[],
                "After-response script, line 1: x"
            )),
            O::ScriptFailed
        );
        let failed = Response::failure(Outcome::TransportError, "Connect", "refused");
        assert_eq!(O::of(&failed), O::TransportError);
        let cancelled = Response::failure(Outcome::Cancelled, "Cancelled", "Request cancelled");
        assert_eq!(O::of(&cancelled), O::Cancelled);
    }

    #[test]
    fn every_outcome_has_its_own_exit_status() {
        let mut statuses: Vec<u8> = RunOutcome::ALL.iter().map(|o| o.exit_status()).collect();
        assert!(!statuses.contains(&2), "2 is for usage errors");
        statuses.sort_unstable();
        statuses.dedup();
        assert_eq!(statuses.len(), RunOutcome::ALL.len());
        assert_eq!(RunOutcome::Passed.exit_status(), 0);
        for outcome in RunOutcome::ALL {
            assert_eq!(
                serde_json::to_value(outcome).unwrap(),
                serde_json::Value::from(outcome.as_str())
            );
        }
    }

    #[test]
    fn excerpts_keep_text_and_describe_bytes() {
        let none = Redactor::default();
        assert_eq!(excerpt(b"hello", 0, &none), None);
        let cut = excerpt("héllo".as_bytes(), 2, &none).unwrap();
        assert_eq!(
            (cut.text.as_deref(), cut.cut, cut.binary, cut.bytes),
            (Some("h"), true, false, 6),
            "a character split by the cut is left out"
        );
        let binary = excerpt(&[0x89, b'P', b'N', b'G', 0, 1], 64, &none).unwrap();
        assert_eq!(
            (binary.text, binary.binary, binary.cut),
            (None, true, false)
        );
        let captured_short = excerpt(&"aé".as_bytes()[..2], 64, &none).unwrap();
        assert_eq!(captured_short.text.as_deref(), Some("a"));
    }

    #[test]
    fn a_secret_across_the_cut_leaves_nothing_behind() {
        let mut redactor = Redactor::default();
        redactor.secret("SECRETVALUE");
        let shown = excerpt(b"0123456789SECRETVALUE tail", 14, &redactor).unwrap();
        let text = shown.text.unwrap();
        assert!(!text.contains("SECR"), "{text}");
        assert!(shown.cut);
    }
}
