//! Collection import and export: one-transaction snapshots, previews and atomic imports.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use imposter_core::ids::{CollectionId, EnvironmentId, FolderId};
use imposter_core::model::{Request, RequestTemplate};
use imposter_core::variables::{Scope, VariableContext};
use imposter_store::collections::{self, Folder};
use imposter_store::{StoreError, StoreHandle, Tx, environments, scopes};
use imposter_transfer::native::{
    self, Name, NativeDocument, NativeError, TransferCollection, TransferEnvironment,
    TransferFolder, unique_name,
};
use imposter_transfer::openapi::{self, OpenApiError};
use imposter_transfer::postman::{self, Decoder, PostmanError};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error(transparent)]
    Postman(#[from] PostmanError),
    #[error(transparent)]
    OpenApi(#[from] OpenApiError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Cannot read the file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Collection file must contain a JSON object")]
    NotAnObject,
    #[error("Native files embed optional environments and globals")]
    NativeCompanions,
    #[error("OpenAPI files have no companion files")]
    OpenApiCompanions,
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
                scripts: folder.scripts.clone(),
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
        saved_values: scopes::saved_values(db, Scope::Collection(id))?,
        scripts: collection.scripts,
    });
    for environment in environments::environments(db)? {
        if scope.environments.contains(&environment.id) {
            document.environments.push(TransferEnvironment {
                name: Name::new(environment.name).ok_or(StoreError::NotFound("collection"))?,
                variables: environment.variables,
                saved_values: scopes::saved_values(db, Scope::Environment(environment.id))?,
                color: environment.color,
            });
        }
    }
    if scope.globals {
        let (globals, saved) = scopes::read(db, Scope::Global)?;
        document.globals = globals;
        document.saved_globals = saved;
    }
    document.fit_version();
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
        .map(|(key, _)| format!("Global {}: existing value retained", quoted_name(key)))
        .collect();
    Ok(ImportPreview {
        document,
        collection_name,
        environment_names: environments,
        warnings,
        requires_removal: false,
    })
}

fn quoted_name(text: &str) -> String {
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
        if !folder.scripts.is_empty() {
            collections::set_folder_scripts(db, created.id, &folder.scripts)?;
        }
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
    if !document.collection.scripts.is_empty() {
        collections::set_collection_scripts(db, collection.id, &document.collection.scripts)?;
    }
    if !document.collection.saved_values.is_empty() {
        scopes::write(
            db,
            Scope::Collection(collection.id),
            &document.collection.variables,
            &document.collection.saved_values,
        )?;
    }
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
        let created = environments::create_environment(db, &name, &environment.variables)?;
        if environment.color.is_some() {
            environments::set_environment_color(db, created.id, environment.color)?;
        }
        if !environment.saved_values.is_empty() {
            scopes::write(
                db,
                Scope::Environment(created.id),
                &environment.variables,
                &environment.saved_values,
            )?;
        }
    }
    if !document.globals.is_empty() {
        // Existing globals win, with their saved values; new names bring theirs.
        let (mut globals, mut saved) = scopes::read(db, Scope::Global)?;
        for (key, value) in &document.globals {
            if !globals.contains_key(key) {
                globals.insert(key.clone(), value.clone());
                saved.set(key, document.saved_globals.values(key));
            }
        }
        scopes::write(db, Scope::Global, &globals, &saved)?;
    }
    Ok(collection.id)
}

/// What an import file holds, judged by its content rather than its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Native,
    Postman,
    OpenApi,
}

impl ImportKind {
    /// Only Postman collections keep environments and globals in separate files.
    pub fn takes_companions(self) -> bool {
        self == Self::Postman
    }
}

/// Parse an import file: JSON, or YAML when it is an OpenAPI document (YAML is how most are
/// published). Other YAML is refused with the JSON error, which names the real problem.
fn load(path: &Path) -> Result<(ImportKind, Value), TransferError> {
    let bytes = native::read_document(path)?;
    let data: Value = match serde_json::from_slice(&bytes) {
        Ok(data) => data,
        Err(json) => match openapi::parse_yaml(&bytes) {
            Ok(data) if openapi::is_openapi(&data) => data,
            _ => return Err(json.into()),
        },
    };
    let object = data.as_object().ok_or(TransferError::NotAnObject)?;
    // A format marker means a native document; one with another marker is refused by its
    // decoder, naming the marker, rather than read as Postman.
    let kind = if object.contains_key("format") {
        ImportKind::Native
    } else if openapi::is_openapi(&data) {
        ImportKind::OpenApi
    } else {
        ImportKind::Postman
    };
    Ok((kind, data))
}

/// What a file is, so the UI asks for companions only where they exist. File reading runs
/// off the async threads.
pub async fn detect(path: PathBuf) -> Result<ImportKind, TransferError> {
    tokio::task::spawn_blocking(move || load(&path).map(|(kind, _)| kind))
        .await
        .map_err(|_| TransferError::Task)?
}

/// Read a native, Postman (with optional companions) or OpenAPI file into a document, the
/// conversion warnings and whether losses need consent.
pub fn read(
    path: &Path,
    companions: &[PathBuf],
) -> Result<(NativeDocument, Vec<String>, bool), TransferError> {
    let (kind, data) = load(path)?;
    match kind {
        ImportKind::Native => {
            if !companions.is_empty() {
                return Err(TransferError::NativeCompanions);
            }
            Ok((native::decode_value(data)?, Vec::new(), false))
        }
        ImportKind::OpenApi => {
            if !companions.is_empty() {
                return Err(TransferError::OpenApiCompanions);
            }
            let (document, warnings) = openapi::decode(&data)?;
            Ok((document, warnings, false))
        }
        ImportKind::Postman => {
            let mut decoder = Decoder::default();
            let mut document = decoder.decode(&data)?;
            for companion in companions {
                let companion: Value = serde_json::from_slice(&native::read_document(companion)?)?;
                decoder.companion(&companion, &mut document)?;
            }
            Ok((document, decoder.warnings, decoder.requires_removal))
        }
    }
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

/// Write `bytes` to `path` atomically (a response body the user saves), off the async
/// threads. A failure leaves no partial file.
pub async fn write_file(path: PathBuf, bytes: Vec<u8>) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || native::atomic_write(&path, &bytes))
        .await
        .map_err(std::io::Error::other)?
}

/// Whether `path` names a file that exists, checked off the async threads (a settings
/// path shown as "File not found" while typed).
pub async fn is_file(path: PathBuf) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
}

/// A result `imposter send` wrote, read off the async threads with the import size limit.
pub async fn read_result(path: PathBuf) -> Result<imposter_core::run_result::RunResult, String> {
    tokio::task::spawn_blocking(move || {
        let bytes = native::read_document(&path).map_err(|e| e.to_string())?;
        imposter_core::run_result::RunResult::read(&bytes).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| TransferError::Task.to_string())?
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
