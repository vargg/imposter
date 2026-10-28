//! Execution history: independent records, retention, settings and UI state storage.
#![cfg(test)]

mod support;

use imposter_core::execution::{ExecutionRecord, Outcome, Response, Timestamp};
use imposter_core::ids::{ExecutionId, RequestId};
use imposter_core::model::{Request, Variables};
use imposter_core::settings::{AppSettings, SettingsError};
use imposter_store::StoreError;
use imposter_store::collections::{create_collection, create_request, delete_request};
use imposter_store::executions::*;
use imposter_store::kv::{self, Table};
use support::{store, template};

fn record(request: Option<RequestId>, body: usize, at: i64) -> ExecutionRecord {
    let request_value = Request {
        id: request,
        template: template("req"),
        ..Request::default()
    };
    ExecutionRecord {
        timestamp: Timestamp(at),
        request_id: request,
        template: request_value.clone(),
        resolved: request_value,
        settings: AppSettings::default(),
        environment_id: None,
        environment: "Global".into(),
        response: Response {
            status: 200,
            body: vec![b'x'; body],
            elapsed: 0.25,
            ..Response::default()
        },
    }
}

#[test]
fn records_round_trip_and_summaries_skip_payloads() {
    let (_dir, store) = store();
    let mut stored = record(Some(RequestId(7)), 10, 1_000);
    stored.response.outcome = Outcome::Cancelled;
    stored.response.truncated = true;
    let expected = stored.clone();

    let id = store
        .call_blocking(move |db| record_execution(db, &stored))
        .unwrap();

    assert_eq!(
        store.call_blocking(move |db| execution(db, id)).unwrap(),
        Some(expected)
    );
    let summary = store
        .call_blocking(|db| page(db, None, 0, 10))
        .unwrap()
        .remove(0);
    assert_eq!(
        (
            summary.id,
            summary.request_id,
            summary.status,
            summary.outcome
        ),
        (id, Some(RequestId(7)), 200, Outcome::Cancelled)
    );
    assert_eq!(
        (
            summary.method.as_str(),
            summary.url.as_str(),
            summary.truncated
        ),
        ("GET", "https://example.com/req", true)
    );
    assert_eq!(
        store
            .call_blocking(|db| execution(db, ExecutionId(999)))
            .unwrap(),
        None
    );
}

fn record_execution(
    db: &rusqlite::Connection,
    value: &ExecutionRecord,
) -> Result<ExecutionId, StoreError> {
    imposter_store::executions::record(db, value, 1000, 250 * 1024 * 1024)
}

#[test]
fn history_survives_deleting_its_request() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            let r = create_request(db, c.id, None, &template("req"))?
                .id
                .unwrap();
            record_execution(db, &record(Some(r), 1, 0))?;
            delete_request(db, r)
        })
        .unwrap();
    assert_eq!(store.call_blocking(|db| count(db)).unwrap(), 1);
}

#[test]
fn pages_are_newest_first_and_filter_by_request() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            for (i, request) in [Some(1), Some(2), None, Some(1)].into_iter().enumerate() {
                record_execution(
                    db,
                    &record(request.map(RequestId), 1, i64::try_from(i).unwrap()),
                )?;
            }
            Ok(())
        })
        .unwrap();

    let all: Vec<_> = store
        .call_blocking(|db| page(db, None, 0, 10))
        .unwrap()
        .into_iter()
        .map(|s| s.timestamp.0)
        .collect();
    assert_eq!(all, [3, 2, 1, 0]);
    let first_page: Vec<_> = store
        .call_blocking(|db| page(db, None, 1, 2))
        .unwrap()
        .into_iter()
        .map(|s| s.timestamp.0)
        .collect();
    assert_eq!(first_page, [2, 1]);
    let one: Vec<_> = store
        .call_blocking(|db| page(db, Some(RequestId(1)), 0, 10))
        .unwrap()
        .into_iter()
        .map(|s| s.timestamp.0)
        .collect();
    assert_eq!(one, [3, 0]);
}

#[test]
fn retention_drops_the_oldest_beyond_the_entry_limit() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            for i in 0..5 {
                imposter_store::executions::record(db, &record(None, 1, i), 3, u64::MAX)?;
            }
            Ok(())
        })
        .unwrap();
    let kept: Vec<_> = store
        .call_blocking(|db| page(db, None, 0, 10))
        .unwrap()
        .into_iter()
        .map(|s| s.timestamp.0)
        .collect();
    assert_eq!(kept, [4, 3, 2]);
}

#[test]
fn retention_drops_the_oldest_beyond_the_byte_budget() {
    let (_dir, store) = store();
    let size = serde_json::to_string(&record(None, 10_000, 0))
        .unwrap()
        .len() as u64;
    store
        .call_blocking(move |db| {
            for i in 0..4 {
                imposter_store::executions::record(
                    db,
                    &record(None, 10_000, i),
                    100,
                    size * 2 + size / 2,
                )?;
            }
            Ok(())
        })
        .unwrap();
    let kept: Vec<_> = store
        .call_blocking(|db| page(db, None, 0, 10))
        .unwrap()
        .into_iter()
        .map(|s| s.timestamp.0)
        .collect();
    assert_eq!(kept, [3, 2]);
}

#[test]
fn a_record_larger_than_the_budget_is_refused_and_nothing_is_dropped() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| record_execution(db, &record(None, 1, 0)).map(drop))
        .unwrap();

    let result = store.call_blocking(|db| {
        imposter_store::executions::record(db, &record(None, 5000, 1), 10, 2048)
    });

    assert!(matches!(result, Err(StoreError::HistoryBudget)));
    assert_eq!(store.call_blocking(|db| count(db)).unwrap(), 1);
}

#[test]
fn retention_can_be_applied_when_settings_shrink_and_clear_empties_history() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            for i in 0..4 {
                record_execution(db, &record(None, 1, i))?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store
            .call_blocking(|db| enforce_retention(db, 1, u64::MAX))
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .call_blocking(|db| enforce_retention(db, 1, u64::MAX))
            .unwrap(),
        0
    );
    store.call_blocking(|db| clear(db)).unwrap();
    assert_eq!(store.call_blocking(|db| count(db)).unwrap(), 0);
}

#[test]
fn settings_default_until_saved_and_are_validated() {
    let (_dir, store) = store();
    assert_eq!(
        store.call_blocking(|db| kv::app_settings(db)).unwrap(),
        AppSettings::default()
    );

    let custom = AppSettings {
        timeout: 5.0,
        autosave: false,
        ..AppSettings::default()
    };
    store
        .call_blocking({
            let c = custom.clone();
            move |db| kv::set_app_settings(db, &c)
        })
        .unwrap();
    assert_eq!(
        store.call_blocking(|db| kv::app_settings(db)).unwrap(),
        custom
    );

    let invalid = AppSettings {
        timeout: 0.0,
        ..AppSettings::default()
    };
    let result = store.call_blocking(move |db| kv::set_app_settings(db, &invalid));
    assert!(matches!(
        result,
        Err(StoreError::Settings(SettingsError::Timeout))
    ));
}

#[test]
fn ui_state_stores_json_by_key() {
    let (_dir, store) = store();
    let value = serde_json::json!({"version": 1, "sidebar_width": 240});
    store
        .call_blocking({
            let v = value.clone();
            move |db| kv::set(db, Table::UiState, "workspace_layout", &v)
        })
        .unwrap();

    assert_eq!(
        store
            .call_blocking(|db| kv::get::<serde_json::Value>(
                db,
                Table::UiState,
                "workspace_layout"
            ))
            .unwrap(),
        Some(value)
    );
    store
        .call_blocking(|db| kv::remove(db, Table::UiState, "workspace_layout"))
        .unwrap();
    assert_eq!(
        store
            .call_blocking(|db| kv::get::<serde_json::Value>(
                db,
                Table::UiState,
                "workspace_layout"
            ))
            .unwrap(),
        None
    );
}
