//! One variable scope (the globals, an environment or a collection): its variables and the
//! values saved to swap them with, read and written together.

use imposter_core::model::{SavedValues, Variables};
use imposter_core::variables::Scope;
use rusqlite::{Connection, OptionalExtension, params};

use crate::StoreError;

/// The table row that holds a scope, and what to call it when it is gone.
fn location(scope: Scope) -> (&'static str, i64, &'static str) {
    match scope {
        Scope::Global => ("globals", 1, "globals"),
        Scope::Environment(id) => ("environments", id.0, "environment"),
        Scope::Collection(id) => ("collections", id.0, "collection"),
    }
}

/// A scope's variables and saved values. Saved values of names the scope no longer defines
/// (removed by an import or an older writer) are left out.
pub fn read(db: &Connection, scope: Scope) -> Result<(Variables, SavedValues), StoreError> {
    let (table, id, what) = location(scope);
    let (variables, saved): (String, String) = db
        .query_row(
            &format!("SELECT variables, saved_values FROM {table} WHERE id = ?1"),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(StoreError::NotFound(what))?;
    let variables: Variables = serde_json::from_str(&variables)?;
    let mut saved: SavedValues = serde_json::from_str(&saved)?;
    saved.retain_defined(&variables);
    Ok((variables, saved))
}

pub fn saved_values(db: &Connection, scope: Scope) -> Result<SavedValues, StoreError> {
    Ok(read(db, scope)?.1)
}

/// Replace a scope's variables and saved values, as the Variables screen saves them.
pub fn write(
    db: &Connection,
    scope: Scope,
    variables: &Variables,
    saved: &SavedValues,
) -> Result<(), StoreError> {
    let (table, id, what) = location(scope);
    let mut saved = saved.clone();
    saved.retain_defined(variables);
    let rows = db.execute(
        &format!("UPDATE {table} SET variables = ?1, saved_values = ?2 WHERE id = ?3"),
        params![
            serde_json::to_string(variables)?,
            serde_json::to_string(&saved)?,
            id
        ],
    )?;
    if rows == 0 {
        return Err(StoreError::NotFound(what));
    }
    Ok(())
}

/// Make `value` the current value of `name`, which the scope must still define. Only that
/// variable changes; order and the other values stay.
pub fn choose_value(
    db: &Connection,
    scope: Scope,
    name: &str,
    value: &str,
) -> Result<(), StoreError> {
    let (mut variables, saved) = read(db, scope)?;
    let current = variables
        .get_mut(name)
        .ok_or(StoreError::NotFound("variable"))?;
    value.clone_into(current);
    write(db, scope, &variables, &saved)
}
