//! The report of a run of several requests (`imposter run`): one entry per request in the
//! order they ran, each its own redacted [`RunResult`] or skipped with the reason, totals,
//! and one outcome for the whole run. A run passes when no request failed, errored or had a
//! check that could not be decided, and at least one check ran; a request without checks
//! only needs a response. A run with no checks at all is [`RunOutcome::NoChecks`], and one
//! with nothing to run is [`RunOutcome::NotSent`].

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::check::Verdict;
use crate::redact::Redactor;
use crate::run_result::{Definition, Input, Limits, RunInfo, RunOutcome, RunResult};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[serde(rename = "imposter-run")]
    ImposterRun,
}

/// One request of the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Ran(Box<RunResult>),
    /// Not run, because an earlier request did not pass (or the run was cancelled).
    Skipped {
        path: String,
        reason: String,
    },
}

impl Step {
    pub fn path(&self) -> &str {
        match self {
            Self::Ran(result) => &result.definition.request,
            Self::Skipped { path, .. } => path,
        }
    }
}

impl RunOutcome {
    /// Whether a request with this outcome lets the run go on: it got a response that broke
    /// no check, with or without checks of its own.
    pub fn continues_run(self) -> bool {
        matches!(self, Self::Passed | Self::NoChecks)
    }
}

/// Counts over the run; checks include the scripts' tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Totals {
    pub requests: usize,
    /// Requests that got a response and broke no check.
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub checks_passed: usize,
    pub checks_failed: usize,
    pub checks_not_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReport {
    pub format: Format,
    pub version: u32,
    pub summary: String,
    pub outcome: RunOutcome,
    pub run: RunInfo,
    /// What ran; `request` is the target: a collection, a folder or one request.
    pub definition: Definition,
    pub inputs: Vec<Input>,
    pub totals: Totals,
    pub steps: Vec<Step>,
    pub limits: Limits,
}

/// What the caller knows about the run besides its steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub run: RunInfo,
    pub definition: Definition,
    pub inputs: Vec<Input>,
    pub limits: Limits,
}

fn totals(steps: &[Step]) -> Totals {
    let mut totals = Totals {
        requests: steps.len(),
        ..Totals::default()
    };
    for step in steps {
        let Step::Ran(result) = step else {
            totals.skipped += 1;
            continue;
        };
        if result.outcome.continues_run() {
            totals.passed += 1;
        } else {
            totals.failed += 1;
        }
        for check in &result.checks {
            match check.verdict {
                Verdict::Passed => totals.checks_passed += 1,
                Verdict::Failed => totals.checks_failed += 1,
                Verdict::Unchecked(_) => totals.checks_not_checked += 1,
            }
        }
        for test in result.script.iter().flat_map(|s| &s.tests) {
            if test.passed {
                totals.checks_passed += 1;
            } else {
                totals.checks_failed += 1;
            }
        }
    }
    totals
}

/// The run's outcome from its requests' (`None` for a skipped one) and how many checks
/// passed: nothing run is not sent; then the outcome that says least was verified; then
/// `no_checks` when no check passed anywhere. One rule for the command line and the app.
pub fn outcome(outcomes: &[Option<RunOutcome>], checks_passed: usize) -> RunOutcome {
    let ran: Vec<RunOutcome> = outcomes.iter().flatten().copied().collect();
    if ran.is_empty() {
        return RunOutcome::NotSent;
    }
    for worst in [
        RunOutcome::Cancelled,
        RunOutcome::NotSent,
        RunOutcome::TransportError,
        RunOutcome::ScriptFailed,
        RunOutcome::Failed,
        RunOutcome::NotChecked,
    ] {
        if ran.contains(&worst) {
            return worst;
        }
    }
    if checks_passed == 0 {
        RunOutcome::NoChecks
    } else {
        RunOutcome::Passed
    }
}

fn step_outcomes(steps: &[Step]) -> Vec<Option<RunOutcome>> {
    steps
        .iter()
        .map(|step| match step {
            Step::Ran(result) => Some(result.outcome),
            Step::Skipped { .. } => None,
        })
        .collect()
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

fn summary(steps: &[Step], totals: &Totals, outcome: RunOutcome, elapsed_ms: u64) -> String {
    let ran = totals.requests - totals.skipped;
    match outcome {
        RunOutcome::Passed => format!(
            "Passed: {}, {} passed · {elapsed_ms} ms",
            plural(ran, "request"),
            plural(totals.checks_passed, "check")
        ),
        RunOutcome::NoChecks => format!(
            "No checks: {} ran, nothing verified · {elapsed_ms} ms",
            plural(ran, "request")
        ),
        _ => {
            let first = steps.iter().find_map(|step| match step {
                Step::Ran(result) if !result.outcome.continues_run() => Some(result),
                _ => None,
            });
            let Some(result) = first else {
                return "Not sent: nothing to run".to_owned();
            };
            let (word, rest) = result
                .summary
                .split_once(": ")
                .unwrap_or((result.summary.as_str(), ""));
            format!(
                "{word} at {}: {rest} · {ran} of {} ran",
                result.definition.request,
                plural(totals.requests, "request")
            )
        }
    }
}

impl RunReport {
    pub fn new(steps: Vec<Step>, meta: Meta) -> Self {
        let totals = totals(&steps);
        let outcome = outcome(&step_outcomes(&steps), totals.checks_passed);
        Self {
            format: Format::ImposterRun,
            version: VERSION,
            summary: summary(&steps, &totals, outcome, meta.run.elapsed_ms),
            outcome,
            run: meta.run,
            definition: meta.definition,
            inputs: meta.inputs,
            totals,
            steps,
            limits: meta.limits,
        }
    }

    /// A run that could not start (a wrong target, file or input), with the reason.
    pub fn unsent(outcome: RunOutcome, error: &str, meta: Meta) -> Self {
        let mut report = Self::new(Vec::new(), meta);
        report.outcome = outcome;
        report.summary = match outcome {
            RunOutcome::Cancelled => "Cancelled before anything was sent".to_owned(),
            _ => format!("Not sent: {}", error.lines().next().unwrap_or_default()),
        };
        report
    }

    /// Every string through `redactor`, as [`RunResult::redacted`] does.
    ///
    /// # Panics
    /// Never: a report holds only strings, numbers, booleans and string-keyed maps, and
    /// replacing strings keeps every field's type.
    #[must_use]
    pub fn redacted(self, redactor: &Redactor) -> Self {
        let mut value = serde_json::to_value(&self).expect("a report always serializes");
        redactor.json(&mut value);
        serde_json::from_value(value).expect("redacting strings keeps a report's shape")
    }

    /// Pretty JSON with a trailing newline.
    ///
    /// # Panics
    /// Never, as for [`RunReport::redacted`].
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a report always serializes");
        text.push('\n');
        text
    }

    /// JUnit XML: one test case per request. Failed and undecidable checks and failed tests
    /// are a `failure`; a request not sent, a transport error, a script error or a cancel is
    /// an `error`; a request not run is `skipped`. Built from the report as it is, so a
    /// redacted report gives redacted XML.
    pub fn junit(&self) -> String {
        let failures = self
            .steps
            .iter()
            .filter(|s| matches!(s, Step::Ran(r) if matches!(r.outcome, RunOutcome::Failed | RunOutcome::NotChecked)))
            .count();
        let errors = self
            .steps
            .iter()
            .filter(|s| {
                matches!(s, Step::Ran(r) if matches!(
                    r.outcome,
                    RunOutcome::NotSent | RunOutcome::TransportError | RunOutcome::ScriptFailed | RunOutcome::Cancelled
                ))
            })
            .count();
        let seconds = |ms: f64| format!("{:.3}", ms / 1000.0);
        #[allow(clippy::cast_precision_loss)] // milliseconds of a run fit an f64 exactly
        let total = seconds(self.run.elapsed_ms as f64);
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let target = escape(&self.definition.request);
        let counts = format!(
            "tests=\"{}\" failures=\"{failures}\" errors=\"{errors}\" skipped=\"{}\" time=\"{total}\"",
            self.totals.requests, self.totals.skipped
        );
        // Writing to a String cannot fail.
        let _ = writeln!(xml, "<testsuites name=\"imposter run\" {counts}>");
        let _ = writeln!(
            xml,
            "  <testsuite name=\"{target}\" {counts} timestamp=\"{}\">",
            escape(&self.run.started)
        );
        for step in &self.steps {
            let path = step.path();
            let (class, name) = path.rsplit_once('/').unwrap_or(("", path));
            let time = match step {
                Step::Ran(result) => {
                    seconds(result.response.as_ref().map_or(0.0, |r| r.elapsed_ms))
                }
                Step::Skipped { .. } => seconds(0.0),
            };
            let _ = write!(
                xml,
                "    <testcase classname=\"{}\" name=\"{}\" time=\"{time}\"",
                escape(class),
                escape(name)
            );
            match step {
                Step::Skipped { reason, .. } => {
                    let _ = writeln!(
                        xml,
                        ">\n      <skipped message=\"{}\"/>\n    </testcase>",
                        escape(reason)
                    );
                }
                Step::Ran(result) => match result.outcome {
                    RunOutcome::Passed | RunOutcome::NoChecks => xml.push_str("/>\n"),
                    RunOutcome::Failed | RunOutcome::NotChecked => {
                        let _ = writeln!(
                            xml,
                            ">\n      <failure message=\"{}\">{}</failure>\n    </testcase>",
                            escape(&result.summary),
                            escape(&failure_lines(result))
                        );
                    }
                    RunOutcome::NotSent
                    | RunOutcome::TransportError
                    | RunOutcome::ScriptFailed
                    | RunOutcome::Cancelled => {
                        let _ = writeln!(
                            xml,
                            ">\n      <error message=\"{}\">{}</error>\n    </testcase>",
                            escape(&result.summary),
                            escape(&result.error)
                        );
                    }
                },
            }
        }
        xml.push_str("  </testsuite>\n</testsuites>\n");
        xml
    }
}

/// One line per check or test that did not pass.
fn failure_lines(result: &RunResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    for check in &result.checks {
        match &check.verdict {
            Verdict::Passed => {}
            Verdict::Failed => lines.push(match &check.actual {
                Some(actual) => format!("{}: got {actual}", check.check.describe()),
                None => format!("{}: failed", check.check.describe()),
            }),
            Verdict::Unchecked(reason) => {
                lines.push(format!("{}: not checked, {reason}", check.check.describe()));
            }
        }
    }
    for test in result.script.iter().flat_map(|s| &s.tests) {
        if !test.passed {
            lines.push(if test.message.is_empty() {
                format!("Script test {:?} failed", test.name)
            } else {
                format!("Script test {:?}: {}", test.name, test.message)
            });
        }
    }
    lines.join("\n")
}

/// Text and attribute values for XML.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no way to carry other control characters.
            c if c.is_control() && !matches!(c, '\n' | '\t' | '\r') => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pass_rule_works_on_outcomes_alone() {
        use RunOutcome as O;
        assert_eq!(outcome(&[], 0), O::NotSent);
        assert_eq!(outcome(&[None, None], 0), O::NotSent, "all skipped");
        assert_eq!(outcome(&[Some(O::NoChecks), Some(O::Passed)], 1), O::Passed);
        assert_eq!(outcome(&[Some(O::NoChecks)], 0), O::NoChecks);
        assert_eq!(
            outcome(&[Some(O::Failed), Some(O::TransportError), None], 0),
            O::TransportError
        );
        assert_eq!(
            outcome(&[Some(O::NotChecked), Some(O::Passed)], 3),
            O::NotChecked
        );
    }

    #[test]
    fn xml_escapes_markup_and_drops_control_characters() {
        assert_eq!(
            escape("a<b & \"c\" 'd'>\u{1}\n"),
            "a&lt;b &amp; &quot;c&quot; &apos;d&apos;&gt;\u{fffd}\n"
        );
    }
}
