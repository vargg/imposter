//! `imposter send`: read the file once, choose the request, read only the named process
//! variables, run it through the app's engine, and report a redacted result. Nothing is
//! read from or written to the personal profile, and nothing goes to History.

use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use imposter_app::document::{self, RequestPath};
use imposter_app::executor::{self, ExecuteError, SystemEntropy};
use imposter_app::formats::native;
use imposter_core::model::Variables;
use imposter_core::redact::{MIN_SECRET, Redactor};
use imposter_core::run_result::{Definition, Input, Limits, Meta, RunInfo, RunOutcome, RunResult};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use crate::USAGE_STATUS;

/// A variable set from a named process variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputArg {
    pub variable: String,
    pub from: String,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendArgs {
    pub file: PathBuf,
    pub request: String,
    pub environment: Option<String>,
    pub inputs: Vec<InputArg>,
    pub out: Option<PathBuf>,
    /// Bytes of each body shown in the result.
    pub max_body: usize,
}

pub const DEFAULT_MAX_BODY: usize = 64 * 1024;

impl Default for SendArgs {
    fn default() -> Self {
        Self {
            file: PathBuf::new(),
            request: String::new(),
            environment: None,
            inputs: Vec::new(),
            out: None,
            max_body: DEFAULT_MAX_BODY,
        }
    }
}

/// Why a run sent nothing, before the engine ran.
struct Unsent(RunOutcome, String);

impl From<String> for Unsent {
    fn from(message: String) -> Self {
        Self(RunOutcome::NotSent, message)
    }
}

/// Read every input before anything is sent; a missing one stops the run.
fn read_inputs(
    inputs: &[InputArg],
    var: &dyn Fn(&str) -> Option<String>,
    redactor: &mut Redactor,
) -> Result<Variables, Unsent> {
    let mut values = Variables::new();
    for input in inputs {
        let value = var(&input.from).filter(|v| !v.is_empty()).ok_or_else(|| {
            format!(
                "Input {}: the process variable {} is not set",
                input.variable, input.from
            )
        })?;
        if input.secret {
            if value.chars().count() < MIN_SECRET {
                return Err(format!(
                    "Secret {}: the value of {} is shorter than {MIN_SECRET} characters, too short to remove from the output reliably",
                    input.variable, input.from
                )
                .into());
            }
            redactor.secret(&value);
        }
        values.insert(input.variable.clone(), value);
    }
    Ok(values)
}

/// Run `args`. Every way a run can end gives a result; none panics or exits.
pub async fn send(
    args: &SendArgs,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
) -> RunResult {
    let clock = Instant::now();
    let started = executor::now();
    let mut entropy = SystemEntropy::default();
    let id = imposter_core::generate::generate("$uuid", &mut entropy).unwrap_or_default();
    let mut redactor = Redactor::default();
    let mut definition = Definition {
        file: args.file.display().to_string(),
        sha256: String::new(),
        request: args.request.clone(),
        environment: args.environment.clone(),
    };
    let outcome = run(args, var, cancel.clone(), &mut redactor, &mut definition).await;
    let meta = Meta {
        run: RunInfo {
            id,
            tool: crate::version(),
            started: started.to_rfc3339(),
            elapsed_ms: u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
        definition,
        inputs: args
            .inputs
            .iter()
            .map(|input| Input {
                variable: input.variable.clone(),
                from: input.from.clone(),
                secret: input.secret,
            })
            .collect(),
        limits: Limits {
            capture_bytes: 0,
            excerpt_bytes: args.max_body,
        },
    };
    let result = match outcome {
        Ok((record, capture_bytes)) => {
            redactor.credentials(&record.resolved.template, &record.settings);
            RunResult::sent(
                &record,
                Meta {
                    limits: Limits {
                        capture_bytes,
                        ..meta.limits
                    },
                    ..meta
                },
                &redactor,
            )
        }
        Err(Unsent(outcome, message)) => {
            // A stop asked for by the user is a cancel, whatever step it interrupted.
            let outcome = if cancel.is_cancelled() {
                RunOutcome::Cancelled
            } else {
                outcome
            };
            RunResult::unsent(outcome, message, meta)
        }
    };
    result.redacted(&redactor)
}

async fn run(
    args: &SendArgs,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<(imposter_core::execution::ExecutionRecord, u64), Unsent> {
    let path = RequestPath::parse(&args.request).map_err(|e| e.to_string())?;
    let file = args.file.clone();
    // Read once: an edit to the file during the run cannot change it.
    let bytes = tokio::task::spawn_blocking(move || native::read_document(&file))
        .await
        .map_err(|_| "The file could not be read".to_owned())?
        .map_err(|e| e.to_string())?;
    definition.sha256 =
        Sha256::digest(&bytes)
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                // Writing to a String cannot fail.
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    let document = native::decode(&bytes).map_err(|e| e.to_string())?;
    let values = read_inputs(&args.inputs, var, redactor)?;
    let selected = document::select(&document, &path, args.environment.as_deref(), &values)
        .map_err(|e| e.to_string())?;
    let capture_bytes = selected.inputs.settings.download_limit.saturating_mul(1024);
    let ran = executor::run(&selected.inputs, selected.request, cancel)
        .await
        .map_err(|error| match error {
            ExecuteError::Script(message) => Unsent(RunOutcome::ScriptFailed, message),
            other => Unsent(RunOutcome::NotSent, other.to_string()),
        })?;
    Ok((ran.record, capture_bytes))
}

/// Write the result: JSON on stdout, the summary on stderr, and the file `--out` names.
/// Returns the exit status.
pub fn report(
    args: &SendArgs,
    result: &RunResult,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let json = result.to_json();
    // A closed pipe cannot be reported anywhere else.
    let _ = stdout.write_all(json.as_bytes());
    let _ = writeln!(stderr, "{}", result.summary);
    if let Some(out) = &args.out
        && let Err(error) = native::atomic_write(out, json.as_bytes())
    {
        let _ = writeln!(
            stderr,
            "error: the result could not be written to {}: {error}",
            out.display()
        );
        return USAGE_STATUS;
    }
    result.outcome.exit_status()
}
