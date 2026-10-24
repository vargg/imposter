//! Execution history with retention by count and by stored size, oldest first.

use poster_core::execution::{ExecutionRecord, Outcome, Timestamp};
use poster_core::ids::{ExecutionId, RequestId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::StoreError;

/// What the history list shows, read without decoding payloads.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionSummary {
    pub id: ExecutionId,
    pub request_id: Option<RequestId>,
    pub timestamp: Timestamp,
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub outcome: Outcome,
    pub elapsed: f64,
    pub environment: String,
    pub truncated: bool,
}

fn outcome_text(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Response => "response",
        Outcome::TransportError => "transport_error",
        Outcome::Cancelled => "cancelled",
    }
}

fn outcome_from(text: &str) -> Outcome {
    match text {
        "transport_error" => Outcome::TransportError,
        "cancelled" => Outcome::Cancelled,
        _ => Outcome::Response,
    }
}

/// Store one execution, then drop the oldest beyond `max_entries` or `max_bytes` in total.
/// A single record larger than `max_bytes` is refused.
pub fn record(
    db: &Connection,
    record: &ExecutionRecord,
    max_entries: u64,
    max_bytes: u64,
) -> Result<ExecutionId, StoreError> {
    let payload = serde_json::to_string(record)?;
    let size = payload.len() as u64;
    if size > max_bytes {
        return Err(StoreError::HistoryBudget);
    }
    db.execute(
        "INSERT INTO executions
           (request_id, timestamp, name, method, url, status, outcome, elapsed, environment,
            truncated, payload, payload_size)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            record.request_id.map(|id| id.0),
            record.timestamp.0,
            record.template.template.name,
            record.resolved.template.method,
            record.resolved.template.url,
            record.response.status,
            outcome_text(record.response.outcome),
            record.response.elapsed,
            record.environment,
            record.response.truncated,
            payload,
            i64::try_from(size).unwrap_or(i64::MAX),
        ],
    )?;
    let id = ExecutionId(db.last_insert_rowid());
    enforce_retention(db, max_entries, max_bytes)?;
    Ok(id)
}

/// Delete the oldest executions until at most `max_entries` remain and their payloads fit
/// in `max_bytes`.
pub fn enforce_retention(
    db: &Connection,
    max_entries: u64,
    max_bytes: u64,
) -> Result<usize, StoreError> {
    let entries = i64::try_from(max_entries).unwrap_or(i64::MAX);
    let bytes = i64::try_from(max_bytes).unwrap_or(i64::MAX);
    let cutoff: Option<i64> = db
        .query_row(
            "SELECT id FROM (
               SELECT id,
                      row_number() OVER (ORDER BY id DESC) AS position,
                      sum(payload_size) OVER (ORDER BY id DESC) AS total
               FROM executions)
             WHERE position > ?1 OR total > ?2
             ORDER BY id DESC LIMIT 1",
            params![entries, bytes],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match cutoff {
        Some(cutoff) => db.execute("DELETE FROM executions WHERE id <= ?1", [cutoff])?,
        None => 0,
    })
}

/// Newest first, optionally for one request.
pub fn page(
    db: &Connection,
    request: Option<RequestId>,
    offset: u64,
    limit: u64,
) -> Result<Vec<ExecutionSummary>, StoreError> {
    let mut statement = db.prepare(
        "SELECT id, request_id, timestamp, name, method, url, status, outcome, elapsed, environment, truncated
         FROM executions WHERE (?1 IS NULL OR request_id = ?1)
         ORDER BY id DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = statement.query_map(
        params![
            request.map(|id| id.0),
            i64::try_from(limit).unwrap_or(i64::MAX),
            i64::try_from(offset).unwrap_or(i64::MAX)
        ],
        |row| {
            Ok(ExecutionSummary {
                id: ExecutionId(row.get(0)?),
                request_id: row.get::<_, Option<i64>>(1)?.map(RequestId),
                timestamp: Timestamp(row.get(2)?),
                name: row.get(3)?,
                method: row.get(4)?,
                url: row.get(5)?,
                status: row.get(6)?,
                outcome: outcome_from(&row.get::<_, String>(7)?),
                elapsed: row.get(8)?,
                environment: row.get(9)?,
                truncated: row.get(10)?,
            })
        },
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn execution(db: &Connection, id: ExecutionId) -> Result<Option<ExecutionRecord>, StoreError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM executions WHERE id = ?1",
            [id.0],
            |row| row.get(0),
        )
        .optional()?;
    Ok(payload
        .map(|text| serde_json::from_str(&text))
        .transpose()?)
}

pub fn count(db: &Connection) -> Result<u64, StoreError> {
    let count: i64 = db.query_row("SELECT count(*) FROM executions", [], |row| row.get(0))?;
    Ok(u64::try_from(count).unwrap_or(0))
}

pub fn clear(db: &Connection) -> Result<(), StoreError> {
    db.execute("DELETE FROM executions", [])?;
    Ok(())
}
