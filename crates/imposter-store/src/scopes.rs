//! One variable scope (the globals, an environment or a collection): its variables and the
//! values saved to swap them with.
//!
//! A collection's and an environment's values live in their files (the shared values) and,
//! for what stays on this computer, in `local_scopes`. The inspector and unshared scopes
//! can supply ordinary local values. An active capture overrides the ordinary value,
//! and its slot stays local when ordinary edits are saved.

use imposter_core::model::{SavedValues, Variables};
use imposter_core::variables::Scope;
use indexmap::IndexMap;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::files::workspace::{Kind, State};
use crate::{Conflict, StoreError, Tx};

/// The `local_scopes` key of a folder's globals, which have no file id: the globals belong
/// to the folder, so this computer's values for them do too.
fn globals_key(state: &State) -> String {
    format!(
        "{}:{}",
        crate::files::format::GLOBALS_ID,
        state.root.display()
    )
}
/// The most recent value written by a capture or script for one variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSlot {
    pub value: String,
    pub active: bool,
}

pub type CaptureSlots = IndexMap<String, CaptureSlot>;

/// Ordinary values and the device-local capture slots from the same scope snapshot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EditableScope {
    pub variables: Variables,
    pub saved: SavedValues,
    pub captures: CaptureSlots,
}

fn read_local(db: &Connection, uid: &str) -> Result<EditableScope, StoreError> {
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT variables, saved_values, latest_captures FROM local_scopes WHERE scope = ?1",
            [uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match row {
        Some((variables, saved, captures)) => Ok(EditableScope {
            variables: serde_json::from_str(&variables)?,
            saved: serde_json::from_str(&saved)?,
            captures: serde_json::from_str(&captures)?,
        }),
        None => Ok(EditableScope::default()),
    }
}

fn apply_captures(variables: &mut Variables, captures: CaptureSlots) {
    for (name, slot) in captures {
        if slot.active {
            variables.insert(name, slot.value);
        }
    }
}

pub(crate) fn local(db: &Connection, uid: &str) -> Result<Variables, StoreError> {
    let EditableScope {
        mut variables,
        captures,
        ..
    } = read_local(db, uid)?;
    apply_captures(&mut variables, captures);
    Ok(variables)
}

fn set_local(db: &Connection, uid: &str, local: &EditableScope) -> Result<(), StoreError> {
    if local.variables.is_empty() && local.saved.is_empty() && local.captures.is_empty() {
        return forget_local(db, uid);
    }
    db.execute(
        "INSERT INTO local_scopes (scope, variables, saved_values, latest_captures)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (scope) DO UPDATE
         SET variables = ?2, saved_values = ?3, latest_captures = ?4",
        params![
            uid,
            serde_json::to_string(&local.variables)?,
            serde_json::to_string(&local.saved)?,
            serde_json::to_string(&local.captures)?,
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

/// Ordinary values and the latest slots, without applying active captures.
pub fn editable(db: &Tx<'_>, scope: Scope) -> Result<EditableScope, StoreError> {
    let filed = db.read(|state| filed(state, scope))?;
    let local = read_local(db, &filed.uid)?;
    let mut variables = filed.variables;
    for (name, value) in local.variables {
        variables.insert(name, value);
    }
    let mut saved = if filed.shares {
        filed.saved
    } else {
        local.saved
    };
    saved.retain_defined(&variables);
    Ok(EditableScope {
        variables,
        saved,
        captures: local.captures,
    })
}

/// A scope's effective values and saved alternatives.
pub fn read(db: &Tx<'_>, scope: Scope) -> Result<(Variables, SavedValues), StoreError> {
    let EditableScope {
        mut variables,
        saved,
        captures,
    } = editable(db, scope)?;
    apply_captures(&mut variables, captures);
    Ok((variables, saved))
}

pub fn saved_values(db: &Tx<'_>, scope: Scope) -> Result<SavedValues, StoreError> {
    Ok(read(db, scope)?.1)
}

/// Replace ordinary values. A caller carrying forward an active capture in its effective
/// values keeps the slot local, even when switching the scope to shared.
pub fn write(
    db: &Tx<'_>,
    scope: Scope,
    variables: &Variables,
    saved: &SavedValues,
) -> Result<(), StoreError> {
    let current = editable(db, scope)?;
    let mut edit = EditableScope {
        variables: variables.clone(),
        saved: saved.clone(),
        captures: current.captures,
    };
    edit.captures.retain(|name, slot| {
        let Some(value) = edit.variables.get(name) else {
            return false;
        };
        slot.active &= *value == slot.value;
        if slot.active {
            if let Some(ordinary) = current.variables.get(name) {
                edit.variables.insert(name.clone(), ordinary.clone());
            } else {
                edit.variables.shift_remove(name);
            }
        }
        true
    });
    write_scope(db, scope, edit)
}

/// Save the editor's ordinary values and explicit slot changes as one transaction.
pub fn write_editor(
    db: &Tx<'_>,
    scope: Scope,
    edit: EditableScope,
    baseline: &CaptureSlots,
) -> Result<(), StoreError> {
    let uid = db.read(|state| filed(state, scope))?.uid;
    if read_local(db, &uid)?.captures != *baseline {
        return Err(Conflict::CapturesChanged.into());
    }
    write_scope(db, scope, edit)
}

fn write_scope(db: &Tx<'_>, scope: Scope, mut edit: EditableScope) -> Result<(), StoreError> {
    edit.saved.retain_defined(&edit.variables);
    let (uid, shares) = db.write(|state, _| {
        let filed = filed(state, scope)?;
        let (file_variables, file_saved) = if filed.shares {
            (edit.variables.clone(), edit.saved.clone())
        } else {
            (
                edit.variables
                    .keys()
                    .map(|name| (name.clone(), String::new()))
                    .collect(),
                SavedValues::default(),
            )
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
        edit.variables.clear();
        edit.saved = SavedValues::default();
    }
    set_local(db, &uid, &edit)
}

/// Replace and activate the latest values from captures or scripts on this computer.
pub fn set_current(
    db: &Tx<'_>,
    scope: Scope,
    values: &[(String, String)],
) -> Result<(), StoreError> {
    let uid = db.read(|state| filed(state, scope))?.uid;
    let mut local = read_local(db, &uid)?;
    for (name, value) in values {
        local.captures.insert(
            name.clone(),
            CaptureSlot {
                value: value.clone(),
                active: true,
            },
        );
    }
    set_local(db, &uid, &local)
}

/// Make `value` the value of `name` on this computer, which the scope must still define.
pub fn choose_value(db: &Tx<'_>, scope: Scope, name: &str, value: &str) -> Result<(), StoreError> {
    let (variables, _) = read(db, scope)?;
    if !variables.contains_key(name) {
        return Err(StoreError::NotFound("variable"));
    }
    let uid = db.read(|state| filed(state, scope))?.uid;
    let mut local = read_local(db, &uid)?;
    if let Some(slot) = local.captures.get_mut(name) {
        slot.active = false;
    }
    local.variables.insert(name.to_owned(), value.to_owned());
    set_local(db, &uid, &local)
}
