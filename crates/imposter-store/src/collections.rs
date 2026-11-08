//! Collections, folders and requests, kept as files in the collections folder
//! (`files`). Each function reads or changes the folder as the command sees it; the
//! files are written when the command commits. Ids are the stable numbers the profile
//! keeps for each file's id.

use imposter_core::ids::{CollectionId, FolderId, RequestId};
use imposter_core::model::{Request, RequestTemplate, SavedValues, Scripts, Variables};

use crate::files::model::{CollectionRow, FolderRow, Model, RequestRow};
use crate::files::workspace::{Ids, Kind, State, new_uid};
use crate::{Conflict, StoreError, Tx};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub description: String,
    /// The values in effect: the file's, with this computer's local values over them.
    pub variables: Variables,
    /// Run around every request in the collection, before its folders' and its own.
    pub scripts: Scripts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub collection_id: CollectionId,
    pub parent_id: Option<FolderId>,
    pub name: String,
    pub description: String,
    pub position: i64,
    /// Run around every request inside the folder, after its parent's.
    pub scripts: Scripts,
}

/// What the tree shows for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSummary {
    pub id: RequestId,
    pub folder_id: Option<FolderId>,
    pub name: String,
    pub method: String,
    pub url: String,
    pub position: i64,
}

/// A whole collection for the tree. Folders and requests are ordered by position, then
/// id; nesting is given by their parent ids.
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

// ---- ids and rows ----------------------------------------------------------------------

fn id_of(ids: &Ids, uid: &str) -> i64 {
    // Every row's file id was given a number when read or created.
    ids.id(uid).unwrap_or(0)
}

fn collection_uid(state: &State, id: CollectionId) -> Option<String> {
    let uid = state.ids.uid(Kind::Collection, id.0)?;
    state
        .model
        .collections
        .iter()
        .any(|c| c.uid == uid)
        .then(|| uid.to_owned())
}

fn folder_uid(state: &State, id: FolderId) -> Option<String> {
    let uid = state.ids.uid(Kind::Folder, id.0)?;
    state
        .model
        .folders
        .iter()
        .any(|f| f.uid == uid)
        .then(|| uid.to_owned())
}

fn request_uid(state: &State, id: RequestId) -> Option<String> {
    let uid = state.ids.uid(Kind::Request, id.0)?;
    state
        .model
        .requests
        .iter()
        .any(|r| r.uid == uid)
        .then(|| uid.to_owned())
}

fn to_folder(ids: &Ids, row: &FolderRow) -> Folder {
    Folder {
        id: FolderId(id_of(ids, &row.uid)),
        collection_id: CollectionId(id_of(ids, &row.collection)),
        parent_id: row.parent.as_deref().map(|p| FolderId(id_of(ids, p))),
        name: row.name.clone(),
        description: row.description.clone(),
        position: row.position,
        scripts: row.scripts.clone(),
    }
}

fn to_request(ids: &Ids, row: &RequestRow) -> Request {
    Request {
        id: Some(RequestId(id_of(ids, &row.uid))),
        collection_id: Some(CollectionId(id_of(ids, &row.collection))),
        folder_id: row.folder.as_deref().map(|f| FolderId(id_of(ids, f))),
        template: row.template.clone(),
        collection_variables_snapshot: Variables::new(),
    }
}

fn to_collection(db: &Tx<'_>, ids: &Ids, row: &CollectionRow) -> Result<Collection, StoreError> {
    let (local, _) = crate::scopes::local(db, &row.uid)?;
    let mut variables = row.variables.clone();
    for (name, value) in local {
        variables.insert(name, value);
    }
    Ok(Collection {
        id: CollectionId(id_of(ids, &row.uid)),
        name: row.name.clone(),
        description: row.description.clone(),
        variables,
        scripts: row.scripts.clone(),
    })
}

// ---- collections -----------------------------------------------------------------------

pub fn set_collection_scripts(
    db: &Tx<'_>,
    id: CollectionId,
    scripts: &Scripts,
) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = collection_uid(state, id).ok_or(StoreError::NotFound("collection"))?;
        if let Some(row) = state.model.collections.iter_mut().find(|c| c.uid == uid) {
            row.scripts = scripts.clone();
        }
        Ok(())
    })
}

pub fn set_folder_scripts(db: &Tx<'_>, id: FolderId, scripts: &Scripts) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = folder_uid(state, id).ok_or(StoreError::NotFound("folder"))?;
        if let Some(row) = state.model.folders.iter_mut().find(|f| f.uid == uid) {
            row.scripts = scripts.clone();
        }
        Ok(())
    })
}

pub fn create_collection(
    db: &Tx<'_>,
    name: &str,
    description: &str,
    variables: &Variables,
) -> Result<Collection, StoreError> {
    non_empty(name)?;
    db.write(|state, sql| {
        let uid = new_uid();
        let id = state.ids.assign(sql, Kind::Collection, &uid)?;
        state.model.collections.push(CollectionRow {
            uid,
            name: name.to_owned(),
            description: description.to_owned(),
            variables: variables.clone(),
            saved_values: SavedValues::default(),
            scripts: Scripts::default(),
        });
        Ok(Collection {
            id: CollectionId(id),
            name: name.to_owned(),
            description: description.to_owned(),
            variables: variables.clone(),
            scripts: Scripts::default(),
        })
    })
}

/// Every collection, by name then id.
pub fn collections(db: &Tx<'_>) -> Result<Vec<Collection>, StoreError> {
    let mut out = db.read(|state| {
        state
            .model
            .collections
            .iter()
            .map(|row| to_collection(db, &state.ids, row))
            .collect::<Result<Vec<_>, _>>()
    })?;
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}

pub fn collection(db: &Tx<'_>, id: CollectionId) -> Result<Option<Collection>, StoreError> {
    db.read(|state| {
        let Some(uid) = collection_uid(state, id) else {
            return Ok(None);
        };
        let row = state.model.collections.iter().find(|c| c.uid == uid);
        row.map(|row| to_collection(db, &state.ids, row))
            .transpose()
    })
}

pub fn rename_collection(db: &Tx<'_>, id: CollectionId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    db.write(|state, _| {
        let uid = collection_uid(state, id).ok_or(StoreError::NotFound("collection"))?;
        if let Some(row) = state.model.collections.iter_mut().find(|c| c.uid == uid) {
            name.clone_into(&mut row.name);
        }
        Ok(())
    })
}

/// Replace the collection's shared values (its file's), as an explicit edit does.
pub fn set_collection_variables(
    db: &Tx<'_>,
    id: CollectionId,
    variables: &Variables,
) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = collection_uid(state, id).ok_or(StoreError::NotFound("collection"))?;
        if let Some(row) = state.model.collections.iter_mut().find(|c| c.uid == uid) {
            row.variables.clone_from(variables);
        }
        Ok(())
    })
}

/// Deletes its folders and requests too (their files go). Execution history stays.
pub fn delete_collection(db: &Tx<'_>, id: CollectionId) -> Result<(), StoreError> {
    let uid = db.write(|state, _| {
        let uid = collection_uid(state, id).ok_or(StoreError::NotFound("collection"))?;
        let model = &mut state.model;
        model.collections.retain(|c| c.uid != uid);
        model.folders.retain(|f| f.collection != uid);
        model.requests.retain(|r| r.collection != uid);
        Ok(uid)
    })?;
    crate::scopes::forget_local(db, &uid)
}

pub fn tree(db: &Tx<'_>, id: CollectionId) -> Result<Option<Tree>, StoreError> {
    let Some(collection) = collection(db, id)? else {
        return Ok(None);
    };
    let folders = folders(db, id)?;
    let requests = db.read(|state| {
        let uid = collection_uid(state, id).unwrap_or_default();
        let mut requests: Vec<RequestSummary> = state
            .model
            .requests
            .iter()
            .filter(|r| r.collection == uid)
            .map(|r| RequestSummary {
                id: RequestId(id_of(&state.ids, &r.uid)),
                folder_id: r.folder.as_deref().map(|f| FolderId(id_of(&state.ids, f))),
                name: r.template.name.clone(),
                method: r.template.method.clone(),
                url: r.template.url.clone(),
                position: r.template.position,
            })
            .collect();
        requests.sort_by_key(|r| (r.position, r.id));
        requests
    });
    Ok(Some(Tree {
        collection,
        folders,
        requests,
    }))
}

/// A request as the command palette lists it: where it lives and what it is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRequest {
    pub id: RequestId,
    pub collection_id: CollectionId,
    pub collection: String,
    /// Folder names from the collection root down.
    pub folders: Vec<String>,
    pub name: String,
    pub method: String,
}

/// Folder names from the collection root down to `parent`.
fn folder_path<'m>(model: &'m Model, mut parent: Option<&'m str>) -> Vec<String> {
    let mut names = Vec::new();
    while let Some(folder) = parent.and_then(|p| model.folders.iter().find(|f| f.uid == p)) {
        // Parents form a tree; the bound only guards a malformed folder.
        if names.len() > model.folders.len() {
            break;
        }
        names.push(folder.name.clone());
        parent = folder.parent.as_deref();
    }
    names.reverse();
    names
}

/// Every request of every collection with its folder path: by collection name, then
/// folder path (root requests first), then position.
pub fn request_index(db: &Tx<'_>) -> Result<Vec<IndexedRequest>, StoreError> {
    Ok(db.read(|state| {
        let model = &state.model;
        let mut index: Vec<(i64, IndexedRequest)> = model
            .requests
            .iter()
            .filter_map(|r| {
                let collection = model.collections.iter().find(|c| c.uid == r.collection)?;
                Some((
                    r.template.position,
                    IndexedRequest {
                        id: RequestId(id_of(&state.ids, &r.uid)),
                        collection_id: CollectionId(id_of(&state.ids, &collection.uid)),
                        collection: collection.name.clone(),
                        folders: folder_path(model, r.folder.as_deref()),
                        name: r.template.name.clone(),
                        method: r.template.method.clone(),
                    },
                ))
            })
            .collect();
        index.sort_by(|(pa, a), (pb, b)| {
            (&a.collection, a.collection_id, &a.folders, *pa, a.id).cmp(&(
                &b.collection,
                b.collection_id,
                &b.folders,
                *pb,
                b.id,
            ))
        });
        index.into_iter().map(|(_, r)| r).collect()
    }))
}

// ---- folders ---------------------------------------------------------------------------

pub fn folders(db: &Tx<'_>, collection: CollectionId) -> Result<Vec<Folder>, StoreError> {
    Ok(db.read(|state| {
        let Some(uid) = collection_uid(state, collection) else {
            return Vec::new();
        };
        let mut folders: Vec<Folder> = state
            .model
            .folders
            .iter()
            .filter(|f| f.collection == uid)
            .map(|f| to_folder(&state.ids, f))
            .collect();
        folders.sort_by_key(|f| (f.position, f.id));
        folders
    }))
}

pub fn folder(db: &Tx<'_>, id: FolderId) -> Result<Option<Folder>, StoreError> {
    Ok(db.read(|state| {
        let uid = folder_uid(state, id)?;
        state
            .model
            .folders
            .iter()
            .find(|f| f.uid == uid)
            .map(|f| to_folder(&state.ids, f))
    }))
}

/// The next position among siblings, so new items go last.
pub fn next_position(
    db: &Tx<'_>,
    collection: CollectionId,
    parent: Option<FolderId>,
) -> Result<i64, StoreError> {
    Ok(db.read(|state| {
        let Some(collection) = collection_uid(state, collection) else {
            return 0;
        };
        let parent = parent.and_then(|p| folder_uid(state, p));
        let model = &state.model;
        let folders = model
            .folders
            .iter()
            .filter(|f| f.collection == collection && f.parent == parent)
            .map(|f| f.position);
        let requests = model
            .requests
            .iter()
            .filter(|r| r.collection == collection && r.folder == parent)
            .map(|r| r.template.position);
        folders.chain(requests).max().map_or(0, |max| max + 1)
    }))
}

pub fn create_folder(
    db: &Tx<'_>,
    collection: CollectionId,
    parent: Option<FolderId>,
    name: &str,
    description: &str,
    position: i64,
) -> Result<Folder, StoreError> {
    non_empty(name)?;
    db.write(|state, sql| {
        let collection_uid =
            collection_uid(state, collection).ok_or(StoreError::NotFound("collection"))?;
        let parent_uid = match parent {
            Some(parent) => Some(folder_in(
                state,
                parent,
                &collection_uid,
                Conflict::ParentOutsideCollection,
            )?),
            None => None,
        };
        let uid = new_uid();
        let id = state.ids.assign(sql, Kind::Folder, &uid)?;
        state.model.folders.push(FolderRow {
            uid,
            collection: collection_uid,
            parent: parent_uid,
            name: name.to_owned(),
            description: description.to_owned(),
            position,
            scripts: Scripts::default(),
        });
        Ok(Folder {
            id: FolderId(id),
            collection_id: collection,
            parent_id: parent,
            name: name.to_owned(),
            description: description.to_owned(),
            position,
            scripts: Scripts::default(),
        })
    })
}

/// The uid of folder `id` when it belongs to `collection`, else `conflict`.
fn folder_in(
    state: &State,
    id: FolderId,
    collection: &str,
    conflict: Conflict,
) -> Result<String, StoreError> {
    let uid = folder_uid(state, id).ok_or(conflict)?;
    let inside = state
        .model
        .folders
        .iter()
        .any(|f| f.uid == uid && f.collection == collection);
    if inside {
        Ok(uid)
    } else {
        Err(conflict.into())
    }
}

pub fn rename_folder(db: &Tx<'_>, id: FolderId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    db.write(|state, _| {
        let uid = folder_uid(state, id).ok_or(StoreError::NotFound("folder"))?;
        if let Some(row) = state.model.folders.iter_mut().find(|f| f.uid == uid) {
            name.clone_into(&mut row.name);
        }
        Ok(())
    })
}

/// Deletes nested folders and requests too.
pub fn delete_folder(db: &Tx<'_>, id: FolderId) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = folder_uid(state, id).ok_or(StoreError::NotFound("folder"))?;
        let gone = state.model.folder_and_descendants(&uid);
        let model = &mut state.model;
        model.folders.retain(|f| !gone.contains(&f.uid));
        model
            .requests
            .retain(|r| r.folder.as_ref().is_none_or(|f| !gone.contains(f)));
        Ok(())
    })
}

/// Move within the same collection, to the end of `parent`; a folder cannot move into
/// itself or its descendants.
pub fn move_folder(db: &Tx<'_>, id: FolderId, parent: Option<FolderId>) -> Result<(), StoreError> {
    place(db, Item::Folder(id), parent, usize::MAX)
}

// ---- requests --------------------------------------------------------------------------

pub fn create_request(
    db: &Tx<'_>,
    collection: CollectionId,
    folder: Option<FolderId>,
    template: &RequestTemplate,
) -> Result<Request, StoreError> {
    db.write(|state, sql| {
        let collection_uid =
            collection_uid(state, collection).ok_or(StoreError::NotFound("collection"))?;
        let folder_uid = match folder {
            Some(folder) => Some(folder_in(
                state,
                folder,
                &collection_uid,
                Conflict::CrossCollection,
            )?),
            None => None,
        };
        let uid = new_uid();
        let id = state.ids.assign(sql, Kind::Request, &uid)?;
        state.model.requests.push(RequestRow {
            uid,
            collection: collection_uid,
            folder: folder_uid,
            template: template.clone(),
        });
        Ok(Request {
            id: Some(RequestId(id)),
            collection_id: Some(collection),
            folder_id: folder,
            template: template.clone(),
            collection_variables_snapshot: Variables::new(),
        })
    })
}

pub fn request(db: &Tx<'_>, id: RequestId) -> Result<Option<Request>, StoreError> {
    Ok(db.read(|state| {
        let uid = request_uid(state, id)?;
        state
            .model
            .requests
            .iter()
            .find(|r| r.uid == uid)
            .map(|r| to_request(&state.ids, r))
    }))
}

/// Every request of a collection with its full template, by position then id.
pub fn requests(db: &Tx<'_>, collection: CollectionId) -> Result<Vec<Request>, StoreError> {
    Ok(db.read(|state| {
        let Some(uid) = collection_uid(state, collection) else {
            return Vec::new();
        };
        let mut requests: Vec<Request> = state
            .model
            .requests
            .iter()
            .filter(|r| r.collection == uid)
            .map(|r| to_request(&state.ids, r))
            .collect();
        requests.sort_by_key(|r| (r.template.position, r.id));
        requests
    }))
}

/// Save the template of an existing request. Its place in the tree does not change: the
/// stored position is kept, since the editor's copy goes stale when the tree is reordered.
pub fn update_request(
    db: &Tx<'_>,
    id: RequestId,
    template: &RequestTemplate,
) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = request_uid(state, id).ok_or(StoreError::NotFound("request"))?;
        if let Some(row) = state.model.requests.iter_mut().find(|r| r.uid == uid) {
            let position = row.template.position;
            row.template = template.clone();
            row.template.position = position;
        }
        Ok(())
    })
}

pub fn rename_request(db: &Tx<'_>, id: RequestId, name: &str) -> Result<(), StoreError> {
    non_empty(name)?;
    db.write(|state, _| {
        let uid = request_uid(state, id).ok_or(StoreError::NotFound("request"))?;
        if let Some(row) = state.model.requests.iter_mut().find(|r| r.uid == uid) {
            name.clone_into(&mut row.template.name);
        }
        Ok(())
    })
}

/// Deletes the request; its execution history stays.
pub fn delete_request(db: &Tx<'_>, id: RequestId) -> Result<(), StoreError> {
    db.write(|state, _| {
        let uid = request_uid(state, id).ok_or(StoreError::NotFound("request"))?;
        state.model.requests.retain(|r| r.uid != uid);
        Ok(())
    })
}

/// Move within the same collection, to the end of a folder or the collection root.
pub fn move_request(
    db: &Tx<'_>,
    id: RequestId,
    folder: Option<FolderId>,
) -> Result<(), StoreError> {
    place(db, Item::Request(id), folder, usize::MAX)
}

/// A folder or a request: what can be placed among siblings in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Folder(FolderId),
    Request(RequestId),
}

/// Put `item` under `parent` (a folder, or the collection root) at `index` among its new
/// siblings, counted without it; an index past the end puts it last. The siblings are then
/// numbered 0, 1, 2… in the order the tree shows (position, then id; folders and requests
/// mixed), so no two share a position.
pub fn place(
    db: &Tx<'_>,
    item: Item,
    parent: Option<FolderId>,
    index: usize,
) -> Result<(), StoreError> {
    db.write(|state, _| {
        let collection = match item {
            Item::Folder(id) => reparent_folder(state, id, parent)?,
            Item::Request(id) => refile_request(state, id, parent)?,
        };
        let parent = parent.and_then(|p| folder_uid(state, p));
        let mut siblings = siblings(state, &collection, parent.as_deref());
        siblings.retain(|sibling| *sibling != item);
        siblings.insert(index.min(siblings.len()), item);
        for (position, sibling) in siblings.into_iter().enumerate() {
            let position = i64::try_from(position).unwrap_or(i64::MAX);
            set_position(state, sibling, position);
        }
        Ok(())
    })
}

fn set_position(state: &mut State, item: Item, position: i64) {
    match item {
        Item::Folder(id) => {
            if let Some(uid) = folder_uid(state, id)
                && let Some(row) = state.model.folders.iter_mut().find(|f| f.uid == uid)
            {
                row.position = position;
            }
        }
        Item::Request(id) => {
            if let Some(uid) = request_uid(state, id)
                && let Some(row) = state.model.requests.iter_mut().find(|r| r.uid == uid)
            {
                row.template.position = position;
            }
        }
    }
}

/// Set a folder's parent after checking it stays in the collection and out of itself.
/// Returns the collection's uid.
fn reparent_folder(
    state: &mut State,
    id: FolderId,
    parent: Option<FolderId>,
) -> Result<String, StoreError> {
    let uid = folder_uid(state, id).ok_or(StoreError::NotFound("folder"))?;
    let collection = state
        .model
        .folders
        .iter()
        .find(|f| f.uid == uid)
        .map(|f| f.collection.clone())
        .unwrap_or_default();
    let parent_uid = match parent {
        Some(parent) => Some(folder_uid(state, parent).ok_or(Conflict::CrossCollection)?),
        None => None,
    };
    let mut ancestor = parent_uid.as_deref();
    while let Some(current) = ancestor {
        if current == uid {
            return Err(Conflict::MoveIntoItself.into());
        }
        let node = state
            .model
            .folders
            .iter()
            .find(|f| f.uid == current && f.collection == collection)
            .ok_or(Conflict::CrossCollection)?;
        ancestor = node.parent.as_deref();
    }
    if let Some(row) = state.model.folders.iter_mut().find(|f| f.uid == uid) {
        row.parent = parent_uid;
    }
    Ok(collection)
}

/// Set a request's folder after checking it stays in the collection. Returns the
/// collection's uid.
fn refile_request(
    state: &mut State,
    id: RequestId,
    folder: Option<FolderId>,
) -> Result<String, StoreError> {
    let uid = request_uid(state, id).ok_or(StoreError::NotFound("request"))?;
    let collection = state
        .model
        .requests
        .iter()
        .find(|r| r.uid == uid)
        .map(|r| r.collection.clone())
        .unwrap_or_default();
    let folder_uid = match folder {
        Some(folder) => Some(folder_in(
            state,
            folder,
            &collection,
            Conflict::CrossCollection,
        )?),
        None => None,
    };
    if let Some(row) = state.model.requests.iter_mut().find(|r| r.uid == uid) {
        row.folder = folder_uid;
    }
    Ok(collection)
}

/// The children of `parent` in the order the tree shows them: position, then id, requests
/// before folders on a full tie, as the sidebar's stable sort leaves them.
fn siblings(state: &State, collection: &str, parent: Option<&str>) -> Vec<Item> {
    let model: &Model = &state.model;
    let mut keyed: Vec<((i64, i64, u8), Item)> = Vec::new();
    for r in model
        .requests
        .iter()
        .filter(|r| r.collection == collection && r.folder.as_deref() == parent)
    {
        let id = id_of(&state.ids, &r.uid);
        keyed.push(((r.template.position, id, 0), Item::Request(RequestId(id))));
    }
    for f in model
        .folders
        .iter()
        .filter(|f| f.collection == collection && f.parent.as_deref() == parent)
    {
        let id = id_of(&state.ids, &f.uid);
        keyed.push(((f.position, id, 1), Item::Folder(FolderId(id))));
    }
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, item)| item).collect()
}
