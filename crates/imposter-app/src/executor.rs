//! Executing a request: snapshot, resolve, validate, send, then record history without ever
//! losing the response.

use std::time::Instant;

use imposter_core::capture::{Destination, Write};
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
    /// The collection's and enclosing folders' scripts, outermost first; the request's own
    /// are added by the caller (`ScriptChain::with_request`).
    pub scripts: crate::scripts::ScriptChain,
    pub writes: Writes,
    /// Where relative file paths in the request are found: the collections folder, or the
    /// file a request was read from. `None` leaves them to the working directory.
    pub base_dir: Option<std::path::PathBuf>,
    /// The cookie jar the execution sends from and fills, when the settings keep cookies:
    /// a copy of the stored one for Send, which keeps what it receives afterwards
    /// ([`execute`]); one jar shared by a run's requests, never stored; `None` for none.
    pub jar: Option<imposter_http::cookies::SharedJar>,
    /// The OAuth 2.0 tokens it may use and add to: the stored ones for Send, which keeps a
    /// new one afterwards ([`execute`]); one cache shared by a run's requests, never
    /// stored. `None` starts empty.
    pub tokens: Option<crate::oauth::SharedTokens>,
}

/// Where an execution's captures and script variables go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writes {
    /// Back to the stored scopes that defined them, as Send does.
    Scopes,
    /// Only into the run: a file has no stored scopes, and a command-line run against the
    /// profile must not change the user's variables behind their back.
    Run,
}

impl Inputs {
    fn destination(&self) -> Destination<'_> {
        match self.writes {
            Writes::Scopes => Destination::Scopes(&self.snapshot, &self.context),
            Writes::Run => Destination::Run,
        }
    }
}

/// Which environment's values an execution reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentChoice {
    /// The one active in the app, as Send uses.
    Active,
    /// This one, or none (globals and collection only), whatever the app has active.
    Chosen(Option<imposter_core::ids::EnvironmentId>),
}

/// Read the inputs for `request`. Inspection uses this too, so it explains exactly what an
/// execution would use.
pub async fn inputs(store: &StoreHandle, request: &Request) -> Result<Inputs, StoreError> {
    let request = request.clone();
    store
        .call(move |db| read_inputs(db, &request, EnvironmentChoice::Active))
        .await
}

/// [`inputs`] inside a database command, with `environment` chosen. Captures go back to
/// their scopes; a caller that keeps them in the run sets [`Inputs::writes`].
pub fn read_inputs(
    db: &imposter_store::Tx<'_>,
    request: &Request,
    environment: EnvironmentChoice,
) -> Result<Inputs, StoreError> {
    let collection_id = request.collection_id;
    let folder_id = request.folder_id;
    let captured = &request.collection_variables_snapshot;
    {
        {
            let settings = kv::app_settings(db)?;
            let context = match environment {
                EnvironmentChoice::Active => environments::variable_context(db)?,
                EnvironmentChoice::Chosen(id) => environments::variable_context_for(db, id)?,
            };
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
                    id: Some(collection.id),
                    name: &collection.name,
                    values: &collection.variables,
                    saved: &saved,
                }),
                None if !captured.is_empty() || collection_id.is_some() => {
                    Some(CollectionScope::Deleted { captured })
                }
                None => None,
            };
            let snapshot = Snapshot::build(&context, scope);
            let jar = crate::cookies::read(db, context.environment_id())?;
            let tokens = imposter_store::oauth::read(db, context.environment_id())?;
            let scripts = match &collection {
                Some(collection) => script_levels(db, collection, folder_id)?,
                None => crate::scripts::ScriptChain::default(),
            };
            Ok(Inputs {
                settings,
                context,
                snapshot,
                scripts,
                writes: Writes::Scopes,
                base_dir: Some(db.folder()),
                jar: Some(imposter_http::cookies::SharedJar::new(jar)),
                tokens: Some(crate::oauth::SharedTokens::new(tokens)),
            })
        }
    }
}

/// The collection's scripts, then those of each folder from the outermost down to the
/// request's own folder. A folder that no longer exists, or belongs elsewhere, adds none.
pub(crate) fn script_levels(
    db: &imposter_store::Tx<'_>,
    collection: &collections::Collection,
    folder: Option<imposter_core::ids::FolderId>,
) -> Result<crate::scripts::ScriptChain, StoreError> {
    use crate::scripts::Level;
    let mut folders = Vec::new();
    let mut next = folder;
    while let Some(id) = next {
        let Some(found) = collections::folder(db, id)? else {
            break;
        };
        // Parents form a tree (moves refuse cycles); the bound only guards a corrupt file.
        if found.collection_id != collection.id || folders.len() > 1000 {
            break;
        }
        next = found.parent_id;
        folders.push(Level {
            name: format!("Folder \"{}\"", found.name),
            scripts: found.scripts,
        });
    }
    let mut levels = vec![Level {
        name: format!("Collection \"{}\"", collection.name),
        scripts: collection.scripts.clone(),
    }];
    levels.extend(folders.into_iter().rev());
    Ok(crate::scripts::ScriptChain(levels))
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
    /// No OAuth 2.0 token could be had, in words ("Token request failed: `invalid_client`").
    /// The request was not sent.
    #[error("{0}")]
    Token(String),
    /// A person must sign in first (OAuth 2.0 authorization code), which a send never does.
    /// `forget` names the cached token that no longer works, for Send to drop.
    #[error("Sign in to get a token: {reason}")]
    SignIn {
        reason: String,
        forget: Option<String>,
    },
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

/// Write captured values to their stored scopes, one transaction for all of them. Values
/// that belong to the run only are not written.
pub async fn write_captures(store: &StoreHandle, writes: Vec<Write>) -> Result<(), StoreError> {
    store
        .call(move |db| {
            let mut secret_scopes = Vec::new();
            for scope in writes.iter().filter_map(Write::secret_scope) {
                if !secret_scopes.contains(&scope) {
                    secret_scopes.push(scope);
                }
            }
            for scope in secret_scopes {
                let values: Vec<(String, String)> = writes
                    .iter()
                    .filter(|w| w.secret_scope() == Some(scope))
                    .map(|w| (w.name.clone(), w.value.clone()))
                    .collect();
                imposter_store::secrets::set(db, scope, &values)?;
            }
            for scope in imposter_core::capture::scopes(&writes) {
                let values: Vec<(String, String)> = writes
                    .iter()
                    .filter(|w| !w.secret && w.scope == Some(scope))
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
    let mut settings = settings.resolve(&template.options);
    let mut template = template;
    if let Some(base) = &inputs.base_dir {
        root_template(&mut template, base);
        root_settings(&mut settings, base);
    }
    let resolved = Request {
        template,
        collection_variables_snapshot: inputs.snapshot.collection_values.clone(),
        ..request.clone()
    };
    Ok((resolved, settings, values))
}

/// `path` under `base` when it is relative; empty and absolute paths stay as they are.
/// Joined part by part, so the result has the OS's separators throughout: a stored
/// `fixtures/data.bin` reads `C:\api\fixtures\data.bin` on Windows in curl and History.
fn rooted(path: &str, base: &std::path::Path) -> Option<String> {
    let path = std::path::Path::new(path);
    let relative = !path.as_os_str().is_empty() && path.is_relative();
    relative.then(|| {
        let mut joined = base.to_path_buf();
        joined.extend(path.components());
        joined.display().to_string()
    })
}

/// Join the request's file paths to `base` when relative: a binary body and multipart
/// files. Run again after the before-sending scripts, which may set paths of their own.
pub fn root_template(template: &mut RequestTemplate, base: &std::path::Path) {
    use imposter_core::model::{BodyType, FormKind};
    match template.body_type {
        BodyType::Binary => {
            if let Some(path) = std::str::from_utf8(&template.body)
                .ok()
                .and_then(|path| rooted(path, base))
            {
                template.body = path.into_bytes();
            }
        }
        BodyType::Multipart => {
            if let Ok(mut fields) = template.multipart_fields() {
                let mut changed = false;
                for field in fields.iter_mut().filter(|f| f.kind == FormKind::File) {
                    if let Some(path) = rooted(&field.value, base) {
                        field.value = path;
                        changed = true;
                    }
                }
                if changed {
                    template.set_multipart_fields(&fields);
                }
            }
        }
        _ => {}
    }
}

/// Join the TLS file settings to `base` when relative: the CA bundle, the client
/// certificate and its key, and every host identity's.
pub fn root_settings(settings: &mut AppSettings, base: &std::path::Path) {
    for path in [
        &mut settings.ca_bundle,
        &mut settings.certificate_path,
        &mut settings.key_file,
    ] {
        if let Some(rooted) = rooted(path, base) {
            *path = rooted;
        }
    }
    for identity in &mut settings.host_identities {
        for path in [&mut identity.certificate_path, &mut identity.key_file] {
            if let Some(rooted) = rooted(path, base) {
                *path = rooted;
            }
        }
    }
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
    let chain = inputs.scripts.with_request(request).compile()?;
    let (resolved, settings, values) =
        resolve_with(request, &inputs, &mut SystemEntropy::default())?;
    // What Send would send includes the before-sending scripts' changes.
    let mut before = chain
        .before(resolved.template, values, CancellationToken::new())
        .await
        .map_err(ExecuteError::Script)?;
    if let Some(base) = &inputs.base_dir {
        root_template(&mut before.template, base);
    }
    use_cached_token(&inputs, &mut before.template);
    add_jar_cookies(&inputs, &settings, &mut before.template);
    let prepared = prepare(&before.template, settings.attach_cookies)
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

/// The jar's cookies for the first location, put into `template` as sent, so History, the
/// response pane and Copy as curl show them; the transport adds those of later hops. Only
/// while the settings keep cookies and attach them.
fn add_jar_cookies(inputs: &Inputs, settings: &AppSettings, template: &mut RequestTemplate) {
    if let Some(jar) = inputs
        .jar
        .as_ref()
        .filter(|_| settings.cookie_jar && settings.attach_cookies)
        && let Ok(url) = imposter_core::prepare::request_url(template)
    {
        let cookies = jar.with(|jar| jar.cookies_for(&url));
        crate::cookies::add_to(template, cookies);
    }
}

/// A recorded execution as the curl command of what it sent.
pub fn recorded_curl(record: &ExecutionRecord) -> Result<String, ExecuteError> {
    let prepared = prepare(&record.resolved.template, record.settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    Ok(imposter_core::curl::to_command(&prepared, &record.settings))
}

/// A finished execution, not yet recorded anywhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    /// What History records: the request as stored and as sent, settings, environment and
    /// response.
    pub record: ExecutionRecord,
    /// Values the request's captures and scripts set, to write where `Inputs::origin` says.
    pub writes: Vec<Write>,
    /// The secret values it used, captured or set: History keeps none of them.
    pub secrets: Vec<String>,
    /// The OAuth 2.0 token fetched for it, for Send to keep.
    pub token: Option<imposter_store::oauth::StoredToken>,
}

/// Send `request` and record it. `cancel` stops the network exchange; a cancelled execution
/// is still recorded, like any other outcome.
pub async fn execute(
    store: &StoreHandle,
    request: Request,
    cancel: CancellationToken,
) -> Result<Executed, ExecuteError> {
    // Boxed for the same reason as [`run`].
    Box::pin(execute_boxed(store, request, cancel)).await
}

async fn execute_boxed(
    store: &StoreHandle,
    request: Request,
    cancel: CancellationToken,
) -> Result<Executed, ExecuteError> {
    let inputs = inputs(store, &request).await?;
    let ran = match run(&inputs, request, cancel).await {
        Ok(ran) => ran,
        Err(ExecuteError::SignIn {
            reason,
            forget: Some(key),
        }) => {
            // The Auth tab then says there is no token, rather than an expired one.
            let environment = inputs.context.environment_id();
            let forget = key.clone();
            if let Err(error) = store
                .call(move |db| imposter_store::oauth::remove(db, environment, &forget))
                .await
            {
                tracing::warn!(%error, "forgetting an expired token failed");
            }
            return Err(ExecuteError::SignIn {
                reason,
                forget: Some(key),
            });
        }
        Err(error) => return Err(error),
    };
    let cookies = ran.record.response.set_cookies.clone();
    let token = ran.token.clone();
    let environment = inputs.context.environment_id();
    let kept = if cookies.is_empty() && token.is_none() {
        Ok(())
    } else {
        store
            .call(move |db| {
                if let Some(token) = &token {
                    imposter_store::oauth::put(db, environment, token)?;
                }
                crate::cookies::keep(db, environment, &cookies)
            })
            .await
    };
    let mut executed = record(store, ran).await;
    if let Err(error) = kept {
        tracing::warn!(%error, "keeping a response's cookies failed");
        if executed.response.history_error.is_empty() {
            COOKIES_ERROR.clone_into(&mut executed.response.history_error);
        }
    }
    Ok(executed)
}

pub const COOKIES_ERROR: &str =
    "Response received, but the cookies it set or its new token could not be kept";

/// Record `ran` in History under the retention limits its settings carry. A failed write
/// keeps the response and says so (`history_error`).
pub async fn record(store: &StoreHandle, ran: Ran) -> Executed {
    // Boxed for the same reason as [`run`]: a record holds the request twice.
    Box::pin(record_boxed(store, ran)).await
}

async fn record_boxed(store: &StoreHandle, ran: Ran) -> Executed {
    let Ran {
        mut record,
        writes,
        secrets,
        token: _,
    } = ran;
    // What is shown now keeps its values; what is stored keeps no secret.
    let mut response = record.response.clone();
    let sent = Box::new(record.resolved.clone());
    let redactor = history_redactor(&secrets);
    if !redactor.is_empty() || record.response.captures.iter().any(|c| c.secret) {
        redactor.record(&mut record);
    }
    let attach_cookies = record.settings.attach_cookies;
    let (entries, bytes) = (
        record.settings.history_max_entries,
        record.settings.history_max_bytes,
    );
    let id = match store
        .call(move |db| executions::record(db, &record, entries, bytes))
        .await
    {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!(%error, "recording an execution failed");
            HISTORY_ERROR.clone_into(&mut response.history_error);
            None
        }
    };
    Executed {
        response,
        record: id,
        writes,
        sent,
        attach_cookies,
    }
}

/// What History removes: each secret long enough to find safely (`redact::MIN_SECRET`); a
/// secret capture's own value goes whole whatever its length.
fn history_redactor(secrets: &[String]) -> imposter_core::redact::Redactor {
    let mut redactor = imposter_core::redact::Redactor::default();
    for secret in secrets
        .iter()
        .filter(|s| s.chars().count() >= imposter_core::redact::MIN_SECRET)
    {
        redactor.secret(secret);
    }
    redactor
}

/// Run `request` with `inputs`: before-sending scripts, prepare, send, checks, captures and
/// after-response scripts. Nothing is read or written here, so Send and a headless run
/// share every step. `cancel` stops scripts and the network exchange.
pub async fn run(
    inputs: &Inputs,
    request: Request,
    cancel: CancellationToken,
) -> Result<Ran, ExecuteError> {
    // Boxed: scripts, the token request and the exchange keep a large state, and inline it
    // pushed every caller's future past clippy's `large_futures` limit.
    Box::pin(run_boxed(inputs, request, cancel)).await
}

async fn run_boxed(
    inputs: &Inputs,
    request: Request,
    cancel: CancellationToken,
) -> Result<Ran, ExecuteError> {
    let chain = inputs.scripts.with_request(&request).compile()?;
    let (mut resolved, settings, mut values) =
        resolve_with(&request, inputs, &mut SystemEntropy::default())?;
    let before = chain
        .before(resolved.template, values.clone(), cancel.clone())
        .await
        .map_err(ExecuteError::Script)?;
    resolved.template = before.template;
    if let Some(base) = &inputs.base_dir {
        root_template(&mut resolved.template, base);
    }
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing = Some(Box::new(imposter_http::transport::IdentityRouting {
        settings: inputs.settings.clone(),
        options: request.template.options.clone(),
        variables: values.clone(),
        base_dir: inputs.base_dir.clone(),
    }));
    let mut secrets: Vec<String> = Vec::new();
    let token = oauth_token(
        inputs,
        &policy,
        &cancel,
        &mut resolved.template,
        &mut secrets,
    )
    .await?;
    add_jar_cookies(inputs, &settings, &mut resolved.template);
    let prepared = prepare(&resolved.template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;

    let started = Instant::now();
    policy.jar = inputs.jar.clone().filter(|_| settings.cookie_jar);
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
    let destination = inputs.destination();
    let (captured, mut writes) =
        imposter_core::capture::evaluate(&request.template.captures, &response, destination);
    response.captures = captured;
    // The after-response script sees the before-sending script's sets, and runs only on a
    // response, as checks do.
    let mut sets = before.sets;
    let mut secret_sets = before.secret_sets;
    response.console = before.console;
    if chain.has_after() && response.outcome == Outcome::Response {
        for (name, value) in &sets {
            values.insert(name.clone(), value.clone());
        }
        for (name, value) in &secret_sets {
            values.insert(imposter_core::variables::secret_key(name), value.clone());
        }
        let after = chain
            .after(
                resolved.template.clone(),
                response.clone(),
                values.clone(),
                cancel.clone(),
            )
            .await;
        response.console.extend(after.console);
        response.script = Some(imposter_core::execution::ScriptReport {
            tests: after.tests,
            // One line per script that failed, each naming its script.
            error: after.errors.join("\n"),
        });
        sets.extend(after.sets);
        secret_sets.extend(after.secret_sets);
    }
    // Scripts set variables where captures would, after the captures.
    writes.extend(sets.into_iter().map(|(name, value)| Write {
        scope: destination.of(&name).0,
        name,
        value,
        secret: false,
    }));
    writes.extend(secret_sets.into_iter().map(|(name, value)| Write {
        scope: destination.of_secret(&name).0,
        name,
        value,
        secret: true,
    }));
    secrets.extend(inputs.snapshot.secret_values().map(str::to_owned));
    secrets.extend(writes.iter().filter(|w| w.secret).map(|w| w.value.clone()));

    let mut template = request.clone();
    template
        .collection_variables_snapshot
        .clone_from(&resolved.collection_variables_snapshot);
    let record = ExecutionRecord {
        timestamp: now(),
        request_id: request.id,
        template,
        resolved,
        settings,
        environment_id: inputs.context.environment_id(),
        environment: inputs.context.environment_name().to_owned(),
        response,
        via: imposter_core::execution::Via::App,
    };
    Ok(Ran {
        record,
        writes,
        secrets,
        token,
    })
}

/// With OAuth 2.0 in effect, send the cached token while it is good, as Copy as curl does:
/// copying never asks the token endpoint for one. Without one, `prepare` says to get it.
fn use_cached_token(inputs: &Inputs, template: &mut RequestTemplate) {
    let Ok(Some(imposter_core::model::Auth::OAuth2(oauth))) = template.auth.active() else {
        return;
    };
    let key = crate::oauth::key(oauth);
    if let Some(token) = inputs
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.get(&key))
        .filter(|token| crate::oauth::good(token, now().0))
    {
        template.auth = imposter_core::model::RequestAuth::bearer(&token.access);
    }
}

/// With OAuth 2.0 in effect, get its token (`oauth::token`) and put it in `template` as the
/// Bearer token it is sent as, so the request as sent shows a Bearer token, which History
/// removes like the client secret and refresh token (added to `secrets`). The token
/// fetched now, if any, is returned for Send to keep.
async fn oauth_token(
    inputs: &Inputs,
    policy: &Policy,
    cancel: &CancellationToken,
    template: &mut RequestTemplate,
    secrets: &mut Vec<String>,
) -> Result<Option<imposter_store::oauth::StoredToken>, ExecuteError> {
    let Ok(Some(imposter_core::model::Auth::OAuth2(oauth))) = template.auth.active() else {
        return Ok(None);
    };
    let oauth = oauth.clone();
    secrets.push(oauth.client_secret.clone());
    let tokens = inputs.tokens.clone().unwrap_or_default();
    let obtained = crate::oauth::token(&oauth, &tokens, policy, cancel, now().0)
        .await
        .map_err(|failure| match failure {
            crate::oauth::TokenFailure::Request(imposter_http::oauth::OAuthError::Cancelled) => {
                ExecuteError::Token("Token request cancelled".to_owned())
            }
            crate::oauth::TokenFailure::Request(error) => {
                ExecuteError::Token(format!("Token request failed: {error}"))
            }
            crate::oauth::TokenFailure::SignIn { reason, forget } => {
                ExecuteError::SignIn { reason, forget }
            }
        })?;
    let token = obtained.token;
    secrets.push(token.access.clone());
    secrets.extend(token.refresh.clone());
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err(ExecuteError::Token(format!(
            "The token endpoint gave a {} token; only Bearer tokens can be sent",
            token.token_type
        )));
    }
    template.auth = imposter_core::model::RequestAuth::bearer(&token.access);
    Ok(obtained.fetched.then_some(token))
}
