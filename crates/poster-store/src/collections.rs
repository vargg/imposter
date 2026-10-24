//! Collections, folders and requests.

use poster_core::ids::{CollectionId, FolderId, RequestId};
use poster_core::model::{Request, RequestTemplate, Variables};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Conflict, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub description: String,
    pub variables: Variables,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub collection_id: CollectionId,
    pub parent_id: Option<FolderId>,
    pub name: String,
    pub description: String,
    pub position: i64,
}

/// What the tree shows for a request, read without decoding its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSummary {
    pub id: RequestId,
    pub folder_id: Option<FolderId>,
    pub name: String,
    pub method: String,
    pub position: i64,
}

/// A whole collection for the tree, loaded with three queries. Folders and requests are
/// ordered by position, then id; nesting is given by their parent ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub collection: Collection,
    pub folders: Vec<Folder>,
    pub requests: Vec<RequestSummary>,
}

fn non_empty(name: &str) -> Result<(), StoreError> {
    if name.trim().is_empty() {
        Err(Conflict::EmptyName.into())
    } else {
        Ok(())
    }
}

fn changed(rows: usize, entity: &'static str) -> Result<(), StoreError> {
    if rows == 0 {
        Err(StoreError::NotFound(entity))
    } else {
        Ok(())
    }
}

fn collection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn to_collection(
    (id, name, description, variables): (i64, String, String, String),
) -> Result<Collection, StoreError> {
    Ok(Collection {
        id: CollectionId(id),
        name,
        description,
        variables: serde_json::from_str(&variables)?,
    })
}

pub fn create_collection(
    db: &Connection,
    name: &str,
    description: &str,
    variables: &Variables,
) -> Result<Collection, StoreError> {
    non_empty(name)?;
    db.execute(
        "INSERT INTO collections (name, description, variables) VALUES (?1, ?2, ?3)",
        params![name, description, serde_json::to_string(variables)?],
    )?;
    Ok(Collection {
        id: CollectionId(db.last_insert_rowid()),
        name: name.to_owned(),
        description: description.to_owned(),
        variables: variables.clone(),
    })
}

pub fn collections(db: &Connection) -> Result<Vec<Collection>, StoreError> {
    let mut statement =
        db.prepare("SELECT id, name, description, variables FROM collections ORDER BY name, id")?;
    let rows = statement
        .query_map([], collection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(to_collection).collect()
}

pub fn collection(db: &Connection, id: CollectionId) -> Result<Option<Collection>, StoreError> {
    db.query_row(
        "SELECT id, name, description, variables FROM collections WHERE id = ?1",
        [id.0],
        collection_row,
    )
    .optional()?
    .map(to_collection)
    .transpose()
}

pub fn rename_collection(db: &Connection, id: CollectionId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    changed(
        db.execute(
            "UPDATE collections SET name = ?1 WHERE id = ?2",
            params![name, id.0],
        )?,
        "collection",
    )
}

pub fn set_collection_variables(
    db: &Connection,
    id: CollectionId,
    variables: &Variables,
) -> Result<(), StoreError> {
    changed(
        db.execute(
            "UPDATE collections SET variables = ?1 WHERE id = ?2",
            params![serde_json::to_string(variables)?, id.0],
        )?,
        "collection",
    )
}

/// Deletes its folders and requests too. Execution history stays.
pub fn delete_collection(db: &Connection, id: CollectionId) -> Result<(), StoreError> {
    changed(
        db.execute("DELETE FROM collections WHERE id = ?1", [id.0])?,
        "collection",
    )
}

pub fn tree(db: &Connection, id: CollectionId) -> Result<Option<Tree>, StoreError> {
    let Some(collection) = collection(db, id)? else {
        return Ok(None);
    };
    let folders = folders(db, id)?;
    let mut statement = db.prepare(
        "SELECT id, folder_id, name, method, position FROM requests
         WHERE collection_id = ?1 ORDER BY position, id",
    )?;
    let requests = statement
        .query_map([id.0], |row| {
            Ok(RequestSummary {
                id: RequestId(row.get(0)?),
                folder_id: row.get::<_, Option<i64>>(1)?.map(FolderId),
                name: row.get(2)?,
                method: row.get(3)?,
                position: row.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Some(Tree {
        collection,
        folders,
        requests,
    }))
}

fn folder_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: FolderId(row.get(0)?),
        collection_id: CollectionId(row.get(1)?),
        parent_id: row.get::<_, Option<i64>>(2)?.map(FolderId),
        name: row.get(3)?,
        description: row.get(4)?,
        position: row.get(5)?,
    })
}

const FOLDER_COLUMNS: &str = "id, collection_id, parent_folder_id, name, description, position";

pub fn folders(db: &Connection, collection: CollectionId) -> Result<Vec<Folder>, StoreError> {
    let mut statement = db.prepare(&format!(
        "SELECT {FOLDER_COLUMNS} FROM folders WHERE collection_id = ?1 ORDER BY position, id"
    ))?;
    Ok(statement
        .query_map([collection.0], folder_row)?
        .collect::<Result<_, _>>()?)
}

pub fn folder(db: &Connection, id: FolderId) -> Result<Option<Folder>, StoreError> {
    Ok(db
        .query_row(
            &format!("SELECT {FOLDER_COLUMNS} FROM folders WHERE id = ?1"),
            [id.0],
            folder_row,
        )
        .optional()?)
}

fn folder_in(
    db: &Connection,
    id: FolderId,
    collection: CollectionId,
    conflict: Conflict,
) -> Result<(), StoreError> {
    match folder(db, id)? {
        Some(folder) if folder.collection_id == collection => Ok(()),
        _ => Err(conflict.into()),
    }
}

/// The next position among siblings, so new items go last.
pub fn next_position(
    db: &Connection,
    collection: CollectionId,
    parent: Option<FolderId>,
) -> Result<i64, StoreError> {
    let parent = parent.map(|id| id.0);
    let folders: i64 = db.query_row(
        "SELECT coalesce(max(position) + 1, 0) FROM folders
         WHERE collection_id = ?1 AND parent_folder_id IS ?2",
        params![collection.0, parent],
        |row| row.get(0),
    )?;
    let requests: i64 = db.query_row(
        "SELECT coalesce(max(position) + 1, 0) FROM requests WHERE collection_id = ?1 AND folder_id IS ?2",
        params![collection.0, parent],
        |row| row.get(0),
    )?;
    Ok(folders.max(requests))
}

pub fn create_folder(
    db: &Connection,
    collection: CollectionId,
    parent: Option<FolderId>,
    name: &str,
    description: &str,
    position: i64,
) -> Result<Folder, StoreError> {
    non_empty(name)?;
    if self::collection(db, collection)?.is_none() {
        return Err(StoreError::NotFound("collection"));
    }
    if let Some(parent) = parent {
        folder_in(db, parent, collection, Conflict::ParentOutsideCollection)?;
    }
    db.execute(
        "INSERT INTO folders (collection_id, parent_folder_id, name, description, position)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            collection.0,
            parent.map(|id| id.0),
            name,
            description,
            position
        ],
    )?;
    Ok(Folder {
        id: FolderId(db.last_insert_rowid()),
        collection_id: collection,
        parent_id: parent,
        name: name.to_owned(),
        description: description.to_owned(),
        position,
    })
}

pub fn rename_folder(db: &Connection, id: FolderId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    changed(
        db.execute(
            "UPDATE folders SET name = ?1 WHERE id = ?2",
            params![name, id.0],
        )?,
        "folder",
    )
}

/// Deletes nested folders and requests too.
pub fn delete_folder(db: &Connection, id: FolderId) -> Result<(), StoreError> {
    changed(
        db.execute("DELETE FROM folders WHERE id = ?1", [id.0])?,
        "folder",
    )
}

/// Move within the same collection; a folder cannot move into itself or its descendants.
pub fn move_folder(
    db: &Connection,
    id: FolderId,
    parent: Option<FolderId>,
) -> Result<(), StoreError> {
    let moving = folder(db, id)?.ok_or(StoreError::NotFound("folder"))?;
    let all = folders(db, moving.collection_id)?;
    let mut ancestor = parent;
    while let Some(current) = ancestor {
        if current == id {
            return Err(Conflict::MoveIntoItself.into());
        }
        let node = all
            .iter()
            .find(|f| f.id == current)
            .ok_or(Conflict::CrossCollection)?;
        ancestor = node.parent_id;
    }
    db.execute(
        "UPDATE folders SET parent_folder_id = ?1 WHERE id = ?2",
        params![parent.map(|p| p.0), id.0],
    )?;
    Ok(())
}

fn to_request(
    id: i64,
    collection: i64,
    folder: Option<i64>,
    template: &str,
) -> Result<Request, StoreError> {
    Ok(Request {
        id: Some(RequestId(id)),
        collection_id: Some(CollectionId(collection)),
        folder_id: folder.map(FolderId),
        template: serde_json::from_str(template)?,
        collection_variables_snapshot: Variables::new(),
    })
}

pub fn create_request(
    db: &Connection,
    collection: CollectionId,
    folder: Option<FolderId>,
    template: &RequestTemplate,
) -> Result<Request, StoreError> {
    if self::collection(db, collection)?.is_none() {
        return Err(StoreError::NotFound("collection"));
    }
    if let Some(folder) = folder {
        folder_in(db, folder, collection, Conflict::CrossCollection)?;
    }
    db.execute(
        "INSERT INTO requests (collection_id, folder_id, template) VALUES (?1, ?2, ?3)",
        params![
            collection.0,
            folder.map(|id| id.0),
            serde_json::to_string(template)?
        ],
    )?;
    Ok(Request {
        id: Some(RequestId(db.last_insert_rowid())),
        collection_id: Some(collection),
        folder_id: folder,
        template: template.clone(),
        collection_variables_snapshot: Variables::new(),
    })
}

pub fn request(db: &Connection, id: RequestId) -> Result<Option<Request>, StoreError> {
    db.query_row(
        "SELECT id, collection_id, folder_id, template FROM requests WHERE id = ?1",
        [id.0],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .optional()?
    .map(|(id, collection, folder, template)| to_request(id, collection, folder, &template))
    .transpose()
}

/// Every request of a collection with its full template (for export).
pub fn requests(db: &Connection, collection: CollectionId) -> Result<Vec<Request>, StoreError> {
    let mut statement = db.prepare(
        "SELECT id, collection_id, folder_id, template FROM requests
         WHERE collection_id = ?1 ORDER BY position, id",
    )?;
    let rows = statement
        .query_map([collection.0], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, c, f, t)| to_request(id, c, f, &t))
        .collect()
}

/// Save the template of an existing request. Its place in the tree does not change.
pub fn update_request(
    db: &Connection,
    id: RequestId,
    template: &RequestTemplate,
) -> Result<(), StoreError> {
    changed(
        db.execute(
            "UPDATE requests SET template = ?1 WHERE id = ?2",
            params![serde_json::to_string(template)?, id.0],
        )?,
        "request",
    )
}

pub fn rename_request(db: &Connection, id: RequestId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    changed(
        db.execute(
            "UPDATE requests SET template = json_set(template, '$.name', ?1) WHERE id = ?2",
            params![name, id.0],
        )?,
        "request",
    )
}

/// Deletes the request; its execution history stays.
pub fn delete_request(db: &Connection, id: RequestId) -> Result<(), StoreError> {
    changed(
        db.execute("DELETE FROM requests WHERE id = ?1", [id.0])?,
        "request",
    )
}

/// Move within the same collection, to a folder or the collection root.
pub fn move_request(
    db: &Connection,
    id: RequestId,
    folder: Option<FolderId>,
) -> Result<(), StoreError> {
    let collection: i64 = db
        .query_row(
            "SELECT collection_id FROM requests WHERE id = ?1",
            [id.0],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(StoreError::NotFound("request"))?;
    if let Some(folder) = folder {
        folder_in(
            db,
            folder,
            CollectionId(collection),
            Conflict::CrossCollection,
        )?;
    }
    db.execute(
        "UPDATE requests SET folder_id = ?1 WHERE id = ?2",
        params![folder.map(|f| f.0), id.0],
    )?;
    Ok(())
}

/// Set positions for siblings in the given order (drag and drop reordering).
pub fn reorder(
    db: &Connection,
    folders: &[FolderId],
    requests: &[RequestId],
) -> Result<(), StoreError> {
    let mut position = 0_i64;
    for id in folders {
        changed(
            db.execute(
                "UPDATE folders SET position = ?1 WHERE id = ?2",
                params![position, id.0],
            )?,
            "folder",
        )?;
        position += 1;
    }
    for id in requests {
        changed(
            db.execute(
                "UPDATE requests SET template = json_set(template, '$.position', ?1) WHERE id = ?2",
                params![position, id.0],
            )?,
            "request",
        )?;
        position += 1;
    }
    Ok(())
}
