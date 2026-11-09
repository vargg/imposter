//! Collections and environments as files in one folder: the format, the model the store
//! works on, and the folder as the database thread holds it.

pub mod format;
pub mod model;
pub mod workspace;

use std::path::Path;

use imposter_core::ids::{CollectionId, EnvironmentId, RequestId};
use imposter_core::model::{EnvColor, RequestTemplate, SavedValues, Scripts, Variables};
use imposter_core::variables::Scope;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::StoreError;
use model::{CollectionRow, EnvironmentRow, FolderRow, Model, RequestRow};
use workspace::{Kind, State};

/// Use `dir` as the collections folder from now on, remembered in the profile: its files
/// are read, nothing is written to it by the switch.
pub fn switch(db: &crate::Tx<'_>, dir: &Path) -> Result<(), StoreError> {
    crate::kv::set(
        db,
        crate::kv::Table::Settings,
        workspace::FOLDER_KEY,
        &dir.display().to_string(),
    )?;
    let state = State::load(db, dir)?;
    db.replace(state);
    Ok(())
}

/// What a reload found changed on disk, by the ids the app knows them by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reloaded {
    /// Requests whose file now says something else.
    pub requests_changed: Vec<RequestId>,
    /// Requests whose file is gone.
    pub requests_removed: Vec<RequestId>,
    /// Anything the collections list, the tree or the palette shows changed.
    pub collections_changed: bool,
    /// Environments were added, removed, renamed or recolored.
    pub environments_changed: bool,
    /// Variable scopes whose shared values or saved values changed.
    pub scopes_changed: Vec<Scope>,
    /// Why the folder does not read now; the last good one stays, and nothing is written.
    pub error: Option<String>,
}

impl Reloaded {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Read the folder again and adopt what changed on disk since it was read or written:
/// another editor, a `git pull`, an agent. The store's own writes match what it remembers,
/// so they are no change. A folder that does not read keeps the last good model, refusing
/// writes until a later reload reads it.
pub fn reload(db: &crate::Tx<'_>) -> Result<Reloaded, StoreError> {
    let root = db.folder();
    let bytes = format::read_dir(&root).map_err(|e| StoreError::Files(e.to_string()))?;
    let (same, had_error) = db.read(|state| (state.files == bytes, state.error.is_some()));
    if same && !had_error {
        return Ok(Reloaded::default());
    }
    if let Err(error) = format::read_owned(&bytes) {
        let message = error.to_string();
        let mut kept = db.read(Clone::clone);
        kept.error = Some(message.clone());
        db.replace(kept);
        return Ok(Reloaded {
            error: Some(message),
            ..Reloaded::default()
        });
    }
    let fresh = State::load(db, &root)?;
    let changes = db.read(|old| difference(old, &fresh));
    db.replace(fresh);
    Ok(changes)
}

/// What differs between two readings of the folder, by id.
fn difference(old: &State, new: &State) -> Reloaded {
    use std::collections::HashMap;
    let id = |state: &State, uid: &str| state.ids.id(uid).unwrap_or(0);
    let mut changes = Reloaded::default();
    let new_requests: HashMap<&str, &RequestRow> = new
        .model
        .requests
        .iter()
        .map(|r| (r.uid.as_str(), r))
        .collect();
    for request in &old.model.requests {
        match new_requests.get(request.uid.as_str()) {
            None => changes
                .requests_removed
                .push(RequestId(id(old, &request.uid))),
            Some(now) if *now != request => changes
                .requests_changed
                .push(RequestId(id(new, &request.uid))),
            Some(_) => {}
        }
    }
    let sorted = |model: &Model| {
        let mut collections = model.collections.clone();
        collections.sort_by(|a, b| a.uid.cmp(&b.uid));
        let mut folders = model.folders.clone();
        folders.sort_by(|a, b| a.uid.cmp(&b.uid));
        let mut requests: Vec<String> = model.requests.iter().map(|r| r.uid.clone()).collect();
        requests.sort();
        (collections, folders, requests)
    };
    changes.collections_changed = sorted(&old.model) != sorted(&new.model)
        || !changes.requests_changed.is_empty()
        || !changes.requests_removed.is_empty();
    for collection in &new.model.collections {
        let before = old
            .model
            .collections
            .iter()
            .find(|c| c.uid == collection.uid);
        if before.is_none_or(|b| {
            b.variables != collection.variables || b.saved_values != collection.saved_values
        }) {
            changes
                .scopes_changed
                .push(Scope::Collection(CollectionId(id(new, &collection.uid))));
        }
    }
    let mut old_environments = old.model.environments.clone();
    let mut new_environments = new.model.environments.clone();
    old_environments.sort_by(|a, b| a.uid.cmp(&b.uid));
    new_environments.sort_by(|a, b| a.uid.cmp(&b.uid));
    changes.environments_changed = old_environments
        .iter()
        .map(|e| (&e.uid, &e.name, e.color, e.share_values))
        .ne(new_environments
            .iter()
            .map(|e| (&e.uid, &e.name, e.color, e.share_values)));
    for environment in &new_environments {
        let before = old_environments.iter().find(|e| e.uid == environment.uid);
        if before.is_none_or(|b| b != environment) {
            changes
                .scopes_changed
                .push(Scope::Environment(EnvironmentId(id(new, &environment.uid))));
        }
    }
    if old.model.globals != new.model.globals {
        changes.scopes_changed.push(Scope::Global);
    }
    changes
}

/// Set once the old `globals` row was moved to `globals.yaml`.
const GLOBALS_MIGRATED_KEY: &str = "globals_migrated";

/// Write the old `globals` row to `globals.yaml` once, values shared (user's choice). A
/// folder that already has the file keeps it; a folder that could not be read is left
/// alone and asked again next time.
fn move_globals(db: &Connection, state: &mut State) -> Result<(), StoreError> {
    let key = GLOBALS_MIGRATED_KEY;
    if crate::kv::get::<bool>(db, crate::kv::Table::Settings, key)?.unwrap_or(false)
        || state.error.is_some()
    {
        return Ok(());
    }
    if state.model.globals.is_none() {
        let (variables, saved): (String, String) = db.query_row(
            "SELECT variables, saved_values FROM globals WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let variables: Variables = serde_json::from_str(&variables)?;
        let saved: SavedValues = serde_json::from_str(&saved)?;
        if !variables.is_empty() {
            let before = state.clone();
            state.model.globals = Some(format::Globals {
                share_values: true,
                variables,
                saved_values: saved,
            });
            state.write_changes(&before)?;
        }
    }
    crate::kv::set(db, crate::kv::Table::Settings, key, &true)
}

/// Set once the old tables' collections and environments were moved to the folder.
const MIGRATED_KEY: &str = "files_migrated";

/// The folder for a database just opened: the old tables moved into it the first time,
/// then read. A database too old to have the `ids` table (a schema test) gets none.
pub fn open(connection: &mut Connection, database: &Path) -> Result<State, StoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let root = workspace::folder(&tx, database)?;
    if !has_table(&tx, "ids")? {
        return Ok(State::unavailable(
            &root,
            "the database predates collection folders",
        ));
    }
    move_tables(&tx, &root)?;
    let mut state = State::load(&tx, &root)?;
    move_globals(&tx, &mut state)?;
    tx.commit()?;
    Ok(state)
}

fn has_table(db: &Connection, name: &str) -> Result<bool, StoreError> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Write the old tables' collections and environments to `root` once, keeping their ids so
/// History and the active environment still point at them. A folder that already holds
/// collections is left alone: it is somebody's, and the old rows stay in the database.
fn move_tables(db: &Connection, root: &Path) -> Result<(), StoreError> {
    if crate::kv::get::<bool>(db, crate::kv::Table::Settings, MIGRATED_KEY)?.unwrap_or(false) {
        return Ok(());
    }
    let existing = format::read_dir(root).map_err(|e| StoreError::Files(e.to_string()))?;
    let taken = existing
        .keys()
        .any(|p| p.ends_with(format::COLLECTION_FILE) || p.starts_with(format::ENVIRONMENTS_DIR));
    if taken {
        tracing::warn!(
            "the collections folder already holds collections; the old ones stay in the database"
        );
    } else {
        let (model, ids) = old_rows(db)?;
        if !model.collections.is_empty() || !model.environments.is_empty() {
            let mut state = State::unavailable(root, "");
            state.error = None;
            let empty = state.clone();
            state.model = model;
            for (uid, kind, id) in &ids {
                state.ids.record(db, *kind, uid, *id)?;
            }
            state.write_changes(&empty)?;
        }
    }
    crate::kv::set(db, crate::kv::Table::Settings, MIGRATED_KEY, &true)
}

type OldIds = Vec<(String, Kind, i64)>;

/// The collections and environments of the tables the folder replaces, with new file ids
/// and the row ids they keep.
fn old_rows(db: &Connection) -> Result<(Model, OldIds), StoreError> {
    let mut model = Model::default();
    let mut ids: OldIds = Vec::new();
    let mut uid_of = std::collections::HashMap::new();
    let mut give = |kind: Kind, id: i64, ids: &mut OldIds| -> String {
        let uid = workspace::new_uid();
        ids.push((uid.clone(), kind, id));
        uid_of.insert((kind, id), uid.clone());
        uid
    };
    let json = |text: String| serde_json::from_str::<serde_json::Value>(&text);
    let mut statement = db.prepare(
        "SELECT id, name, description, variables, saved_values, scripts FROM collections ORDER BY id",
    )?;
    let collections = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, name, description, variables, saved, scripts) in collections {
        let uid = give(Kind::Collection, id, &mut ids);
        model.collections.push(CollectionRow {
            uid,
            name,
            description,
            variables: serde_json::from_str::<Variables>(&variables)?,
            saved_values: serde_json::from_str::<SavedValues>(&saved)?,
            scripts: serde_json::from_str::<Scripts>(&scripts)?,
        });
    }
    let _ = json;
    let mut statement = db.prepare(
        "SELECT id, collection_id, parent_folder_id, name, description, position, scripts FROM folders ORDER BY id",
    )?;
    let folders = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, ..) in &folders {
        give(Kind::Folder, *id, &mut ids);
    }
    let lookup = |kind: Kind, id: i64| -> Option<String> {
        ids.iter()
            .find(|(_, k, i)| *k == kind && *i == id)
            .map(|(u, _, _)| u.clone())
    };
    for (id, collection, parent, name, description, position, scripts) in folders {
        let (Some(uid), Some(collection)) = (
            lookup(Kind::Folder, id),
            lookup(Kind::Collection, collection),
        ) else {
            continue;
        };
        model.folders.push(FolderRow {
            uid,
            collection,
            parent: parent.and_then(|p| lookup(Kind::Folder, p)),
            name,
            description,
            position,
            scripts: serde_json::from_str::<Scripts>(&scripts)?,
        });
    }
    let mut statement =
        db.prepare("SELECT id, collection_id, folder_id, template FROM requests ORDER BY id")?;
    let requests = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut request_ids = Vec::new();
    for (id, collection, folder, template) in requests {
        let Some(collection) = lookup(Kind::Collection, collection) else {
            continue;
        };
        let uid = workspace::new_uid();
        request_ids.push((uid.clone(), Kind::Request, id));
        model.requests.push(RequestRow {
            uid,
            collection,
            folder: folder.and_then(|f| lookup(Kind::Folder, f)),
            template: serde_json::from_str::<RequestTemplate>(&template)?,
        });
    }
    ids.extend(request_ids);
    let mut statement = db
        .prepare("SELECT id, name, variables, saved_values, color FROM environments ORDER BY id")?;
    let environments = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, name, variables, saved, color) in environments {
        let uid = workspace::new_uid();
        ids.push((uid.clone(), Kind::Environment, id));
        model.environments.push(EnvironmentRow {
            uid,
            name,
            color: color.as_deref().and_then(EnvColor::from_name),
            share_values: true,
            variables: serde_json::from_str::<Variables>(&variables)?,
            saved_values: serde_json::from_str::<SavedValues>(&saved)?,
        });
    }
    Ok((model, ids))
}
