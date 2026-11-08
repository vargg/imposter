//! Secrets: the global ones and each environment's, by name. Kept in their own table so
//! exports, which read variables, never see them.

use imposter_core::model::Variables;
use imposter_core::variables::SecretScope;
use rusqlite::{Connection, params};

use crate::StoreError;

fn environment_id(scope: SecretScope) -> Option<i64> {
    match scope {
        SecretScope::Global => None,
        SecretScope::Environment(id) => Some(id.0),
    }
}

/// A scope's secrets, in the order they were written.
pub fn read(db: &Connection, scope: SecretScope) -> Result<Variables, StoreError> {
    let mut statement = db
        .prepare_cached("SELECT name, value FROM secrets WHERE environment_id IS ?1 ORDER BY id")?;
    let rows = statement.query_map([environment_id(scope)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<Variables, _>>()?)
}

/// Every scope's secret names, values left out: what may be listed. Global first, then
/// by environment id.
pub fn names(db: &Connection) -> Result<Vec<(SecretScope, String)>, StoreError> {
    let mut statement = db.prepare_cached(
        "SELECT environment_id, name FROM secrets ORDER BY coalesce(environment_id, 0), id",
    )?;
    let rows = statement.query_map([], |row| {
        let scope = match row.get::<_, Option<i64>>(0)? {
            None => SecretScope::Global,
            Some(id) => SecretScope::Environment(imposter_core::ids::EnvironmentId(id)),
        };
        Ok((scope, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Replace a scope's secrets, as the Variables screen saves them.
pub fn write(db: &Connection, scope: SecretScope, secrets: &Variables) -> Result<(), StoreError> {
    let id = environment_id(scope);
    db.execute("DELETE FROM secrets WHERE environment_id IS ?1", [id])?;
    let mut insert =
        db.prepare_cached("INSERT INTO secrets (environment_id, name, value) VALUES (?1, ?2, ?3)")?;
    for (name, value) in secrets {
        insert.execute(params![id, name, value])?;
    }
    Ok(())
}

/// Set `values` in `scope`: existing names keep their place, new ones go last. Captures
/// and scripts store secrets this way.
pub fn set(
    db: &Connection,
    scope: SecretScope,
    values: &[(String, String)],
) -> Result<(), StoreError> {
    let id = environment_id(scope);
    for (name, value) in values {
        let updated = db.execute(
            "UPDATE secrets SET value = ?3 WHERE environment_id IS ?1 AND name = ?2",
            params![id, name, value],
        )?;
        if updated == 0 {
            db.execute(
                "INSERT INTO secrets (environment_id, name, value) VALUES (?1, ?2, ?3)",
                params![id, name, value],
            )?;
        }
    }
    Ok(())
}
