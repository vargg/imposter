//! Load runs: one request sent many times by concurrent workers, measured and summed up.
//! A run is kept apart from Send: nothing goes to History, captures and script variables are
//! never written, and it runs on its own runtime so the app's other work does not skew its
//! timings. The request's scripts run for every request, compiled once.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use imposter_core::check::{self, Check};
use imposter_core::load::{CheckTally, LoadPlan, Sample, SampleOutcome, Summary, micros};
use imposter_core::model::Request;
use imposter_core::prepare::prepare;
use imposter_core::select::Selector;
use imposter_http::transport::IdentityRouting;
use imposter_http::{Policy, SendError, Session};
use imposter_store::StoreHandle;
use tokio_util::sync::CancellationToken;

use crate::executor::{ExecuteError, Inputs, SystemEntropy, inputs, resolve_with};
use crate::scripts::{After, CompiledChain};

/// How far a run has got, read by the UI while it runs (plain counters, never a lock).
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    errors: AtomicU64,
}

impl Progress {
    /// Requests finished so far, with a response or an error.
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// Of those, transport errors and responses failing a check.
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    fn record(&self, sample: &Sample) {
        self.done.fetch_add(1, Ordering::Relaxed);
        if sample.is_error() {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub summary: Summary,
    /// Stopped by the user before every request was sent.
    pub stopped: bool,
    /// Ended by the time limit.
    pub timed_out: bool,
    /// The first script error of the run, in words naming the script and its line; the
    /// failures themselves are counted in the summary.
    pub script_error: Option<String>,
}

/// What every worker shares.
struct Run {
    request: Request,
    inputs: Inputs,
    plan: LoadPlan,
    session: Session,
    /// Body bytes to keep for the checks; 0 when none reads the body.
    keep: u64,
    next: AtomicU32,
    progress: Arc<Progress>,
    /// Cancelled by the user or the time limit; requests in flight are dropped.
    stop: CancellationToken,
    /// The scripts around the request (collection, folders, its own), compiled once for
    /// every request.
    scripts: CompiledChain,
    script_error: Mutex<Option<String>>,
}

impl Run {
    /// Keep the first script error the run meets.
    fn script_failed(&self, message: String) {
        let mut first = self
            .script_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        first.get_or_insert(message);
    }
}

/// The run's own runtime, shut down in the background however `run` ends: dropping a
/// runtime normally blocks, which an async context must not do (a run dropped half way, as
/// when the app's runtime shuts down, panicked).
struct OwnRuntime(Option<tokio::runtime::Runtime>);

impl OwnRuntime {
    fn handle(&self) -> &tokio::runtime::Handle {
        self.0
            .as_ref()
            .expect("the runtime is only taken when dropped")
            .handle()
    }
}

impl Drop for OwnRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// Whether a check reads the body, which then has to be kept up to the capture limit.
fn reads_body(checks: &[Check]) -> bool {
    checks.iter().filter(|c| c.enabled).any(|c| {
        matches!(
            c.selector,
            Selector::Json(_) | Selector::Body | Selector::Size
        )
    })
}

/// Run `plan` with `request`. The variables and settings are read once, and the request is
/// checked once before anything is sent: one that cannot be sent stops the run as it stops
/// Send. Each request then gets fresh generated values. `cancel` stops the run; what
/// finished is still summed up.
pub async fn run(
    store: &StoreHandle,
    request: Request,
    plan: LoadPlan,
    cancel: CancellationToken,
    progress: Arc<Progress>,
) -> Result<Report, ExecuteError> {
    let inputs = inputs(store, &request).await?;
    let scripts = inputs.scripts.with_request(&request).compile()?;
    let (resolved, settings, values) =
        resolve_with(&request, &inputs, &mut SystemEntropy::default())?;
    // The first request is checked whole, its before-sending scripts included, as Send would.
    let checked = scripts
        .before(resolved.template, values.clone(), cancel.clone())
        .await
        .map_err(ExecuteError::Script)?;
    prepare(&checked.template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing = Some(Box::new(IdentityRouting {
        settings: inputs.settings.clone(),
        options: request.template.options.clone(),
        variables: values,
    }));
    let keep = if reads_body(&request.template.checks) || scripts.has_after() {
        policy.capture_limit.unwrap_or(u64::MAX)
    } else {
        0
    };
    let idle = if plan.keep_alive() {
        usize::from(plan.concurrency())
    } else {
        0
    };
    let runtime = OwnRuntime(Some(
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("imposter-load")
            .enable_all()
            .build()
            .map_err(|e| ExecuteError::Invalid(format!("Cannot start the load run: {e}")))?,
    ));
    let stop = cancel.child_token();
    let timed_out = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Run {
        request,
        inputs,
        plan,
        session: Session::new(policy, idle),
        keep,
        next: AtomicU32::new(0),
        progress,
        stop: stop.clone(),
        scripts,
        script_error: Mutex::new(None),
    });
    let shared = Arc::clone(&run);
    let started = Instant::now();
    if let Some(limit) = plan.time_limit() {
        let (stop, timed_out) = (stop.clone(), timed_out.clone());
        runtime.handle().spawn(async move {
            tokio::select! {
                () = stop.cancelled() => {}
                () = tokio::time::sleep(limit) => {
                    timed_out.store(true, Ordering::SeqCst);
                    stop.cancel();
                }
            }
        });
    }
    let workers = runtime.handle().spawn(async move {
        let mut workers = tokio::task::JoinSet::new();
        for index in 0..plan.concurrency() {
            workers.spawn(worker(run.clone(), index));
        }
        let mut samples = Vec::with_capacity(plan.requests() as usize);
        while let Some(done) = workers.join_next().await {
            samples.extend(done.unwrap_or_default());
        }
        samples
    });
    let samples = workers.await;
    let wall = started.elapsed();
    stop.cancel();
    drop(runtime);
    let samples = samples
        .map_err(|_| ExecuteError::Invalid("The load run failed unexpectedly".to_owned()))?;
    Ok(Report {
        summary: Summary::from_samples(&samples, wall),
        stopped: cancel.is_cancelled(),
        timed_out: timed_out.load(Ordering::SeqCst),
        script_error: shared
            .script_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    })
}

/// One worker: wait for its start in the ramp-up, then take request numbers until all are
/// taken or the run stops.
async fn worker(run: Arc<Run>, index: u16) -> Vec<Sample> {
    let mut samples = Vec::new();
    tokio::select! {
        () = run.stop.cancelled() => return samples,
        () = tokio::time::sleep(run.plan.start_offset(index)) => {}
    }
    let mut entropy = SystemEntropy::default();
    while !run.stop.is_cancelled() && run.next.fetch_add(1, Ordering::SeqCst) < run.plan.requests()
    {
        let sample = tokio::select! {
            biased;
            () = run.stop.cancelled() => break,
            sample = one(&run, &mut entropy) => sample,
        };
        run.progress.record(&sample);
        samples.push(sample);
    }
    samples
}

/// Resolve, send and check one request.
async fn one(run: &Run, entropy: &mut SystemEntropy) -> Sample {
    let started = Instant::now();
    let failed = |kind: &'static str| Sample {
        total_us: micros(started.elapsed()),
        first_byte_us: None,
        outcome: SampleOutcome::Error(kind),
        bytes: 0,
        checks: CheckTally::default(),
    };
    let Ok((resolved, settings, mut values)) = resolve_with(&run.request, &run.inputs, entropy)
    else {
        return failed("InvalidRequest");
    };
    // Script time is not the server's: latency is measured around the exchange only.
    let template = match run
        .scripts
        .before(resolved.template, values.clone(), run.stop.clone())
        .await
    {
        Ok(before) => {
            // The after scripts see what the before ones set, as after a Send.
            for (name, value) in before.sets {
                values.insert(name, value);
            }
            before.template
        }
        Err(message) => {
            run.script_failed(message);
            return failed("ScriptError");
        }
    };
    let Ok(prepared) = prepare(&template, settings.attach_cookies) else {
        return failed("InvalidRequest");
    };
    match run.session.measure(&prepared, run.keep).await {
        Ok(measured) => {
            let results =
                check::evaluate(&run.request.template.checks, &measured.response, &values);
            let mut checks = CheckTally::of(&results);
            if run.scripts.has_after() {
                let after = run
                    .scripts
                    .after(
                        template,
                        measured.response.clone(),
                        values,
                        run.stop.clone(),
                    )
                    .await;
                tally_after(run, &after, &mut checks);
            }
            Sample {
                total_us: micros(measured.total),
                first_byte_us: Some(micros(measured.first_byte)),
                outcome: SampleOutcome::Status(measured.response.status),
                bytes: measured.bytes,
                checks,
            }
        }
        Err(SendError::Failed { kind, .. }) => failed(kind.as_str()),
        Err(SendError::Cancelled) => failed("Cancelled"),
    }
}

/// Count the after-response scripts' tests with the checks; each script that failed counts
/// as one failure, as after a Send. Variables they set and what they print are dropped.
fn tally_after(run: &Run, after: &After, checks: &mut CheckTally) {
    let passed = after.tests.iter().filter(|t| t.passed).count() as u64;
    checks.passed += passed;
    checks.failed += after.tests.len() as u64 - passed;
    checks.failed += after.errors.len() as u64;
    if let Some(first) = after.errors.first() {
        run.script_failed(first.clone());
    }
}
