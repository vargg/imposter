//! `imposter run`: a folder or a collection as one run, through the app's runner. Progress
//! goes to stderr one line per request as it ends; the report goes to stdout, `--out` and,
//! as JUnit XML, `--junit`. Everything printed or written is redacted.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use imposter_app::document::{self, RequestPath, Selected};
use imposter_app::executor::{self, SystemEntropy};
use imposter_app::formats::native;
use imposter_app::http::destination::Destinations;
use imposter_app::runner::{self, Options};
use imposter_app::{paths, profile};
use imposter_core::redact::Redactor;
use imposter_core::run_report::{self, RunReport, Step};
use imposter_core::run_result::{self, Definition, Input, Limits, RunInfo, RunOutcome, RunResult};
use tokio_util::sync::CancellationToken;

use crate::USAGE_STATUS;
use crate::send::{
    SendArgs, Unsent, read_inputs, read_project, recorded_input, sha256_hex, source_of,
    without_secrets,
};

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

/// `selected` with relative file paths found under `base`.
fn based(mut selected: Vec<Selected>, base: Option<&std::path::Path>) -> Vec<Selected> {
    for one in &mut selected {
        one.inputs.base_dir = base.map(std::path::Path::to_path_buf);
    }
    selected
}

/// The requests to run, read once at the start.
async fn select(
    args: &SendArgs,
    var: &dyn Fn(&str) -> Option<String>,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<Requests, Unsent> {
    let target = RequestPath::parse(&args.request).map_err(|e| e.to_string())?;
    if args.file.is_none()
        && let Some(dir) = &args.project
    {
        let (folder, revision) = read_project(dir).await?;
        definition.sha256 = Some(revision);
        let document = folder.document(&target).map_err(|e| e.to_string())?;
        let values = read_inputs(&args.inputs, var, redactor)?;
        let selected =
            document::select_all(document, &target, args.environment.as_deref(), &values)
                .map_err(|e| e.to_string())?;
        return Ok(Requests::File(based(selected, Some(dir))));
    }
    if let Some(file) = &args.file {
        let reading = file.clone();
        let bytes = tokio::task::spawn_blocking(move || native::read_document(&reading))
            .await
            .map_err(|_| "The file could not be read".to_owned())?
            .map_err(|e| e.to_string())?;
        definition.sha256 = Some(sha256_hex(&bytes));
        let document = native::decode(&bytes).map_err(|e| e.to_string())?;
        let values = read_inputs(&args.inputs, var, redactor)?;
        let selected =
            document::select_all(&document, &target, args.environment.as_deref(), &values)
                .map_err(|e| e.to_string())?;
        return Ok(Requests::File(based(selected, file.parent())));
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
    let run_id = new_id();
    let mut redactor = Redactor::default();
    let mut definition = Definition {
        source: source_of(&args.send),
        file: args
            .send
            .file
            .as_ref()
            .or(args.send.project.as_ref())
            .map(|f| f.display().to_string()),
        sha256: None,
        request: args.send.request.clone(),
        environment: args.send.environment.clone(),
    };
    let inputs: Vec<Input> = args.send.inputs.iter().map(recorded_input).collect();
    let limits = Limits {
        capture_bytes: 0,
        excerpt_bytes: args.send.max_body,
    };
    let selected = select(&args.send, var, &mut redactor, &mut definition).await;
    let meta = run_report::Meta {
        run: RunInfo {
            id: run_id,
            tool: crate::version(),
            started,
            elapsed_ms: 0,
        },
        definition,
        inputs,
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
            let mut meta = meta;
            meta.run.elapsed_ms = milliseconds(clock);
            return RunReport::unsent(outcome, &message, meta).redacted(&redactor);
        }
    };
    let options = Options {
        keep_going: args.keep_going,
        record: store
            .as_ref()
            .map(|store| (store.handle(), imposter_core::execution::Via::CommandLine)),
        headless: true,
    };
    let report = run_selected(requests, options, meta, redactor, cancel, None, |line| {
        let _ = writeln!(stderr, "{line}");
    })
    .await;
    if let Some(store) = store {
        // Closing waits for the database thread to finish its last History write.
        let _ = tokio::task::spawn_blocking(move || store.close()).await;
    }
    report
}

pub(crate) fn new_id() -> String {
    imposter_core::generate::generate("$uuid", &mut SystemEntropy::default()).unwrap_or_default()
}

pub(crate) async fn run_selected(
    requests: Vec<Selected>,
    options: Options,
    mut meta: run_report::Meta,
    mut redactor: Redactor,
    cancel: CancellationToken,
    destinations: Option<&Destinations>,
    mut print: impl FnMut(&str),
) -> RunReport {
    let clock = Instant::now();
    let mut steps: Vec<Step> = Vec::with_capacity(requests.len());
    let step_meta = |path: &str, started, limits: Limits| run_result::Meta {
        run: RunInfo {
            id: new_id(),
            tool: meta.run.tool.clone(),
            started,
            elapsed_ms: 0,
        },
        definition: Definition {
            request: path.to_owned(),
            ..meta.definition.clone()
        },
        inputs: meta.inputs.clone(),
        limits,
    };
    if destinations.is_some() {
        for selected in &requests {
            redactor.credentials(&selected.request.template, &selected.inputs.settings);
        }
    }
    runner::run_with_grant(requests, options, cancel, destinations, |ended| {
        let step = match ended {
            runner::Step::Ran {
                path,
                record,
                notes,
                secrets,
            } => {
                let record = without_secrets(record, secrets, &mut redactor);
                redactor.credentials(&record.resolved.template, &record.settings);
                let effective = record.settings.download_limit_bytes();
                let capture_bytes = destinations.map_or(effective.unwrap_or_default(), |_| {
                    let max = imposter_app::http::destination::CAPTURE_LIMIT;
                    effective.unwrap_or(max).min(max)
                });
                meta.limits.capture_bytes = meta.limits.capture_bytes.max(capture_bytes);
                let mut result = RunResult::sent(
                    &record,
                    step_meta(
                        path,
                        record.timestamp.to_rfc3339(),
                        Limits {
                            capture_bytes,
                            ..meta.limits
                        },
                    ),
                    &redactor,
                );
                result.notes.clone_from(notes);
                Step::Ran(Box::new(result))
            }
            runner::Step::NotSent {
                path,
                outcome,
                error,
            } => Step::Ran(Box::new(RunResult::unsent(
                *outcome,
                error.clone(),
                step_meta(
                    path,
                    executor::now().to_rfc3339(),
                    Limits {
                        capture_bytes: 0,
                        ..meta.limits
                    },
                ),
            ))),
            runner::Step::Skipped { path, reason } => Step::Skipped {
                path: path.clone(),
                reason: reason.clone(),
            },
        };
        print(&redactor.text(&progress(&step)));
        steps.push(step);
    })
    .await;
    meta.run.elapsed_ms = milliseconds(clock);
    RunReport::new(steps, meta).redacted(&redactor)
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
