//! `imposter run`: a folder or a collection as one run, through the app's runner. Progress
//! goes to stderr one line per request as it ends; the report goes to stdout, `--out` and,
//! as JUnit XML, `--junit`. Everything printed or written is redacted.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use imposter_app::document::{self, RequestPath, Selected};
use imposter_app::executor::{self, SystemEntropy};
use imposter_app::formats::native;
use imposter_app::runner::{self, Options};
use imposter_app::{paths, profile};
use imposter_core::redact::Redactor;
use imposter_core::run_report::{self, RunReport, Step};
use imposter_core::run_result::{
    self, Definition, Input, Limits, RunInfo, RunOutcome, RunResult, Source,
};
use tokio_util::sync::CancellationToken;

use crate::USAGE_STATUS;
use crate::send::{SendArgs, Unsent, read_inputs, sha256_hex};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunArgs {
    /// The target in `request`, and the options it shares with `send`.
    pub send: SendArgs,
    pub keep_going: bool,
    pub junit: Option<PathBuf>,
}

/// What the definitions were read from.
enum Requests {
    File(Vec<Selected>),
    Profile(imposter_app::store::Store, Vec<Selected>),
}

/// The requests to run, read once at the start.
async fn select(
    args: &SendArgs,
    var: &dyn Fn(&str) -> Option<String>,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<Requests, Unsent> {
    let target = RequestPath::parse(&args.request).map_err(|e| e.to_string())?;
    if let Some(file) = &args.file {
        let file = file.clone();
        let bytes = tokio::task::spawn_blocking(move || native::read_document(&file))
            .await
            .map_err(|_| "The file could not be read".to_owned())?
            .map_err(|e| e.to_string())?;
        definition.sha256 = Some(sha256_hex(&bytes));
        let document = native::decode(&bytes).map_err(|e| e.to_string())?;
        let values = read_inputs(&args.inputs, var, redactor)?;
        let selected =
            document::select_all(&document, &target, args.environment.as_deref(), &values)
                .map_err(|e| e.to_string())?;
        return Ok(Requests::File(selected));
    }
    let values = read_inputs(&args.inputs, var, redactor)?;
    let explicit = var(paths::DATA_DIR_VARIABLE).map(std::ffi::OsString::from);
    let store = tokio::task::spawn_blocking(move || {
        paths::open_existing_profile(explicit, std::env::home_dir())
    })
    .await
    .map_err(|_| "The profile could not be opened".to_owned())?
    .map_err(|e| e.to_string())?;
    match profile::select_all(
        &store.handle(),
        &target,
        args.environment.as_deref(),
        &values,
    )
    .await
    {
        Ok(selected) => Ok(Requests::Profile(store, selected)),
        Err(error) => {
            let _ = tokio::task::spawn_blocking(move || store.close()).await;
            Err(error.to_string().into())
        }
    }
}

fn milliseconds(clock: Instant) -> u64 {
    u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// One line for stderr as a request ends.
fn progress(step: &Step) -> String {
    match step {
        Step::Ran(result) => {
            let word = if result.outcome.continues_run() {
                "ok     "
            } else {
                "FAILED "
            };
            format!("{word} {} · {}", result.definition.request, result.summary)
        }
        Step::Skipped { path, reason } => format!("skipped {path} ({reason})"),
    }
}

/// Run `args` and return the report. `stderr` gets a line per request as it ends.
pub async fn run(
    args: &RunArgs,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    stderr: &mut dyn Write,
) -> RunReport {
    let clock = Instant::now();
    let started = executor::now().to_rfc3339();
    let mut entropy = SystemEntropy::default();
    let mut uuid = || imposter_core::generate::generate("$uuid", &mut entropy).unwrap_or_default();
    let run_id = uuid();
    let mut redactor = Redactor::default();
    let mut definition = Definition {
        source: if args.send.file.is_some() {
            Source::File
        } else {
            Source::Profile
        },
        file: args.send.file.as_ref().map(|f| f.display().to_string()),
        sha256: None,
        request: args.send.request.clone(),
        environment: args.send.environment.clone(),
    };
    let inputs: Vec<Input> = args
        .send
        .inputs
        .iter()
        .map(|input| Input {
            variable: input.variable.clone(),
            from: input.from.clone(),
            secret: input.secret,
        })
        .collect();
    let limits = Limits {
        capture_bytes: 0,
        excerpt_bytes: args.send.max_body,
    };
    let selected = select(&args.send, var, &mut redactor, &mut definition).await;
    let meta = |definition: Definition, elapsed_ms| run_report::Meta {
        run: RunInfo {
            id: run_id.clone(),
            tool: crate::version(),
            started: started.clone(),
            elapsed_ms,
        },
        definition,
        inputs: inputs.clone(),
        limits,
    };
    let (requests, store) = match selected {
        Ok(Requests::File(requests)) => (requests, None),
        Ok(Requests::Profile(store, requests)) => (requests, Some(store)),
        Err(Unsent(outcome, message)) => {
            let outcome = if cancel.is_cancelled() {
                RunOutcome::Cancelled
            } else {
                outcome
            };
            return RunReport::unsent(outcome, &message, meta(definition, milliseconds(clock)))
                .redacted(&redactor);
        }
    };
    let capture_bytes = requests
        .first()
        .map_or(0, |r| r.inputs.settings.download_limit.saturating_mul(1024));
    let mut steps: Vec<Step> = Vec::with_capacity(requests.len());
    let step_definition = |path: &str| Definition {
        request: path.to_owned(),
        ..definition.clone()
    };
    let options = Options {
        keep_going: args.keep_going,
        record: store.as_ref().map(imposter_app::store::Store::handle),
    };
    runner::run(requests, options, cancel, |ended| {
        let step = match ended {
            runner::Step::Ran {
                path,
                record,
                notes,
            } => {
                redactor.credentials(&record.resolved.template, &record.settings);
                let mut result = RunResult::sent(
                    record,
                    run_result::Meta {
                        run: RunInfo {
                            id: uuid(),
                            tool: crate::version(),
                            started: record.timestamp.to_rfc3339(),
                            elapsed_ms: 0,
                        },
                        definition: step_definition(path),
                        inputs: inputs.clone(),
                        limits: Limits {
                            capture_bytes,
                            ..limits
                        },
                    },
                    &redactor,
                );
                result.notes.clone_from(notes);
                Step::Ran(Box::new(result.redacted(&redactor)))
            }
            runner::Step::NotSent {
                path,
                outcome,
                error,
            } => Step::Ran(Box::new(
                RunResult::unsent(
                    *outcome,
                    error.clone(),
                    run_result::Meta {
                        run: RunInfo {
                            id: uuid(),
                            tool: crate::version(),
                            started: executor::now().to_rfc3339(),
                            elapsed_ms: 0,
                        },
                        definition: step_definition(path),
                        inputs: inputs.clone(),
                        limits,
                    },
                )
                .redacted(&redactor),
            )),
            runner::Step::Skipped { path, reason } => Step::Skipped {
                path: path.clone(),
                reason: reason.clone(),
            },
        };
        let _ = writeln!(stderr, "{}", redactor.text(&progress(&step)));
        steps.push(step);
    })
    .await;
    if let Some(store) = store {
        // Closing waits for the database thread to finish its last History write.
        let _ = tokio::task::spawn_blocking(move || store.close()).await;
    }
    let mut definition = definition.clone();
    definition.request.clone_from(&args.send.request);
    RunReport::new(steps, meta(definition, milliseconds(clock))).redacted(&redactor)
}

/// Write the report: JSON on stdout, the summary on stderr, and the files `--out` and
/// `--junit` name. Returns the exit status.
pub fn report(
    args: &RunArgs,
    report: &RunReport,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let json = report.to_json();
    // A closed pipe cannot be reported anywhere else.
    let _ = stdout.write_all(json.as_bytes());
    let _ = writeln!(stderr, "{}", report.summary);
    let mut status = report.outcome.exit_status();
    for (file, contents) in [
        (args.send.out.as_ref(), json),
        (args.junit.as_ref(), report.junit()),
    ] {
        if let Some(file) = file
            && let Err(error) = native::atomic_write(file, contents.as_bytes())
        {
            let _ = writeln!(stderr, "error: could not write {}: {error}", file.display());
            status = USAGE_STATUS;
        }
    }
    status
}
