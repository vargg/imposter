//! Environments, global variables and the active environment. Environments are files in
//! the collections folder (`environments/*.yaml`); globals and which environment is active
//! stay in the profile.

use imposter_core::ids::EnvironmentId;
use imposter_core::model::{EnvColor, SavedValues, Variables};
use imposter_core::variables::{Scope, SecretScope, VariableContext};
use rusqlite::Connection;

use crate::files::model::EnvironmentRow;
use crate::files::workspace::{Kind, State, new_uid};
use crate::kv::{self, Table};
use crate::scopes;
use crate::{Conflict, StoreError, Tx};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: String,
    /// The values in effect: the file's, with this computer's local values over them.
    pub variables: Variables,
    pub color: Option<EnvColor>,
    /// Off: the file lists the names only, and the values stay on this computer.
    pub share_values: bool,
}

const ACTIVE_KEY: &str = "active_environment";

fn environment_uid(state: &State, id: EnvironmentId) -> Option<String> {
    let uid = state.ids.uid(Kind::Environment, id.0)?;
    state
        .model
        .environments
        .iter()
        .any(|e| e.uid == uid)
        .then(|| uid.to_owned())
}

fn to_environment(
    db: &Connection,
    state: &State,
    row: &EnvironmentRow,
) -> Result<Environment, StoreError> {
    let (local, _) = scopes::local(db, &row.uid)?;
    let mut variables = row.variables.clone();
    for (name, value) in local {
        variables.insert(name, value);
    }
    Ok(Environment {
        id: EnvironmentId(state.ids.id(&row.uid).unwrap_or(0)),
        name: row.name.clone(),
        variables,
        color: row.color,
        share_values: row.share_values,
    })
}

/// "Global" names the scope of global variables, so no environment may take it, in any
/// case: a "global" environment beside the Global scope would read as the same thing.
fn check_name(state: &State, name: &str, except: Option<&str>) -> Result<(), StoreError> {
    let taken = state
        .model
        .environments
        .iter()
        .any(|e| e.name == name && Some(e.uid.as_str()) != except);
    let global = name
        .trim()
        .eq_ignore_ascii_case(VariableContext::GLOBAL_NAME);
    if name.trim().is_empty() || global || taken {
        return Err(Conflict::EnvironmentName.into());
    }
    Ok(())
}

/// Every environment, by name then id.
pub fn environments(db: &Tx<'_>) -> Result<Vec<Environment>, StoreError> {
    let mut out = db.read(|state| {
        state
            .model
            .environments
            .iter()
            .map(|row| to_environment(db, state, row))
            .collect::<Result<Vec<_>, _>>()
    })?;
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}

pub fn environment(db: &Tx<'_>, id: EnvironmentId) -> Result<Option<Environment>, StoreError> {
    db.read(|state| {
        let Some(uid) = environment_uid(state, id) else {
            return Ok(None);
        };
        let row = state.model.environments.iter().find(|e| e.uid == uid);
        row.map(|row| to_environment(db, state, row)).transpose()
    })
}

pub fn environment_names(db: &Tx<'_>) -> Result<Vec<String>, StoreError> {
    Ok(db.read(|state| {
        state
            .model
            .environments
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }))
}

pub fn create_environment(
    db: &Tx<'_>,
    name: &str,
    variables: &Variables,
) -> Result<Environment, StoreError> {
    db.write(|state, sql| {
        check_name(state, name, None)?;
        let uid = new_uid();
        let id = state.ids.assign(sql, Kind::Environment, &uid)?;
        state.model.environments.push(EnvironmentRow {
            uid,
            name: name.to_owned(),
            color: None,
            share_values: true,
            variables: variables.clone(),
            saved_values: SavedValues::default(),
        });
        Ok(Environment {
            id: EnvironmentId(id),
            name: name.to_owned(),
            variables: variables.clone(),
            color: None,
            share_values: true,
        })
    })
}

pub fn rename_environment(db: &Tx<'_>, id: EnvironmentId, name: &str) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = environment_uid(state, id).ok_or(StoreError::NotFound("environment"))?;
        check_name(state, name, Some(&uid))?;
        if let Some(row) = state.model.environments.iter_mut().find(|e| e.uid == uid) {
            name.clone_into(&mut row.name);
        }
        Ok(())
    })
}

pub fn set_environment_color(
    db: &Tx<'_>,
    id: EnvironmentId,
    color: Option<EnvColor>,
) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = environment_uid(state, id).ok_or(StoreError::NotFound("environment"))?;
        if let Some(row) = state.model.environments.iter_mut().find(|e| e.uid == uid) {
            row.color = color;
        }
        Ok(())
    })
}

/// Share the environment's values in its file, or keep them on this computer only. The
/// values in effect do not change.
pub fn set_share_values(db: &Tx<'_>, id: EnvironmentId, share: bool) -> Result<(), StoreError> {
    let scope = Scope::Environment(id);
    let (variables, saved) = scopes::read(db, scope)?;
    db.write(|state, _| {
        let uid = environment_uid(state, id).ok_or(StoreError::NotFound("environment"))?;
        if let Some(row) = state.model.environments.iter_mut().find(|e| e.uid == uid) {
            row.share_values = share;
        }
        Ok(())
    })?;
    scopes::write(db, scope, &variables, &saved)
}

/// Replace the environment's shared values, as an explicit edit does.
pub fn set_environment_variables(
    db: &Tx<'_>,
    id: EnvironmentId,
    variables: &Variables,
) -> Result<(), StoreError> {
    let saved = scopes::saved_values(db, Scope::Environment(id))?;
    scopes::write(db, Scope::Environment(id), variables, &saved)
}

/// Deleting the active environment makes globals-only the active scope. Its secrets and
/// local values go with it.
pub fn delete_environment(db: &Tx<'_>, id: EnvironmentId) -> Result<(), StoreError> {
    let uid = db.write(|state, _| {
        let uid = environment_uid(state, id).ok_or(StoreError::NotFound("environment"))?;
        state.model.environments.retain(|e| e.uid != uid);
        Ok(uid)
    })?;
    scopes::forget_local(db, &uid)?;
    crate::secrets::write(db, SecretScope::Environment(id), &Variables::new())?;
    if active_environment(db)? == Some(id) {
        set_active_environment(db, None)?;
    }
    Ok(())
}

/// The global variables in effect: `globals.yaml`'s, with this computer's over them.
pub fn globals(db: &Tx<'_>) -> Result<Variables, StoreError> {
    Ok(scopes::read(db, Scope::Global)?.0)
}

/// Replace the global variables, as an explicit edit does.
pub fn set_globals(db: &Tx<'_>, variables: &Variables) -> Result<(), StoreError> {
    let saved = scopes::saved_values(db, Scope::Global)?;
    scopes::write(db, Scope::Global, variables, &saved)
}

/// Whether the globals' values are written to `globals.yaml` (they are until turned off).
pub fn globals_shared(db: &Tx<'_>) -> bool {
    db.read(|state| state.model.globals.as_ref().is_none_or(|g| g.share_values))
}

/// Share the globals' values in `globals.yaml`, or keep them on this computer only. The
/// values in effect do not change.
pub fn set_globals_shared(db: &Tx<'_>, share: bool) -> Result<(), StoreError> {
    let (variables, saved) = scopes::read(db, Scope::Global)?;
    db.write(|state, _| {
        let globals = state
            .model
            .globals
            .get_or_insert_with(|| crate::files::format::Globals {
                share_values: true,
                variables: Variables::new(),
                saved_values: SavedValues::default(),
            });
        globals.share_values = share;
        Ok(())
    })?;
    scopes::write(db, Scope::Global, &variables, &saved)
}

/// The active environment, or `None` for globals only. A stale id (deleted elsewhere) reads
/// as `None`.
pub fn active_environment(db: &Tx<'_>) -> Result<Option<EnvironmentId>, StoreError> {
    let Some(id) = kv::get::<Option<EnvironmentId>>(db, Table::Settings, ACTIVE_KEY)?.flatten()
    else {
        return Ok(None);
    };
    Ok(environment(db, id)?.map(|env| env.id))
}

pub fn set_active_environment(db: &Tx<'_>, id: Option<EnvironmentId>) -> Result<(), StoreError> {
    if let Some(id) = id
        && environment(db, id)?.is_none()
    {
        return Err(StoreError::NotFound("environment"));
    }
    kv::set(db, Table::Settings, ACTIVE_KEY, &id)
}

/// Globals and the active environment, read together so execution sees one consistent state.
pub fn variable_context(db: &Tx<'_>) -> Result<VariableContext, StoreError> {
    variable_context_for(db, active_environment(db)?)
}

/// Globals and the environment `chosen` (none: globals only), whichever is active.
pub fn variable_context_for(
    db: &Tx<'_>,
    chosen: Option<EnvironmentId>,
) -> Result<VariableContext, StoreError> {
    let environment = match chosen {
        Some(id) => match environment(db, id)? {
            Some(env) => Some(imposter_core::variables::ActiveEnvironment {
                saved: scopes::saved_values(db, Scope::Environment(env.id))?,
                id: Some(env.id),
                name: env.name,
                values: env.variables,
                secrets: crate::secrets::read(db, SecretScope::Environment(env.id))?,
            }),
            None => None,
        },
        None => None,
    };
    Ok(VariableContext {
        globals: globals(db)?,
        global_saved: scopes::saved_values(db, Scope::Global)?,
        environment,
        global_secrets: crate::secrets::read(db, SecretScope::Global)?,
    })
}
