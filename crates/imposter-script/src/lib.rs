//! Scripts in Rhai, run in a sandbox: no files, network, clock or environment; the host gives
//! time and randomness. A before-sending script may change the request; an after-response
//! script reads the response and runs tests; both may set variables, which the caller
//! writes as captures are written. A mock route's script computes its answer from the
//! request, with a map of state kept between requests.

mod api;
mod crypto;
mod values;

use std::sync::{Arc, Mutex};

use imposter_core::execution::{Response, ScriptTest};
use imposter_core::generate::{Entropy, generate};
use imposter_core::mock::{MockReply, MockRequest};
use imposter_core::model::{RequestTemplate, Variables};
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, NativeCallContext, Position, Scope};

pub use api::{API, ApiEntry, Phase, readme_rows};
use values::{MockIn, MockOut, ScriptRequest, ScriptResponse, Vars, lock};

/// Scripts longer than this are refused before compiling.
pub const MAX_SOURCE: usize = 64 * 1024;
pub const MAX_OPERATIONS: u64 = 1_000_000;
/// Request and response bodies are strings in scripts, so the limit leaves room for them.
pub const MAX_STRING: usize = 8 * 1024 * 1024;
pub const MAX_COLLECTION: usize = 100_000;
pub const MAX_CALL_LEVELS: usize = 64;
pub const MAX_TESTS: usize = 500;
pub const MAX_CONSOLE_LINES: usize = 200;
pub const MAX_CONSOLE_BYTES: usize = 64 * 1024;
/// How often, in operations, the host is asked whether to stop.
const STOP_CHECK: u64 = 1024;

/// Why the host stopped a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The time a script may take ran out.
    TimeLimit,
    /// The user cancelled the send.
    Cancelled,
}

/// What the host gives a script: randomness and time, and when to stop.
pub struct Host {
    pub entropy: Box<dyn Entropy + Send>,
    pub stop: Arc<dyn Fn() -> Option<Stop> + Send + Sync>,
}

/// Why a script did not finish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("Scripts are limited to 64 KiB")]
    TooLong,
    #[error("Line {line}, column {column}: {message}")]
    Compile {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{}", match line { Some(line) => format!("Line {line}: {message}"), None => message.clone() })]
    Run {
        line: Option<usize>,
        message: String,
    },
    /// A limit or the host stopped it.
    #[error("{0}")]
    Stopped(String),
}

impl ScriptError {
    /// The error in a sentence naming the script: "Before-sending script, line 3: no key".
    pub fn describe(&self, script: &str) -> String {
        match self {
            Self::Compile {
                line,
                column,
                message,
            } => format!("{script}, line {line}, column {column}: {message}"),
            Self::Run {
                line: Some(line),
                message,
            } => format!("{script}, line {line}: {message}"),
            Self::Run {
                line: None,
                message,
            } => format!("{script}: {message}"),
            Self::TooLong | Self::Stopped(_) => format!("{script}: {self}"),
        }
    }

    /// The line the error points at, if any.
    pub const fn line(&self) -> Option<usize> {
        match self {
            Self::Compile { line, .. } => Some(*line),
            Self::Run { line, .. } => *line,
            Self::TooLong | Self::Stopped(_) => None,
        }
    }
}

/// What a run produced. On an error, what came before it is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub tests: Vec<ScriptTest>,
    /// `vars.set` calls in order; the caller decides each one's scope.
    pub sets: Vec<(String, String)>,
    /// `secrets.set` calls in order, stored as secrets where the caller decides.
    pub secret_sets: Vec<(String, String)>,
    pub console: Vec<String>,
    pub error: Option<ScriptError>,
}

/// Output for the console, capped so a loop of prints cannot fill memory.
#[derive(Default)]
struct Console {
    lines: Vec<String>,
    bytes: usize,
    cut: bool,
}

impl Console {
    fn push(&mut self, line: &str) {
        if self.cut {
            return;
        }
        if self.lines.len() >= MAX_CONSOLE_LINES || self.bytes + line.len() > MAX_CONSOLE_BYTES {
            self.lines.push("… output cut".to_owned());
            self.cut = true;
            return;
        }
        self.bytes += line.len();
        self.lines.push(line.to_owned());
    }
}

/// The state one run shares with the functions it registers.
struct Run {
    console: Arc<Mutex<Console>>,
    tests: Arc<Mutex<Vec<ScriptTest>>>,
    vars: Vars,
    secrets: values::Secrets,
}

/// An `Entropy` behind a lock, as the registered functions need.
struct Shared(Arc<Mutex<Box<dyn Entropy + Send>>>);

impl Entropy for Shared {
    fn now(&self) -> imposter_core::execution::Timestamp {
        lock(&self.0).now()
    }
    fn utc_offset(&self) -> i32 {
        lock(&self.0).utc_offset()
    }
    fn random(&mut self) -> u64 {
        lock(&self.0).random()
    }
}

fn generated(
    entropy: &Arc<Mutex<Box<dyn Entropy + Send>>>,
    name: &str,
) -> Result<String, Box<EvalAltResult>> {
    generate(name, &mut Shared(entropy.clone())).map_err(Into::into)
}

/// Whether an error from inside a test's closure must end the script rather than fail the
/// test: limits and stops are never a test's verdict.
fn is_stop(error: &EvalAltResult) -> bool {
    matches!(
        error.unwrap_inner(),
        EvalAltResult::ErrorTooManyOperations(_)
            | EvalAltResult::ErrorTerminated(..)
            | EvalAltResult::ErrorDataTooLarge(..)
            | EvalAltResult::ErrorStackOverflow(_)
    )
}

/// A value as a message shows it: text quoted, the rest as Rhai prints it.
fn shown(value: &Dynamic) -> String {
    if value.is_string() {
        format!("{:?}", value.to_string())
    } else {
        value.to_string()
    }
}

/// The sandboxed engine for one run, with every function registered.
fn engine(run: &Run, phase: Phase, host: Host) -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_string_size(MAX_STRING)
        .set_max_array_size(MAX_COLLECTION)
        .set_max_map_size(MAX_COLLECTION)
        .set_max_call_levels(MAX_CALL_LEVELS)
        .set_max_expr_depths(64, 32);
    engine.disable_symbol("eval");
    let stop = host.stop;
    engine.on_progress(move |operations| {
        if operations % STOP_CHECK != 0 {
            return None;
        }
        stop().map(|reason| Dynamic::from(reason == Stop::Cancelled))
    });
    let console = run.console.clone();
    engine.on_print(move |text| lock(&console).push(text));
    let console = run.console.clone();
    engine.on_debug(move |text, _, _| lock(&console).push(text));

    values::register(&mut engine);
    crypto::register(&mut engine);
    let entropy = Arc::new(Mutex::new(host.entropy));
    let (uuid, now, iso, random) = (entropy.clone(), entropy.clone(), entropy.clone(), entropy);
    engine
        .register_fn("uuid", move || generated(&uuid, "$uuid"))
        .register_fn("now_ms", move || -> Result<i64, Box<EvalAltResult>> {
            Ok(generated(&now, "$timestampMs")?.parse().unwrap_or_default())
        })
        .register_fn("now_iso", move || generated(&iso, "$isoTimestamp"))
        .register_fn(
            "random_int",
            move |min: i64, max: i64| -> Result<i64, Box<EvalAltResult>> {
                let text = generated(&random, &format!("$randomInt {min} {max}"))?;
                Ok(text.parse().unwrap_or(min))
            },
        )
        .register_fn(
            "to_json",
            |value: Dynamic| -> Result<String, Box<EvalAltResult>> {
                let value: serde_json::Value = rhai::serde::from_dynamic(&value)?;
                Ok(value.to_string())
            },
        )
        .register_fn(
            "expect",
            |context: NativeCallContext,
             actual: Dynamic,
             expected: Dynamic|
             -> Result<(), Box<EvalAltResult>> {
                let equal = context
                    .call_native_fn::<bool>("==", (actual.clone(), expected.clone()))
                    .unwrap_or(false);
                if equal {
                    Ok(())
                } else {
                    Err(format!("expected {}, got {}", shown(&expected), shown(&actual)).into())
                }
            },
        );
    register_tests(&mut engine, run, phase);
    engine
}

fn register_tests(engine: &mut Engine, run: &Run, phase: Phase) {
    let refusal = match phase {
        Phase::Before => Some("Tests run on the response: put them in the after-response script"),
        Phase::Mock => Some("Tests belong in request scripts; a mock only answers"),
        Phase::After | Phase::Requests | Phase::All => None,
    };
    if let Some(refusal) = refusal {
        let refuse = move |_: &str, _: Dynamic| -> Result<(), Box<EvalAltResult>> {
            Err(refusal.to_owned().into())
        };
        engine.register_fn("test", refuse);
        return;
    }
    let record = {
        let tests = run.tests.clone();
        move |name: &str, passed: bool, message: String| -> Result<(), Box<EvalAltResult>> {
            let mut tests = lock(&tests);
            if tests.len() >= MAX_TESTS {
                return Err(format!("A script may run at most {MAX_TESTS} tests").into());
            }
            tests.push(ScriptTest {
                name: name.to_owned(),
                passed,
                message,
            });
            Ok(())
        }
    };
    let by_value = record.clone();
    engine
        .register_fn("test", move |name: &str, passed: bool| {
            let message = if passed {
                String::new()
            } else {
                "false".to_owned()
            };
            by_value(name, passed, message)
        })
        .register_fn(
            "test",
            move |context: NativeCallContext, name: &str, body: FnPtr| match body
                .call_within_context::<Dynamic>(&context, ())
            {
                Ok(value) if value.as_bool() == Ok(false) => {
                    record(name, false, "false".to_owned())
                }
                Ok(_) => record(name, true, String::new()),
                Err(error) if is_stop(&error) => Err(error),
                Err(error) => record(name, false, message(&error)),
            },
        );
}

/// The words of an error, without Rhai's position suffix (the line is reported apart).
fn message(error: &EvalAltResult) -> String {
    let inner = error.unwrap_inner();
    if let EvalAltResult::ErrorRuntime(value, _) = inner {
        return value.to_string();
    }
    let text = inner.to_string();
    match text.rfind(" (line ") {
        Some(at) if text.ends_with(')') => text[..at].to_owned(),
        _ => text,
    }
}

fn line(position: Position) -> Option<usize> {
    position.line()
}

fn run_error(error: &EvalAltResult) -> ScriptError {
    match error.unwrap_inner() {
        EvalAltResult::ErrorTooManyOperations(_) => ScriptError::Stopped(format!(
            "Stopped: used more than {} operations",
            count(MAX_OPERATIONS)
        )),
        EvalAltResult::ErrorTerminated(reason, _) => ScriptError::Stopped(
            if reason.as_bool() == Ok(true) {
                "Stopped: the send was cancelled"
            } else {
                "Stopped: the script took too long"
            }
            .to_owned(),
        ),
        EvalAltResult::ErrorDataTooLarge(what, _) => ScriptError::Stopped(format!(
            "Stopped: {} grew past the limit",
            what.to_lowercase()
        )),
        EvalAltResult::ErrorStackOverflow(_) => {
            ScriptError::Stopped("Stopped: functions called each other too deeply".to_owned())
        }
        _ => ScriptError::Run {
            line: line(error.position()).or_else(|| line(error.unwrap_inner().position())),
            message: message(error),
        },
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Whether `source` compiles; the editor shows the answer under the script.
pub fn check(source: &str) -> Result<(), ScriptError> {
    compile(source).map(drop)
}

/// A compiled script, shared cheaply: a load run compiles once and every request runs it.
#[derive(Clone)]
pub struct Compiled(Arc<rhai::AST>);

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Compiled")
    }
}

/// Compile `source` once, to run with [`before_compiled`] and [`after_compiled`]. The
/// syntax tree does not depend on the engine that runs it, so any run's engine can take it.
pub fn compile(source: &str) -> Result<Compiled, ScriptError> {
    if source.len() > MAX_SOURCE {
        return Err(ScriptError::TooLong);
    }
    let mut engine = Engine::new_raw();
    engine.set_max_expr_depths(64, 32).disable_symbol("eval");
    engine
        .compile(source)
        .map(|ast| Compiled(Arc::new(ast)))
        .map_err(|error| {
            let position = error.position();
            ScriptError::Compile {
                line: position.line().unwrap_or(1),
                column: position.position().unwrap_or(1),
                message: error.err_type().to_string(),
            }
        })
}

fn execute(
    run: &Run,
    engine: &Engine,
    script: &Compiled,
    scope: &mut Scope<'_>,
    vars: bool,
) -> Result<(), ScriptError> {
    if vars {
        scope.push("vars", run.vars.clone());
        scope.push("secrets", run.secrets.clone());
    }
    engine
        .run_ast_with_scope(scope, &script.0)
        .map_err(|error| run_error(&error))
}

/// What a script that did not compile reports: nothing ran.
fn not_compiled(error: ScriptError) -> Outcome {
    Outcome {
        error: Some(error),
        ..Outcome::default()
    }
}

fn new_run(values: &Variables) -> Run {
    let vars = Vars::new(values.clone());
    let secrets = values::Secrets::new(vars.values.clone());
    Run {
        console: Arc::new(Mutex::new(Console::default())),
        tests: Arc::new(Mutex::new(Vec::new())),
        vars,
        secrets,
    }
}

fn finish(run: &Run, error: Option<ScriptError>) -> Outcome {
    Outcome {
        tests: lock(&run.tests).clone(),
        sets: lock(&run.vars.sets).clone(),
        secret_sets: lock(&run.secrets.sets).clone(),
        console: lock(&run.console).lines.clone(),
        error,
    }
}

/// Run a before-sending script on the resolved `template`, which it may change. On an
/// error the template is left as it was: the request is not sent.
pub fn before(
    source: &str,
    template: &mut RequestTemplate,
    values: &Variables,
    host: Host,
) -> Outcome {
    match compile(source) {
        Ok(script) => before_compiled(&script, template, values, host),
        Err(error) => not_compiled(error),
    }
}

/// [`before`] with a script compiled once.
pub fn before_compiled(
    script: &Compiled,
    template: &mut RequestTemplate,
    values: &Variables,
    host: Host,
) -> Outcome {
    let run = new_run(values);
    let engine = engine(&run, Phase::Before, host);
    let request = ScriptRequest::new(template.clone(), true);
    let mut scope = Scope::new();
    scope.push("request", request.clone());
    let result = execute(&run, &engine, script, &mut scope, true);
    if result.is_ok() {
        *template = request.template();
    }
    finish(&run, result.err())
}

/// Run an after-response script against `response`, with the request as sent.
pub fn after(
    source: &str,
    template: &RequestTemplate,
    response: &Response,
    values: &Variables,
    host: Host,
) -> Outcome {
    match compile(source) {
        Ok(script) => after_compiled(&script, template, response, values, host),
        Err(error) => not_compiled(error),
    }
}

/// [`after`] with a script compiled once.
pub fn after_compiled(
    script: &Compiled,
    template: &RequestTemplate,
    response: &Response,
    values: &Variables,
    host: Host,
) -> Outcome {
    let run = new_run(values);
    let engine = engine(&run, Phase::After, host);
    let mut scope = Scope::new();
    scope.push("request", ScriptRequest::new(template.clone(), false));
    scope.push("response", ScriptResponse(Arc::new(response.clone())));
    let result = execute(&run, &engine, script, &mut scope, true);
    finish(&run, result.err())
}

/// A mock server's state: kept between requests while it runs, empty on every start.
#[derive(Debug, Clone, Default)]
pub struct MockState(rhai::Map);

/// What a mock route's script made of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockRun {
    /// The answer, or why there is none (the caller answers 500 with it).
    pub reply: Result<MockReply, ScriptError>,
    pub console: Vec<String>,
}

/// Run a mock route's script: `reply` is the route's static answer, which it may change;
/// `state` is the server's, changed only when the script succeeds.
pub fn mock(
    source: &str,
    request: &MockRequest,
    reply: MockReply,
    state: &mut MockState,
    host: Host,
) -> MockRun {
    let script = match compile(source) {
        Ok(script) => script,
        Err(error) => {
            return MockRun {
                reply: Err(error),
                console: Vec::new(),
            };
        }
    };
    let run = new_run(&Variables::new());
    let engine = engine(&run, Phase::Mock, host);
    let out = MockOut(Arc::new(Mutex::new(reply)));
    let mut scope = Scope::new();
    scope.push("request", MockIn(Arc::new(request.clone())));
    scope.push("response", out.clone());
    scope.push("state", state.0.clone());
    let result = execute(&run, &engine, &script, &mut scope, false).and_then(|()| {
        let kept = scope
            .get_value::<rhai::Map>("state")
            .ok_or_else(|| ScriptError::Run {
                line: None,
                message: "state must stay a map".to_owned(),
            })?;
        let reply = lock(&out.0).clone();
        if let Some(problem) = reply.problem() {
            return Err(ScriptError::Run {
                line: None,
                message: problem,
            });
        }
        state.0 = kept;
        Ok(reply)
    });
    MockRun {
        reply: result,
        console: lock(&run.console).lines.clone(),
    }
}

#[cfg(test)]
mod tests;
