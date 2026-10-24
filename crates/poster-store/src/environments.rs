//! Environments, global variables and the active environment.

use poster_core::ids::EnvironmentId;
use poster_core::model::Variables;
use poster_core::variables::VariableContext;
use rusqlite::{Connection, OptionalExtension, params};

use crate::kv::{self, Table};
use crate::{Conflict, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: String,
    pub variables: Variables,
}

const ACTIVE_KEY: &str = "active_environment";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn to_environment((id, name, variables): (i64, String, String)) -> Result<Environment, StoreError> {
    Ok(Environment {
        id: EnvironmentId(id),
        name,
        variables: serde_json::from_str(&variables)?,
    })
}

/// "Global" names the scope of global variables, so no environment may take it.
fn check_name(
    db: &Connection,
    name: &str,
    except: Option<EnvironmentId>,
) -> Result<(), StoreError> {
    let taken: bool = db.query_row(
        "SELECT exists(SELECT 1 FROM environments WHERE name = ?1 AND id IS NOT ?2)",
        params![name, except.map(|id| id.0)],
        |row| row.get(0),
    )?;
    if name.trim().is_empty() || name == VariableContext::GLOBAL_NAME || taken {
        return Err(Conflict::EnvironmentName.into());
    }
    Ok(())
}

pub fn environments(db: &Connection) -> Result<Vec<Environment>, StoreError> {
    let mut statement =
        db.prepare("SELECT id, name, variables FROM environments ORDER BY name, id")?;
    let rows = statement
        .query_map([], row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(to_environment).collect()
}

pub fn environment(db: &Connection, id: EnvironmentId) -> Result<Option<Environment>, StoreError> {
    db.query_row(
        "SELECT id, name, variables FROM environments WHERE id = ?1",
        [id.0],
        row,
    )
    .optional()?
    .map(to_environment)
    .transpose()
}

pub fn environment_names(db: &Connection) -> Result<Vec<String>, StoreError> {
    let mut statement = db.prepare("SELECT name FROM environments")?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

pub fn create_environment(
    db: &Connection,
    name: &str,
    variables: &Variables,
) -> Result<Environment, StoreError> {
    check_name(db, name, None)?;
    db.execute(
        "INSERT INTO environments (name, variables) VALUES (?1, ?2)",
        params![name, serde_json::to_string(variables)?],
    )?;
    Ok(Environment {
        id: EnvironmentId(db.last_insert_rowid()),
        name: name.to_owned(),
        variables: variables.clone(),
    })
}

pub fn rename_environment(
    db: &Connection,
    id: EnvironmentId,
    name: &str,
) -> Result<(), StoreError> {
    check_name(db, name, Some(id))?;
    let rows = db.execute(
        "UPDATE environments SET name = ?1 WHERE id = ?2",
        params![name, id.0],
    )?;
    if rows == 0 {
        Err(StoreError::NotFound("environment"))
    } else {
        Ok(())
    }
}

pub fn set_environment_variables(
    db: &Connection,
    id: EnvironmentId,
    variables: &Variables,
) -> Result<(), StoreError> {
    let rows = db.execute(
        "UPDATE environments SET variables = ?1 WHERE id = ?2",
        params![serde_json::to_string(variables)?, id.0],
    )?;
    if rows == 0 {
        Err(StoreError::NotFound("environment"))
    } else {
        Ok(())
    }
}

/// Deleting the active environment makes globals-only the active scope.
pub fn delete_environment(db: &Connection, id: EnvironmentId) -> Result<(), StoreError> {
    let rows = db.execute("DELETE FROM environments WHERE id = ?1", [id.0])?;
    if rows == 0 {
        return Err(StoreError::NotFound("environment"));
    }
    if active_environment(db)? == Some(id) {
        set_active_environment(db, None)?;
    }
    Ok(())
}

pub fn globals(db: &Connection) -> Result<Variables, StoreError> {
    let text: String = db.query_row("SELECT variables FROM globals WHERE id = 1", [], |row| {
        row.get(0)
    })?;
    Ok(serde_json::from_str(&text)?)
}

pub fn set_globals(db: &Connection, variables: &Variables) -> Result<(), StoreError> {
    db.execute(
        "UPDATE globals SET variables = ?1 WHERE id = 1",
        [serde_json::to_string(variables)?],
    )?;
    Ok(())
}

/// The active environment, or `None` for globals only. A stale id (deleted elsewhere) reads
/// as `None`.
pub fn active_environment(db: &Connection) -> Result<Option<EnvironmentId>, StoreError> {
    let Some(id) = kv::get::<Option<EnvironmentId>>(db, Table::Settings, ACTIVE_KEY)?.flatten()
    else {
        return Ok(None);
    };
    Ok(environment(db, id)?.map(|env| env.id))
}

pub fn set_active_environment(
    db: &Connection,
    id: Option<EnvironmentId>,
) -> Result<(), StoreError> {
    if let Some(id) = id
        && environment(db, id)?.is_none()
    {
        return Err(StoreError::NotFound("environment"));
    }
    kv::set(db, Table::Settings, ACTIVE_KEY, &id)
}

/// Globals and the active environment, read together so execution sees one consistent state.
pub fn variable_context(db: &Connection) -> Result<VariableContext, StoreError> {
    let environment = match active_environment(db)? {
        Some(id) => environment(db, id)?.map(|env| poster_core::variables::ActiveEnvironment {
            id: env.id,
            name: env.name,
            values: env.variables,
        }),
        None => None,
    };
    Ok(VariableContext {
        globals: globals(db)?,
        environment,
    })
}
