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
use imposter_app::{paths, profile};
use imposter_core::execution::{ExecutionRecord, Via};
use imposter_core::model::Variables;
use imposter_core::redact::{MIN_SECRET, Redactor};
use imposter_core::run_result::{
    Definition, Input, Limits, Meta, RunInfo, RunOutcome, RunResult, Source,
};
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
    /// A native collection file to read instead of the personal profile.
    pub file: Option<PathBuf>,
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
            file: None,
            request: String::new(),
            environment: None,
            inputs: Vec::new(),
            out: None,
            max_body: DEFAULT_MAX_BODY,
        }
    }
}

/// Why a run sent nothing, before the engine ran.
pub(crate) struct Unsent(pub(crate) RunOutcome, pub(crate) String);

impl From<String> for Unsent {
    fn from(message: String) -> Self {
        Self(RunOutcome::NotSent, message)
    }
}

/// Read every input before anything is sent; a missing one stops the run.
pub(crate) fn read_inputs(
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
        source: if args.file.is_some() {
            Source::File
        } else {
            Source::Profile
        },
        file: args.file.as_ref().map(|f| f.display().to_string()),
        sha256: None,
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
        Ok((record, capture_bytes, notes)) => {
            redactor.credentials(&record.resolved.template, &record.settings);
            let mut result = RunResult::sent(
                &record,
                Meta {
                    limits: Limits {
                        capture_bytes,
                        ..meta.limits
                    },
                    ..meta
                },
                &redactor,
            );
            result.notes = notes;
            result
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

/// SHA-256 of a file's bytes in hex: the revision of the definitions that ran.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// What a run produced: its record, the capture limit it ran under, and notes.
type Ran = (ExecutionRecord, u64, Vec<String>);

fn unsent(error: ExecuteError) -> Unsent {
    match error {
        ExecuteError::Script(message) => Unsent(RunOutcome::ScriptFailed, message),
        other => Unsent(RunOutcome::NotSent, other.to_string()),
    }
}

async fn run(
    args: &SendArgs,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<Ran, Unsent> {
    let path = RequestPath::parse(&args.request).map_err(|e| e.to_string())?;
    match &args.file {
        Some(file) => run_file(args, file, &path, var, cancel, redactor, definition).await,
        None => run_profile(args, &path, var, cancel, redactor).await,
    }
}

async fn run_file(
    args: &SendArgs,
    file: &std::path::Path,
    path: &RequestPath,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<Ran, Unsent> {
    let file = file.to_owned();
    // Read once: an edit to the file during the run cannot change it.
    let bytes = tokio::task::spawn_blocking(move || native::read_document(&file))
        .await
        .map_err(|_| "The file could not be read".to_owned())?
        .map_err(|e| e.to_string())?;
    definition.sha256 = Some(sha256_hex(&bytes));
    let document = native::decode(&bytes).map_err(|e| e.to_string())?;
    let values = read_inputs(&args.inputs, var, redactor)?;
    let selected = document::select(&document, path, args.environment.as_deref(), &values)
        .map_err(|e| e.to_string())?;
    let capture_bytes = selected.inputs.settings.download_limit.saturating_mul(1024);
    let ran = executor::run(&selected.inputs, selected.request, cancel)
        .await
        .map_err(unsent)?;
    Ok((ran.record, capture_bytes, Vec::new()))
}

/// A saved request from the personal profile: read in one database command, run, and
/// recorded in History as the command line's. Captures stay in the run.
async fn run_profile(
    args: &SendArgs,
    path: &RequestPath,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
) -> Result<Ran, Unsent> {
    let values = read_inputs(&args.inputs, var, redactor)?;
    let explicit = var(paths::DATA_DIR_VARIABLE).map(std::ffi::OsString::from);
    let store = tokio::task::spawn_blocking(move || {
        paths::open_existing_profile(explicit, std::env::home_dir())
    })
    .await
    .map_err(|_| "The profile could not be opened".to_owned())?
    .map_err(|e| e.to_string())?;
    let handle = store.handle();
    let outcome = async {
        let selected = profile::select(
            &handle,
            path,
            args.environment.as_deref(),
            &values,
            profile::Purpose::Run,
        )
        .await
        .map_err(|e| e.to_string())?;
        let capture_bytes = selected.inputs.settings.download_limit.saturating_mul(1024);
        let mut ran = executor::run(&selected.inputs, selected.request, cancel)
            .await
            .map_err(unsent)?;
        ran.record.via = Via::CommandLine;
        let record = ran.record.clone();
        let executed = executor::record(&handle, ran).await;
        let notes = if executed.record.is_none() {
            vec![executed.response.history_error]
        } else {
            Vec::new()
        };
        Ok((record, capture_bytes, notes))
    }
    .await;
    // Closing waits for the database thread to finish its last write.
    let _ = tokio::task::spawn_blocking(move || store.close()).await;
    outcome
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
    for note in &result.notes {
        let _ = writeln!(stderr, "note: {note}");
    }
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
