//! One variable scope (the globals, an environment or a collection): its variables and the
//! values saved to swap them with.
//!
//! A collection's and an environment's values live in their files (the shared values) and,
//! for what stays on this computer, in `local_scopes`: what captures and the inspector's
//! value swap set, and all of an environment's values when it does not share them. The
//! values in effect are the local ones over the shared ones. Saving on the Variables screen
//! ([`write`]) makes what it shows the shared values and clears the local ones. Globals
//! work the same way, in `globals.yaml`.

use imposter_core::model::{SavedValues, Variables};
use imposter_core::variables::Scope;
use rusqlite::{Connection, OptionalExtension, params};

use crate::files::workspace::{Kind, State};

/// The `local_scopes` key of a folder's globals, which have no file id: the globals belong
/// to the folder, so this computer's values for them do too.
fn globals_key(state: &State) -> String {
    format!(
        "{}:{}",
        crate::files::format::GLOBALS_ID,
        state.root.display()
    )
}
use crate::{StoreError, Tx};

/// This computer's values for the scope whose file id is `uid`, and its saved values when
/// they are kept here.
pub(crate) fn local(db: &Connection, uid: &str) -> Result<(Variables, SavedValues), StoreError> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT variables, saved_values FROM local_scopes WHERE scope = ?1",
            [uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((variables, saved)) => Ok((
            serde_json::from_str(&variables)?,
            serde_json::from_str(&saved)?,
        )),
        None => Ok((Variables::new(), SavedValues::default())),
    }
}

fn set_local(
    db: &Connection,
    uid: &str,
    variables: &Variables,
    saved: &SavedValues,
) -> Result<(), StoreError> {
    if variables.is_empty() && saved.is_empty() {
        return forget_local(db, uid);
    }
    db.execute(
        "INSERT INTO local_scopes (scope, variables, saved_values) VALUES (?1, ?2, ?3)
         ON CONFLICT (scope) DO UPDATE SET variables = ?2, saved_values = ?3",
        params![
            uid,
            serde_json::to_string(variables)?,
            serde_json::to_string(saved)?
        ],
    )?;
    Ok(())
}

pub(crate) fn forget_local(db: &Connection, uid: &str) -> Result<(), StoreError> {
    db.execute("DELETE FROM local_scopes WHERE scope = ?1", [uid])?;
    Ok(())
}

/// Where a file-backed scope lives: its file id, its shared values and saved values, and
/// whether it shares its values (collections always do).
struct Filed {
    uid: String,
    variables: Variables,
    saved: SavedValues,
    shares: bool,
}

fn filed(state: &State, scope: Scope) -> Result<Filed, StoreError> {
    match scope {
        // No file yet: no globals, shared once there are some.
        Scope::Global => Ok(state.model.globals.as_ref().map_or_else(
            || Filed {
                uid: globals_key(state),
                variables: Variables::new(),
                saved: SavedValues::default(),
                shares: true,
            },
            |g| Filed {
                uid: globals_key(state),
                variables: g.variables.clone(),
                saved: g.saved_values.clone(),
                shares: g.share_values,
            },
        )),
        Scope::Collection(id) => {
            let uid = state.ids.uid(Kind::Collection, id.0);
            state
                .model
                .collections
                .iter()
                .find(|c| Some(c.uid.as_str()) == uid)
                .map(|c| Filed {
                    uid: c.uid.clone(),
                    variables: c.variables.clone(),
                    saved: c.saved_values.clone(),
                    shares: true,
                })
                .ok_or(StoreError::NotFound("collection"))
        }
        Scope::Environment(id) => {
            let uid = state.ids.uid(Kind::Environment, id.0);
            state
                .model
                .environments
                .iter()
                .find(|e| Some(e.uid.as_str()) == uid)
                .map(|e| Filed {
                    uid: e.uid.clone(),
                    variables: e.variables.clone(),
                    saved: e.saved_values.clone(),
                    shares: e.share_values,
                })
                .ok_or(StoreError::NotFound("environment"))
        }
    }
}

/// A scope's variables in effect and its saved values. Saved values of names the scope no
/// longer defines are left out.
pub fn read(db: &Tx<'_>, scope: Scope) -> Result<(Variables, SavedValues), StoreError> {
    let filed = db.read(|state| filed(state, scope))?;
    let (local, local_saved) = local(db, &filed.uid)?;
    let mut variables = filed.variables;
    for (name, value) in local {
        variables.insert(name, value);
    }
    let mut saved = if filed.shares {
        filed.saved
    } else {
        local_saved
    };
    saved.retain_defined(&variables);
    Ok((variables, saved))
}

pub fn saved_values(db: &Tx<'_>, scope: Scope) -> Result<SavedValues, StoreError> {
    Ok(read(db, scope)?.1)
}

/// Replace a scope's variables and saved values, as the Variables screen saves them: the
/// shared values in its file (names only for an environment that does not share, whose
/// values then stay here), and no local values left over.
pub fn write(
    db: &Tx<'_>,
    scope: Scope,
    variables: &Variables,
    saved: &SavedValues,
) -> Result<(), StoreError> {
    let mut saved = saved.clone();
    saved.retain_defined(variables);
    let (uid, shares) = db.write(|state, _| {
        let filed = filed(state, scope)?;
        let names_only: Variables = variables
            .keys()
            .map(|name| (name.clone(), String::new()))
            .collect();
        let (file_variables, file_saved) = if filed.shares {
            (variables.clone(), saved.clone())
        } else {
            (names_only, SavedValues::default())
        };
        match scope {
            Scope::Collection(_) => {
                if let Some(row) = state
                    .model
                    .collections
                    .iter_mut()
                    .find(|c| c.uid == filed.uid)
                {
                    row.variables = file_variables;
                    row.saved_values = file_saved;
                }
            }
            Scope::Environment(_) => {
                if let Some(row) = state
                    .model
                    .environments
                    .iter_mut()
                    .find(|e| e.uid == filed.uid)
                {
                    row.variables = file_variables;
                    row.saved_values = file_saved;
                }
            }
            Scope::Global => {
                // Empty shared globals need no file; a choice not to share is kept.
                state.model.globals = (!filed.shares
                    || !file_variables.is_empty()
                    || !file_saved.is_empty())
                .then(|| crate::files::format::Globals {
                    share_values: filed.shares,
                    variables: file_variables,
                    saved_values: file_saved,
                });
            }
        }
        Ok((filed.uid, filed.shares))
    })?;
    if shares {
        forget_local(db, &uid)
    } else {
        set_local(db, &uid, variables, &saved)
    }
}

/// Set this computer's values of `values`' names in `scope`: existing names keep their
/// place, new ones are added at the end. Captures and scripts write this way, so a send
/// never changes a file.
pub fn set_current(
    db: &Tx<'_>,
    scope: Scope,
    values: &[(String, String)],
) -> Result<(), StoreError> {
    let uid = db.read(|state| filed(state, scope))?.uid;
    let (mut local, saved) = local(db, &uid)?;
    for (name, value) in values {
        local.insert(name.clone(), value.clone());
    }
    set_local(db, &uid, &local, &saved)
}

/// Make `value` the value of `name` on this computer, which the scope must still define.
pub fn choose_value(db: &Tx<'_>, scope: Scope, name: &str, value: &str) -> Result<(), StoreError> {
    let (variables, _) = read(db, scope)?;
    if !variables.contains_key(name) {
        return Err(StoreError::NotFound("variable"));
    }
    set_current(db, scope, &[(name.to_owned(), value.to_owned())])
}
