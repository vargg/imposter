//! Collections and environments as files in one folder: the format, the model the store
//! works on, and the folder as the database thread holds it.

pub mod format;
pub mod model;
pub mod workspace;

use std::path::Path;

use imposter_core::model::{EnvColor, RequestTemplate, SavedValues, Scripts, Variables};
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
    let state = State::load(&tx, &root)?;
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
