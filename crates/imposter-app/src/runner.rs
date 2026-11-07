//! Running several requests as one run: each through `executor::run`, in order, with the
//! values earlier requests captured or set layered above every scope (`Source::Run`), so a
//! login's token reaches the requests after it. By default the run stops at the first
//! request that does not pass; the rest are skipped. Nothing is written to the user's
//! variables; a run from the profile records each request in History as the command line's.

use imposter_core::execution::{ExecutionRecord, Via};
use imposter_core::model::Variables;
use imposter_core::run_result::RunOutcome;
use imposter_core::variables::Source;
use imposter_store::StoreHandle;
use tokio_util::sync::CancellationToken;

use crate::document::Selected;
use crate::executor::{self, ExecuteError, Ran};

#[derive(Clone, Default)]
pub struct Options {
    /// Run every request even after one did not pass.
    pub keep_going: bool,
    /// Record each request in this profile's History.
    pub record: Option<StoreHandle>,
}

/// How one request of the run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// It was sent (or the attempt failed on the wire, or was cancelled).
    Ran {
        path: String,
        record: Box<ExecutionRecord>,
        /// A History write that failed.
        notes: Vec<String>,
    },
    /// It could not be sent: an unresolved variable, a bad URL, a before-sending script.
    NotSent {
        path: String,
        outcome: RunOutcome,
        error: String,
    },
    Skipped {
        path: String,
        reason: String,
    },
}

impl Step {
    /// The request's own outcome; `None` when it was skipped.
    pub fn outcome(&self) -> Option<RunOutcome> {
        match self {
            Self::Ran { record, .. } => Some(RunOutcome::of(&record.response)),
            Self::NotSent { outcome, .. } => Some(*outcome),
            Self::Skipped { .. } => None,
        }
    }
}

pub const SKIPPED_AFTER_FAILURE: &str = "an earlier request did not pass";
pub const SKIPPED_AFTER_CANCEL: &str = "the run was cancelled";

/// Run `requests` in order. `on_step` sees each step as it ends, so progress can be shown
/// while the run goes on.
pub async fn run(
    requests: Vec<Selected>,
    options: Options,
    cancel: CancellationToken,
    mut on_step: impl FnMut(&Step),
) -> Vec<Step> {
    let mut carried = Variables::new();
    let mut done = Vec::with_capacity(requests.len());
    let mut halted: Option<&'static str> = None;
    for selected in requests {
        let path = selected.path.to_string();
        if halted.is_none() && cancel.is_cancelled() {
            halted = Some(SKIPPED_AFTER_CANCEL);
        }
        let step = if let Some(reason) = halted {
            Step::Skipped {
                path,
                reason: reason.to_owned(),
            }
        } else {
            let mut inputs = selected.inputs;
            inputs.snapshot = inputs.snapshot.with_layer(&carried, &Source::Run);
            match executor::run(&inputs, selected.request, cancel.clone()).await {
                Ok(ran) => {
                    for write in &ran.writes {
                        carried.insert(write.name.clone(), write.value.clone());
                    }
                    record(path, ran, options.record.as_ref()).await
                }
                Err(error) => Step::NotSent {
                    path,
                    outcome: if cancel.is_cancelled() {
                        RunOutcome::Cancelled
                    } else if matches!(error, ExecuteError::Script(_)) {
                        RunOutcome::ScriptFailed
                    } else {
                        RunOutcome::NotSent
                    },
                    error: error.to_string(),
                },
            }
        };
        if halted.is_none()
            && let Some(outcome) = step.outcome()
        {
            if outcome == RunOutcome::Cancelled {
                halted = Some(SKIPPED_AFTER_CANCEL);
            } else if !outcome.continues_run() && !options.keep_going {
                halted = Some(SKIPPED_AFTER_FAILURE);
            }
        }
        on_step(&step);
        done.push(step);
    }
    done
}

async fn record(path: String, mut ran: Ran, store: Option<&StoreHandle>) -> Step {
    let mut notes = Vec::new();
    if let Some(store) = store {
        ran.record.via = Via::CommandLine;
        let copy = ran.record.clone();
        let executed = executor::record(
            store,
            Ran {
                record: copy,
                writes: Vec::new(),
            },
        )
        .await;
        if executed.record.is_none() {
            notes.push(executed.response.history_error);
        }
    }
    Step::Ran {
        path,
        record: Box::new(ran.record),
        notes,
    }
}
