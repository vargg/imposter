//! `imposter send`: read the request once (from the profile, a native file or a collections
//! folder), read only the named process variables, run it through the app's engine, and
//! report a redacted result. A file or folder run reads nothing from the personal profile
//! and writes nothing to History.

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use imposter_app::document::{self, RequestPath};
use imposter_app::executor::{self, ExecuteError, SystemEntropy};
use imposter_app::formats::native;
use imposter_app::formats::native::NativeDocument;
use imposter_app::project::{self, ProjectFolder};
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
    /// A collections folder to read instead of the personal profile.
    pub project: Option<PathBuf>,
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
            project: None,
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
            // `{{secret:name}}` too, so a request written for the app's secrets runs.
            values.insert(
                imposter_core::variables::secret_key(&input.variable),
                value.clone(),
            );
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
        source: source_of(args),
        file: args
            .file
            .as_ref()
            .or(args.project.as_ref())
            .map(|f| f.display().to_string()),
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

/// Where `args` reads the definitions.
pub(crate) fn source_of(args: &SendArgs) -> Source {
    match (&args.file, &args.project) {
        (Some(_), _) => Source::File,
        (None, Some(_)) => Source::Project,
        (None, None) => Source::Profile,
    }
}

/// SHA-256 of a file's bytes in hex: the revision of the definitions that ran.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    digest
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// SHA-256 over a folder's files, each as its path, a NUL, its length and its bytes, in
/// path order: changing, adding or removing any file changes it.
pub(crate) fn folder_revision(files: &std::collections::BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(bytes);
    }
    hex(&hasher.finalize())
}

/// The folder `--project` names, read once, and the revision of its files.
pub(crate) async fn read_project(dir: &Path) -> Result<(ProjectFolder, String), Unsent> {
    let dir = dir.to_owned();
    let folder = tokio::task::spawn_blocking(move || project::read(&dir))
        .await
        .map_err(|_| "The collections folder could not be read".to_owned())?
        .map_err(|e| e.to_string())?;
    let revision = folder_revision(&folder.files);
    Ok((folder, revision))
}

/// `record` as a result may show it: every secret the run used, captured or set is added
/// to `redactor`, and a secret capture's value is replaced whatever its length.
pub(crate) fn without_secrets(
    record: &ExecutionRecord,
    secrets: &[String],
    redactor: &mut Redactor,
) -> ExecutionRecord {
    for secret in secrets.iter().filter(|s| s.chars().count() >= MIN_SECRET) {
        redactor.secret(secret);
    }
    let mut record = record.clone();
    for captured in record.response.captures.iter_mut().filter(|c| c.secret) {
        if let imposter_core::capture::Taken::Value(value) = &mut captured.taken {
            imposter_core::redact::MARK.clone_into(value);
        }
    }
    record
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
    match (&args.file, &args.project) {
        (Some(file), _) => run_file(args, file, &path, var, cancel, redactor, definition).await,
        (None, Some(dir)) => {
            let (folder, revision) = read_project(dir).await?;
            definition.sha256 = Some(revision);
            let document = folder.document(&path).map_err(|e| e.to_string())?;
            run_document(args, document, &path, Some(dir), var, cancel, redactor).await
        }
        (None, None) => run_profile(args, &path, var, cancel, redactor).await,
    }
}

async fn run_file(
    args: &SendArgs,
    file: &Path,
    path: &RequestPath,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
    definition: &mut Definition,
) -> Result<Ran, Unsent> {
    let reading = file.to_owned();
    // Read once: an edit to the file during the run cannot change it.
    let bytes = tokio::task::spawn_blocking(move || native::read_document(&reading))
        .await
        .map_err(|_| "The file could not be read".to_owned())?
        .map_err(|e| e.to_string())?;
    definition.sha256 = Some(sha256_hex(&bytes));
    let document = native::decode(&bytes).map_err(|e| e.to_string())?;
    run_document(args, &document, path, file.parent(), var, cancel, redactor).await
}

/// A request of a document read from a file or a folder: nothing is recorded.
/// Relative file paths in its requests are found under `base`: the folder, or the file's
/// own directory.
async fn run_document(
    args: &SendArgs,
    document: &NativeDocument,
    path: &RequestPath,
    base: Option<&Path>,
    var: &dyn Fn(&str) -> Option<String>,
    cancel: CancellationToken,
    redactor: &mut Redactor,
) -> Result<Ran, Unsent> {
    let values = read_inputs(&args.inputs, var, redactor)?;
    let mut selected = document::select(document, path, args.environment.as_deref(), &values)
        .map_err(|e| e.to_string())?;
    selected.inputs.base_dir = base.map(Path::to_path_buf);
    let capture_bytes = selected.inputs.settings.download_limit.saturating_mul(1024);
    let ran = executor::run(&selected.inputs, selected.request, cancel)
        .await
        .map_err(unsent)?;
    let record = without_secrets(&ran.record, &ran.secrets, redactor);
    Ok((record, capture_bytes, Vec::new()))
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
        let record = without_secrets(&ran.record, &ran.secrets, redactor);
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
