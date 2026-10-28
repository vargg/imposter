//! Executing a request: snapshot, resolve, validate, send, then record history without ever
//! losing the response.

use std::time::Instant;

use poster_core::execution::{ExecutionRecord, Outcome, Response, Timestamp};
use poster_core::ids::ExecutionId;
use poster_core::model::Request;
use poster_core::prepare::prepare;
use poster_core::settings::AppSettings;
use poster_core::variables::{CollectionScope, Snapshot, VariableContext, resolve_template};
use poster_http::{Policy, SendError};
use poster_store::{StoreError, StoreHandle, collections, environments, executions, kv};
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
            let scope = match &collection {
                Some(collection) => Some(CollectionScope::Live {
                    id: collection.id,
                    name: &collection.name,
                    values: &collection.variables,
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
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Executed {
    pub response: Response,
    /// The history entry, unless recording failed (see `response.history_error`).
    pub record: Option<ExecutionId>,
}

pub const HISTORY_ERROR: &str = "Response received, but history could not be saved";

/// Current wall-clock time for history timestamps.
pub fn now() -> Timestamp {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Resolve `request` against `inputs`: inherited settings become explicit options first, so
/// variables inside inherited paths resolve against the same snapshot as the request.
pub fn resolve(request: &Request, inputs: &Inputs) -> Result<(Request, AppSettings), ExecuteError> {
    let values = inputs.snapshot.values();
    let mut missing = std::collections::BTreeSet::new();
    let url = poster_core::variables::substitute(
        &request.template.url,
        |name| values.get(name).map(String::as_str),
        &mut missing,
    );
    let settings = inputs
        .settings
        .resolve_for_url(&url, &request.template.options);
    let mut template = request.template.clone();
    template.options = settings.as_options();
    let template =
        resolve_template(&template, &values).map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    let settings = settings.resolve(&template.options);
    let resolved = Request {
        template,
        collection_variables_snapshot: inputs.snapshot.collection_values.clone(),
        ..request.clone()
    };
    Ok((resolved, settings))
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
    let (resolved, settings) = resolve(request, &inputs)?;
    let prepared = prepare(&resolved.template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    Ok((
        poster_core::curl::to_command(&prepared, &settings),
        !settings.host_identities.is_empty(),
    ))
}

/// A recorded execution as the curl command of what it sent.
pub fn recorded_curl(record: &ExecutionRecord) -> Result<String, ExecuteError> {
    let prepared = prepare(&record.resolved.template, record.settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;
    Ok(poster_core::curl::to_command(&prepared, &record.settings))
}

/// Send `request` and record it. `cancel` stops the network exchange; a cancelled execution
/// is still recorded, like any other outcome.
pub async fn execute(
    store: &StoreHandle,
    request: Request,
    cancel: CancellationToken,
) -> Result<Executed, ExecuteError> {
    let inputs = inputs(store, &request).await?;
    let (resolved, settings) = resolve(&request, &inputs)?;
    let prepared = prepare(&resolved.template, settings.attach_cookies)
        .map_err(|e| ExecuteError::Invalid(e.to_string()))?;

    let started = Instant::now();
    let mut policy = Policy::from_settings(&settings);
    policy.identity_routing = Some(Box::new(poster_http::transport::IdentityRouting {
        settings: inputs.settings.clone(),
        options: request.template.options.clone(),
        variables: inputs.snapshot.values(),
    }));
    let mut response = match poster_http::send(&prepared, &policy, &cancel).await {
        Ok(response) => response,
        Err(SendError::Cancelled) => {
            Response::failure(Outcome::Cancelled, "Cancelled", "Request cancelled")
        }
        Err(SendError::Failed { kind, message }) => {
            Response::failure(Outcome::TransportError, kind.as_str(), message)
        }
    };
    response.elapsed = started.elapsed().as_secs_f64();

    let mut template = request.clone();
    template.collection_variables_snapshot = resolved.collection_variables_snapshot.clone();
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
        }),
        Err(error) => {
            tracing::warn!(%error, "recording an execution failed");
            HISTORY_ERROR.clone_into(&mut response.history_error);
            Ok(Executed {
                response,
                record: None,
            })
        }
    }
}
