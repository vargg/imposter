//! Collection import and export: one-transaction snapshots, previews and atomic imports.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use imposter_core::ids::{CollectionId, EnvironmentId, FolderId};
use imposter_core::model::{Request, RequestTemplate};
use imposter_core::variables::VariableContext;
use imposter_store::collections::{self, Folder};
use imposter_store::{StoreError, StoreHandle, Tx, environments};
use imposter_transfer::native::{
    self, Name, NativeDocument, NativeError, TransferCollection, TransferEnvironment,
    TransferFolder, unique_name,
};
use imposter_transfer::postman::{self, Decoder, PostmanError};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error(transparent)]
    Postman(#[from] PostmanError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Cannot read the file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Collection file must contain a JSON object")]
    NotAnObject,
    #[error("Native files embed optional environments and globals")]
    NativeCompanions,
    #[error("Explicitly remove unsupported execution features or cancel the import")]
    ConsentRequired,
    #[error("Cannot write the export: {0}")]
    Io(#[from] std::io::Error),
    #[error("The background task stopped unexpectedly")]
    Task,
}

/// What an import would create, shown before anything is written.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPreview {
    pub document: NativeDocument,
    pub collection_name: String,
    pub environment_names: Vec<String>,
    pub warnings: Vec<String>,
    /// Losses change what requests send: the user must agree to remove them.
    pub requires_removal: bool,
}

/// What to include in an export.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExportScope {
    pub collection: Option<CollectionId>,
    pub environments: Vec<EnvironmentId>,
    pub globals: bool,
    /// The editor's current request, exported in place of its saved version.
    pub draft: Option<Request>,
}

/// Sibling positions must be unique to survive fresh ids: renumber groups with ties by
/// (position, id), the order the tree shows.
fn normalize_positions(folders: &mut [Folder], requests: &mut [Request]) {
    #[derive(Clone, Copy)]
    enum Item {
        Folder(usize),
        Request(usize),
    }
    let mut groups: HashMap<Option<FolderId>, Vec<(i64, i64, Item)>> = HashMap::new();
    for (index, folder) in folders.iter().enumerate() {
        groups.entry(folder.parent_id).or_default().push((
            folder.position,
            folder.id.0,
            Item::Folder(index),
        ));
    }
    for (index, request) in requests.iter().enumerate() {
        let id = request.id.map_or(i64::MAX, |id| id.0);
        groups.entry(request.folder_id).or_default().push((
            request.template.position,
            id,
            Item::Request(index),
        ));
    }
    for children in groups.values_mut() {
        let unique: HashSet<i64> = children.iter().map(|(position, _, _)| *position).collect();
        if unique.len() == children.len() {
            continue;
        }
        children.sort_by_key(|(position, id, _)| (*position, *id));
        for (position, (_, _, item)) in children.iter().enumerate() {
            let position = i64::try_from(position).unwrap_or(i64::MAX);
            match *item {
                Item::Folder(index) => folders[index].position = position,
                Item::Request(index) => requests[index].template.position = position,
            }
        }
    }
}

/// Capture one collection consistently, optionally with environments, globals and the draft.
pub fn snapshot(db: &Tx<'_>, scope: &ExportScope) -> Result<NativeDocument, StoreError> {
    let id = scope.collection.ok_or(StoreError::NotFound("collection"))?;
    let collection = collections::collection(db, id)?.ok_or(StoreError::NotFound("collection"))?;
    let mut folders = collections::folders(db, id)?;
    let mut requests = collections::requests(db, id)?;
    if let Some(draft) = &scope.draft
        && (draft.collection_id == Some(id) || draft.id.is_none())
    {
        requests.retain(|request| request.id.is_none() || request.id != draft.id);
        let mut draft = draft.clone();
        if draft.id.is_none() {
            draft.folder_id = None;
        }
        requests.push(draft);
    }
    normalize_positions(&mut folders, &mut requests);
    let mut nodes: HashMap<FolderId, TransferFolder> = HashMap::new();
    for folder in &folders {
        nodes.insert(
            folder.id,
            TransferFolder {
                name: Name::new(folder.name.clone()).ok_or(StoreError::NotFound("collection"))?,
                description: folder.description.clone(),
                position: folder.position,
                folders: Vec::new(),
                requests: Vec::new(),
            },
        );
    }
    requests.sort_by_key(|r| (r.template.position, r.id.map_or(i64::MAX, |id| id.0)));
    let mut root_requests = Vec::new();
    for request in requests {
        match request.folder_id.and_then(|folder| nodes.get_mut(&folder)) {
            Some(node) => node.requests.push(request.template),
            None => root_requests.push(request.template),
        }
    }
    // Attach folders bottom-up: children before their parents are moved.
    let mut order: Vec<&Folder> = folders.iter().collect();
    let depth = |folder: &Folder| {
        let mut depth = 0;
        let mut parent = folder.parent_id;
        while let Some(id) = parent {
            depth += 1;
            parent = folders
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.parent_id);
        }
        depth
    };
    order.sort_by_key(|folder| std::cmp::Reverse(depth(folder)));
    let mut root_folders = Vec::new();
    for folder in order {
        let node = nodes
            .remove(&folder.id)
            .ok_or(StoreError::NotFound("collection"))?;
        match folder.parent_id.and_then(|parent| nodes.get_mut(&parent)) {
            Some(parent) => parent.folders.push(node),
            None => root_folders.push(node),
        }
    }
    sort_folders(&mut root_folders);
    let mut document = NativeDocument::new(TransferCollection {
        name: Name::new(collection.name).ok_or(StoreError::NotFound("collection"))?,
        description: collection.description,
        variables: collection.variables,
        folders: root_folders,
        requests: root_requests,
    });
    for environment in environments::environments(db)? {
        if scope.environments.contains(&environment.id) {
            document.environments.push(TransferEnvironment {
                name: Name::new(environment.name).ok_or(StoreError::NotFound("collection"))?,
                variables: environment.variables,
            });
        }
    }
    if scope.globals {
        document.globals = environments::globals(db)?;
    }
    Ok(document)
}

fn reserved_environment_names(db: &Tx<'_>) -> Result<HashSet<String>, StoreError> {
    let mut names: HashSet<String> = environments::environment_names(db)?.into_iter().collect();
    names.insert(VariableContext::GLOBAL_NAME.to_owned());
    Ok(names)
}

/// Fresh names and global conflicts, computed before anything is written. The document must
/// already be validated (`native::validate_templates`).
pub fn preview(db: &Tx<'_>, document: NativeDocument) -> Result<ImportPreview, StoreError> {
    let mut collection_names: HashSet<String> = collections::collections(db)?
        .into_iter()
        .map(|c| c.name)
        .collect();
    let collection_name = unique_name(document.collection.name.as_str(), &mut collection_names);
    let mut environment_names = reserved_environment_names(db)?;
    let environments = document
        .environments
        .iter()
        .map(|e| unique_name(e.name.as_str(), &mut environment_names))
        .collect();
    let existing = environments::globals(db)?;
    let warnings = document
        .globals
        .iter()
        .filter(|(key, value)| existing.get(*key).is_some_and(|current| current != *value))
        .map(|(key, _)| format!("Global {}: existing value retained", python_repr(key)))
        .collect();
    Ok(ImportPreview {
        document,
        collection_name,
        environment_names: environments,
        warnings,
        requires_removal: false,
    })
}

fn python_repr(text: &str) -> String {
    if text.contains('\'') && !text.contains('"') {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', "\\'"))
    }
}

/// Order folders by position at every depth.
fn sort_folders(folders: &mut [TransferFolder]) {
    folders.sort_by_key(|f| f.position);
    for folder in folders {
        sort_folders(&mut folder.folders);
    }
}

/// Create `folders` and `requests` under `parent`, depth first.
fn create_tree(
    db: &Tx<'_>,
    collection: CollectionId,
    folders: &[TransferFolder],
    requests: &[RequestTemplate],
    parent: Option<FolderId>,
) -> Result<(), StoreError> {
    for request in requests {
        collections::create_request(db, collection, parent, request)?;
    }
    for folder in folders {
        let created = collections::create_folder(
            db,
            collection,
            parent,
            folder.name.as_str(),
            &folder.description,
            folder.position,
        )?;
        create_tree(
            db,
            collection,
            &folder.folders,
            &folder.requests,
            Some(created.id),
        )?;
    }
    Ok(())
}

/// Import everything in one transaction with fresh ids; never changes existing collections,
/// environments or global values.
/// The caller checks consent and validity first (see [`import`]).
pub fn commit(db: &Tx<'_>, preview: &ImportPreview) -> Result<CollectionId, StoreError> {
    let document = &preview.document;
    let mut names: HashSet<String> = collections::collections(db)?
        .into_iter()
        .map(|c| c.name)
        .collect();
    let name = unique_name(document.collection.name.as_str(), &mut names);
    let collection = collections::create_collection(
        db,
        &name,
        &document.collection.description,
        &document.collection.variables,
    )?;
    create_tree(
        db,
        collection.id,
        &document.collection.folders,
        &document.collection.requests,
        None,
    )?;
    let mut environment_names = reserved_environment_names(db)?;
    for environment in &document.environments {
        let name = unique_name(environment.name.as_str(), &mut environment_names);
        environments::create_environment(db, &name, &environment.variables)?;
    }
    if !document.globals.is_empty() {
        let mut globals = environments::globals(db)?;
        for (key, value) in &document.globals {
            if !globals.contains_key(key) {
                globals.insert(key.clone(), value.clone());
            }
        }
        environments::set_globals(db, &globals)?;
    }
    Ok(collection.id)
}

/// Read a native or Postman file (with optional Postman companions) into a document, the
/// conversion warnings and whether losses need consent.
pub fn read(
    path: &Path,
    companions: &[PathBuf],
) -> Result<(NativeDocument, Vec<String>, bool), TransferError> {
    let data: Value = serde_json::from_slice(&native::read_document(path)?)?;
    let object = data.as_object().ok_or(TransferError::NotAnObject)?;
    if object.get("format").and_then(Value::as_str) == Some("poster") {
        if !companions.is_empty() {
            return Err(TransferError::NativeCompanions);
        }
        let document: NativeDocument = serde_json::from_value(data).map_err(NativeError::Json)?;
        return Ok((document, Vec::new(), false));
    }
    let mut decoder = Decoder::default();
    let mut document = decoder.decode(&data)?;
    for companion in companions {
        let companion: Value = serde_json::from_slice(&native::read_document(companion)?)?;
        decoder.companion(&companion, &mut document)?;
    }
    Ok((document, decoder.warnings, decoder.requires_removal))
}

/// Read a file and preview its import. File reading runs off the async threads.
pub async fn read_preview(
    store: &StoreHandle,
    path: PathBuf,
    companions: Vec<PathBuf>,
) -> Result<ImportPreview, TransferError> {
    let (document, warnings, requires_removal) =
        tokio::task::spawn_blocking(move || read(&path, &companions))
            .await
            .map_err(|_| TransferError::Task)??;
    native::validate_templates(&document)?;
    let mut preview = store.call(move |db| preview(db, document)).await?;
    preview.warnings.extend(warnings);
    preview.requires_removal = requires_removal;
    Ok(preview)
}

pub async fn import(
    store: &StoreHandle,
    preview: ImportPreview,
    remove_unsupported: bool,
) -> Result<CollectionId, TransferError> {
    if preview.requires_removal && !remove_unsupported {
        return Err(TransferError::ConsentRequired);
    }
    native::validate_templates(&preview.document)?;
    Ok(store.call(move |db| commit(db, &preview)).await?)
}

/// Export as native `.imposter.json`, published atomically.
pub async fn export_native(
    store: &StoreHandle,
    scope: ExportScope,
    path: PathBuf,
) -> Result<(), TransferError> {
    let document = store.call(move |db| snapshot(db, &scope)).await?;
    tokio::task::spawn_blocking(move || native::atomic_write(&path, &native::encode(&document)))
        .await
        .map_err(|_| TransferError::Task)??;
    Ok(())
}

/// The Postman export of `scope` and its warnings, for review before writing.
pub async fn postman_export(
    store: &StoreHandle,
    scope: ExportScope,
) -> Result<(NativeDocument, Value, Vec<String>), TransferError> {
    let document = store.call(move |db| snapshot(db, &scope)).await?;
    let (data, warnings) = postman::encode(&document)?;
    Ok((document, data, warnings))
}

/// Write a reviewed Postman export: one file, or a new directory with companions.
pub async fn write_postman(
    document: NativeDocument,
    data: Value,
    path: PathBuf,
) -> Result<(), TransferError> {
    tokio::task::spawn_blocking(move || -> Result<(), TransferError> {
        let bytes = postman::to_bytes(&data);
        if document.environments.is_empty() && document.globals.is_empty() {
            native::atomic_write(&path, &bytes)?;
        } else {
            postman::write_bundle(&path, &bytes, &document)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| TransferError::Task)?
}
