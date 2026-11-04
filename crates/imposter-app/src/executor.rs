//! Executing a request: snapshot, resolve, validate, send, then record history without ever
//! losing the response.

use std::time::Instant;

use imposter_core::capture::Write;
use imposter_core::execution::{ExecutionRecord, Outcome, Response, Timestamp};
use imposter_core::generate::Entropy;
use imposter_core::ids::ExecutionId;
use imposter_core::model::{Request, RequestTemplate, SavedValues, Variables};
use imposter_core::prepare::prepare;
use imposter_core::settings::AppSettings;
use imposter_core::variables::{
    CollectionScope, Scope, Snapshot, VariableContext, resolve_template,
};
use imposter_http::{Policy, SendError};
use imposter_store::{StoreError, StoreHandle, collections, environments, executions, kv, scopes};
use tokio_util::sync::CancellationToken;

/// Everything an execution depends on, read in one database command. The database thread runs
/// commands in order, so an environment switch queued before Send applies, and one made after
/// cannot change a request already running.
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub settings: AppSettings,
    pub context: VariableContext,
    pub snapshot: Snapshot,
}

/// Read the inputs for `request`. Inspection uses this too, so it explains exactly what an
/// execution would use.
pub async fn inputs(store: &StoreHandle, request: &Request) -> Result<Inputs, StoreError> {
    let collection_id = request.collection_id;
    let captured = request.collection_variables_snapshot.clone();
    store
        .call(move |db| {
            let settings = kv::app_settings(db)?;
            let context = environments::variable_context(db)?;
            let collection = match collection_id {
                Some(id) => collections::collection(db, id)?,
                None => None,
            };
            let saved = match &collection {
                Some(collection) => scopes::saved_values(db, Scope::Collection(collection.id))?,
                None => SavedValues::default(),
            };
            let scope = match &collection {
                Some(collection) => Some(CollectionScope::Live {
                    id: collection.id,
                    name: &collection.name,
                    values: &collection.variables,
                    saved: &saved,
                }),
                None if !captured.is_empty() || collection_id.is_some() => {
                    Some(CollectionScope::Deleted {
                        captured: &captured,
                    })
                }
                None => None,
            };
            let snapshot = Snapshot::build(&context, scope);
            Ok(Inputs {
                settings,
                context,
                snapshot,
            })
        })
        .await
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    /// The request cannot be sent as it is (bad URL, unresolved variables, conflicts…).
    /// Nothing was sent and nothing is recorded.
    #[error("{0}")]
    Invalid(String),
    /// Variables the request uses that nothing defines, by name: the UI offers to define
    /// them. Nothing was sent.
    #[error("Unresolved variables: {}", .0.join(", "))]
    Unresolved(Vec<String>),
    /// The before-sending script failed or was stopped, in words naming it and its line.
    /// Nothing was sent.
    #[error("{0}")]
    Script(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Executed {
    pub response: Response,
    /// The history entry, unless recording failed (see `response.history_error`).
    pub record: Option<ExecutionId>,
    /// Values the request's captures took, to write into variables. The caller writes them
    /// ([`write_captures`]) once it knows no unsaved edit to the same scope would be lost.
    pub writes: Vec<Write>,
    /// What was sent: variables and generated values filled in, as History records it.
    pub sent: Box<Request>,
    /// Whether cookies were attached, to show the request as sent.
    pub attach_cookies: bool,
}

/// Write captured values, one transaction for all of them.
pub async fn write_captures(store: &StoreHandle, writes: Vec<Write>) -> Result<(), StoreError> {
    store
        .call(move |db| {
            for scope in imposter_core::capture::scopes(&writes) {
                let values: Vec<(String, String)> = writes
                    .iter()
                    .filter(|w| w.scope == scope)
                    .map(|w| (w.name.clone(), w.value.clone()))
                    .collect();
                scopes::set_current(db, scope, &values)?;
            }
            Ok(())
        })
        .await
}

pub const HISTORY_ERROR: &str = "Response received, but history could not be saved";

/// Current wall-clock time for history timestamps.
pub fn now() -> Timestamp {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// The clock, local time zone and operating-system randomness behind generated values.
pub struct SystemEntropy {
    /// Mixed in only if the operating system cannot supply randomness.
    fallback: u64,
}

impl Default for SystemEntropy {
    fn default() -> Self {
        Self {
            fallback: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    & u128::from(u64::MAX),
            )
            .unwrap_or_default()
                ^ u64::from(std::process::id()),
        }
    }
}

impl Entropy for SystemEntropy {
    fn now(&self) -> Timestamp {
        now()
    }

    fn utc_offset(&self) -> i32 {
        jiff::tz::TimeZone::system()
            .to_offset(jiff::Timestamp::now())
            .seconds()
    }

    fn random(&mut self) -> u64 {
        getrandom::u64().unwrap_or_else(|_| {
            // SplitMix64: distinct values even without the operating system's randomness.
            self.fallback = self.fallback.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.fallback;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        })
    }
}

/// Resolve `request` against `inputs`: inherited settings become explicit options first, so
/// variables inside inherited paths resolve against the same snapshot as the request.
/// Generated values come from the system.
pub fn resolve(request: &Request, inputs: &Inputs) -> Result<(Request, AppSettings), ExecuteError> {
    resolve_with(request, inputs, &mut SystemEntropy::default()).map(|(r, s, _)| (r, s))
}

/// [`resolve`] with generated values from `entropy`, also returning every value used, so
/// checks and redirects see the same generated values as the request.
pub fn resolve_with(
    request: &Request,
    inputs: &Inputs,
    entropy: &mut impl Entropy,
) -> Result<(Request, AppSettings, Variables), ExecuteError> {
    let mut values = inputs.snapshot.values();
    let mut with_settings = request.template.clone();
    // Inherited settings can hold placeholders too: generate for the request with them, once.
    with_settings.options = inputs
        .settings
        .resolve_for_url(&request.template.url, &request.template.options)
        .as_options();
    imposter_core::generate::fill(&with_settings, &mut values, entropy);
    let mut missing = std::collections::BTreeSet::new();
    let url = imposter_core::variables::substitute(
        &request.template.url,
        |name| values.get(name).map(String::as_str),
        &mut missing,
    );
    let settings = inputs
        .settings
        .resolve_for_url(&url, &request.template.options);
    let mut template = request.template.clone();
    template.options = settings.as_options();
    let template = resolve_template(&template, &values).map_err(|error| match error {
        imposter_core::variables::ResolveError::Unresolved(names) => {
            ExecuteError::Unresolved(names.into_iter().collect())
        }
        other => ExecuteError::Invalid(other.to_string()),
    })?;
    let settings = settings.resolve(&template.options);
    let resolved = Request {
        template,
        collection_variables_snapshot: inputs.snapshot.collection_values.clone(),
        ..request.clone()
    };
    Ok((resolved, settings, values))
}

/// `request` as a curl command with what Send would send now: variables resolved, auth
/// headers built, settings applied. It contains secrets; only copy it on request.
pub async fn curl_command(store: &StoreHandle, request: &Request) -> Result<String, ExecuteError> {
    Ok(curl_with_identity_notice(store, request).await?.0)
}

pub const CURL_IDENTITY_NOTICE: &str = "Copied as curl using the initial client identity; curl cannot switch host certificates across redirects";

/// The warning travels with the same settings snapshot that produced the command.
pub async fn curl_with_identity_notice(
    store: &StoreHandle,
    request: &Request,
) -> Result<(String, bool), ExecuteError> {
    let inputs = inputs(store, request).await?;
    let (resolved, settings, values) =
        resolve_with(request, &inputs, &mut SystemEntropy::default())?;
    // What Send would send includes the before-sending script's changes.
    let (template, _) =
        before_script(request, resolved.template, values, CancellationToken::new()).await?;
    let prepared = prepare(&template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    Ok((
        imposter_core::curl::to_command(&prepared, &settings),
        !settings.host_identities.is_empty(),
    ))
}

/// How long one script may run before it is stopped.
pub const SCRIPT_TIME: std::time::Duration = std::time::Duration::from_secs(2);

/// The words naming each script in errors.
pub const BEFORE_SCRIPT: &str = "Before-sending script";
pub const AFTER_SCRIPT: &str = "After-response script";

/// What a script may use: the system's randomness and clock, stopped by Cancel or once
/// [`SCRIPT_TIME`] has passed since it started.
pub fn script_host(cancel: CancellationToken, time: std::time::Duration) -> imposter_script::Host {
    let deadline = Instant::now() + time;
    imposter_script::Host {
        entropy: Box::new(SystemEntropy::default()),
        stop: std::sync::Arc::new(move || {
            if cancel.is_cancelled() {
                Some(imposter_script::Stop::Cancelled)
            } else if Instant::now() >= deadline {
                Some(imposter_script::Stop::TimeLimit)
            } else {
                None
            }
        }),
    }
}

/// Run `request`'s before-sending script on the resolved template, off the async workers.
/// A failure is the request's "not sent" reason.
async fn before_script(
    request: &Request,
    mut resolved: RequestTemplate,
    values: Variables,
    cancel: CancellationToken,
) -> Result<(RequestTemplate, imposter_script::Outcome), ExecuteError> {
    let source = request.template.scripts.before.clone();
    if source.trim().is_empty() {
        return Ok((resolved, imposter_script::Outcome::default()));
    }
    let (resolved, outcome) = tokio::task::spawn_blocking(move || {
        let outcome = imposter_script::before(
            &source,
            &mut resolved,
            &values,
            script_host(cancel, SCRIPT_TIME),
        );
        (resolved, outcome)
    })
    .await
    .map_err(|_| ExecuteError::Script(format!("{BEFORE_SCRIPT} failed unexpectedly")))?;
    match &outcome.error {
        Some(error) => Err(ExecuteError::Script(error.describe(BEFORE_SCRIPT))),
        None => Ok((resolved, outcome)),
    }
}

/// Run the after-response script; its error is reported with the response, never instead.
async fn after_script(
    source: String,
    sent: RequestTemplate,
    response: Response,
    values: Variables,
    cancel: CancellationToken,
) -> imposter_script::Outcome {
    tokio::task::spawn_blocking(move || {
        imposter_script::after(
            &source,
            &sent,
            &response,
            &values,
            script_host(cancel, SCRIPT_TIME),
        )
    })
    .await
    .unwrap_or_else(|_| imposter_script::Outcome {
        error: Some(imposter_script::ScriptError::Stopped(
            "Stopped: the script failed unexpectedly".to_owned(),
        )),
        ..imposter_script::Outcome::default()
    })
}

/// A recorded execution as the curl command of what it sent.
pub fn recorded_curl(record: &ExecutionRecord) -> Result<String, ExecuteError> {
    let prepared = prepare(&record.resolved.template, record.settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    Ok(imposter_core::curl::to_command(&prepared, &record.settings))
}

/// Send `request` and record it. `cancel` stops the network exchange; a cancelled execution
/// is still recorded, like any other outcome.
pub async fn execute(
    store: &StoreHandle,
    request: Request,
    cancel: CancellationToken,
) -> Result<Executed, ExecuteError> {
    let inputs = inputs(store, &request).await?;
    let (mut resolved, settings, mut values) =
        resolve_with(&request, &inputs, &mut SystemEntropy::default())?;
    let (template, before) =
        before_script(&request, resolved.template, values.clone(), cancel.clone()).await?;
    resolved.template = template;
    let prepared = prepare(&resolved.template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;

    let started = Instant::now();
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing = Some(Box::new(imposter_http::transport::IdentityRouting {
        settings: inputs.settings.clone(),
        options: request.template.options.clone(),
        variables: values.clone(),
    }));
    let mut response = match imposter_http::send(&prepared, &policy, &cancel).await {
        Ok(response) => response,
        Err(SendError::Cancelled) => {
            Response::failure(Outcome::Cancelled, "Cancelled", "Request cancelled")
        }
        Err(SendError::Failed { kind, message }) => {
            Response::failure(Outcome::TransportError, kind.as_str(), message)
        }
    };
    response.elapsed = started.elapsed().as_secs_f64();
    response.checks = imposter_core::check::evaluate(&request.template.checks, &response, &values);
    let (captured, mut writes) = imposter_core::capture::evaluate(
        &request.template.captures,
        &response,
        &inputs.snapshot,
        &inputs.context,
    );
    response.captures = captured;
    // The after-response script sees the before-sending script's sets, and runs only on a
    // response, as checks do.
    let mut sets = before.sets;
    response.console = before.console;
    let after_source = request.template.scripts.after.clone();
    if !after_source.trim().is_empty() && response.outcome == Outcome::Response {
        for (name, value) in &sets {
            values.insert(name.clone(), value.clone());
        }
        let after = after_script(
            after_source,
            resolved.template.clone(),
            response.clone(),
            values.clone(),
            cancel.clone(),
        )
        .await;
        response.console.extend(after.console);
        response.script = Some(imposter_core::execution::ScriptReport {
            tests: after.tests,
            error: after
                .error
                .map(|e| e.describe(AFTER_SCRIPT))
                .unwrap_or_default(),
        });
        sets.extend(after.sets);
    }
    // Scripts set variables where captures would, after the captures.
    writes.extend(sets.into_iter().map(|(name, value)| Write {
        scope: imposter_core::capture::target(&inputs.snapshot, &inputs.context, &name).0,
        name,
        value,
    }));

    let mut template = request.clone();
    template.collection_variables_snapshot = resolved.collection_variables_snapshot.clone();
    let sent = Box::new(resolved.clone());
    let attach_cookies = settings.attach_cookies;
    let record = ExecutionRecord {
        timestamp: now(),
        request_id: request.id,
        template,
        resolved,
        settings: settings.clone(),
        environment_id: inputs.context.environment_id(),
        environment: inputs.context.environment_name().to_owned(),
        response: response.clone(),
    };
    let (entries, bytes) = (settings.history_max_entries, settings.history_max_bytes);
    match store
        .call(move |db| executions::record(db, &record, entries, bytes))
        .await
    {
        Ok(id) => Ok(Executed {
            response,
            record: Some(id),
            writes,
            sent,
            attach_cookies,
        }),
        Err(error) => {
            tracing::warn!(%error, "recording an execution failed");
            HISTORY_ERROR.clone_into(&mut response.history_error);
            Ok(Executed {
                response,
                record: None,
                writes,
                sent,
                attach_cookies,
            })
        }
    }
}
