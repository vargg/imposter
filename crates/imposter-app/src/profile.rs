//! Running a request saved in the personal profile from outside the app: the command line
//! names it by its breadcrumb (`Shop API / Users / Create user`) and gets the same inputs
//! Send would read, with three differences the user chose. The environment is the one named,
//! never the app's active one; a red (production) environment needs the app's
//! `allow_production_from_cli` setting; and captures and script variables stay in the run.

use imposter_core::model::{EnvColor, Variables};
use imposter_store::{StoreError, StoreHandle, collections, environments, kv};

use crate::document::{DocumentError, RequestPath, Selected};
use crate::executor::{EnvironmentChoice, Writes, read_inputs};

/// Why a request is chosen: running one may refuse production, inspecting one never does,
/// since it reveals only names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Run,
    Inspect,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error(transparent)]
    Path(#[from] DocumentError),
    #[error(
        "{name} is a production (red) environment. To use it from the command line, turn on \
         \"Allow production environments\" in the app's Settings → Command line"
    )]
    Production { name: String },
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn path_of(request: &collections::IndexedRequest) -> RequestPath {
    let mut place: Vec<&str> = vec![request.collection.as_str()];
    place.extend(request.folders.iter().map(String::as_str));
    RequestPath::new(&place, &request.name)
}

fn paths(db: &imposter_store::Tx<'_>) -> Result<Vec<String>, StoreError> {
    Ok(collections::request_index(db)?
        .iter()
        .map(|request| path_of(request).to_string())
        .collect())
}

/// Every saved request's path, collection first, as the command line names them.
pub async fn request_paths(store: &StoreHandle) -> Result<Vec<String>, StoreError> {
    store.call(paths).await
}

/// Choose the saved request at `path` and read its inputs in one database command, so the
/// run sees one consistent snapshot. `values` go above every scope.
pub async fn select(
    store: &StoreHandle,
    path: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
    purpose: Purpose,
) -> Result<Selected, ProfileError> {
    let path = path.clone();
    let environment = environment.map(str::to_owned);
    let values = values.clone();
    store
        .call(move |db| {
            Ok(select_in(
                db,
                &path,
                environment.as_deref(),
                &values,
                purpose,
            ))
        })
        .await?
}

fn select_in(
    db: &imposter_store::Tx<'_>,
    path: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
    purpose: Purpose,
) -> Result<Selected, ProfileError> {
    let index = collections::request_index(db)?;
    let matches: Vec<&collections::IndexedRequest> = index
        .iter()
        .filter(|request| {
            let names: Vec<String> = path_of(request).parts().to_vec();
            path.is(&names.iter().map(String::as_str).collect::<Vec<_>>())
        })
        .collect();
    let id = match matches.as_slice() {
        [one] => one.id,
        [] => {
            return Err(DocumentError::NoRequest {
                path: path.to_string(),
                available: paths(db)?,
            }
            .into());
        }
        several => {
            return Err(DocumentError::Ambiguous {
                path: path.to_string(),
                count: several.len(),
            }
            .into());
        }
    };
    let request = collections::request(db, id)?.ok_or_else(|| DocumentError::NoRequest {
        path: path.to_string(),
        available: Vec::new(),
    })?;
    let chosen = chosen_environment(db, environment, purpose)?;
    let mut inputs = read_inputs(db, &request, EnvironmentChoice::Chosen(chosen))?;
    inputs.writes = Writes::Run;
    inputs.snapshot = inputs.snapshot.with_inputs(values);
    Ok(Selected {
        path: path.clone(),
        request,
        inputs,
    })
}

/// The environment named (none: globals and collection only), refused when it is red and
/// the run would send without the app's permission.
fn chosen_environment(
    db: &imposter_store::Tx<'_>,
    environment: Option<&str>,
    purpose: Purpose,
) -> Result<Option<imposter_core::ids::EnvironmentId>, ProfileError> {
    let Some(name) = environment else {
        return Ok(None);
    };
    let all = environments::environments(db)?;
    let Some(found) = all.iter().find(|e| e.name == name) else {
        return Err(DocumentError::NoEnvironment {
            name: name.to_owned(),
            available: all.iter().map(|e| e.name.clone()).collect(),
        }
        .into());
    };
    if purpose == Purpose::Run
        && found.color == Some(EnvColor::Red)
        && !kv::app_settings(db)?.allow_production_from_cli
    {
        return Err(ProfileError::Production {
            name: found.name.clone(),
        });
    }
    Ok(Some(found.id))
}

/// Every saved request under `target` (a collection, a folder or one request), in the tree's
/// order: siblings by position, folders and requests mixed, as the sidebar shows them. All
/// are read in one database command, so the run is frozen at its start.
pub async fn select_all(
    store: &StoreHandle,
    target: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
) -> Result<Vec<Selected>, ProfileError> {
    let target = target.clone();
    let environment = environment.map(str::to_owned);
    let values = values.clone();
    store
        .call(move |db| Ok(select_all_in(db, &target, environment.as_deref(), &values)))
        .await?
}

/// The collection and every folder, as a run's target can name them.
fn targets(db: &imposter_store::Tx<'_>) -> Result<Vec<String>, StoreError> {
    let mut out = Vec::new();
    for collection in collections::collections(db)? {
        let Some(tree) = collections::tree(db, collection.id)? else {
            continue;
        };
        out.push(RequestPath::new(&[], &collection.name).to_string());
        for folder in &tree.folders {
            let mut place = vec![collection.name.clone()];
            place.extend(folder_names(&tree, folder.parent_id));
            let names: Vec<&str> = place.iter().map(String::as_str).collect();
            out.push(RequestPath::new(&names, &folder.name).to_string());
        }
    }
    Ok(out)
}

/// The names of `folder` and its parents, outermost first.
fn folder_names(
    tree: &collections::Tree,
    mut folder: Option<imposter_core::ids::FolderId>,
) -> Vec<String> {
    let mut names = Vec::new();
    while let Some(found) = folder.and_then(|id| tree.folders.iter().find(|f| f.id == id)) {
        // Parents form a tree (moves refuse cycles); the bound only guards a corrupt file.
        if names.len() > tree.folders.len() {
            break;
        }
        names.push(found.name.clone());
        folder = found.parent_id;
    }
    names.reverse();
    names
}

/// The requests under `folder` (the collection's root for `None`), depth first, siblings by
/// position then id.
fn ordered(
    tree: &collections::Tree,
    folder: Option<imposter_core::ids::FolderId>,
    out: &mut Vec<imposter_core::ids::RequestId>,
) {
    enum Node {
        Request(imposter_core::ids::RequestId),
        Folder(imposter_core::ids::FolderId),
    }
    let mut nodes: Vec<(i64, i64, Node)> = tree
        .requests
        .iter()
        .filter(|r| r.folder_id == folder)
        .map(|r| (r.position, r.id.0, Node::Request(r.id)))
        .chain(
            tree.folders
                .iter()
                .filter(|f| f.parent_id == folder)
                .map(|f| (f.position, f.id.0, Node::Folder(f.id))),
        )
        .collect();
    nodes.sort_by_key(|(position, id, _)| (*position, *id));
    for (_, _, node) in nodes {
        match node {
            Node::Request(id) => out.push(id),
            Node::Folder(id) => ordered(tree, Some(id), out),
        }
    }
}

fn select_all_in(
    db: &imposter_store::Tx<'_>,
    target: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
) -> Result<Vec<Selected>, ProfileError> {
    let no_target = |db: &imposter_store::Tx<'_>| -> Result<ProfileError, StoreError> {
        Ok(DocumentError::NoTarget {
            path: target.to_string(),
            available: targets(db)?,
        }
        .into())
    };
    let parts = target.parts();
    let collections: Vec<_> = collections::collections(db)?
        .into_iter()
        .filter(|c| c.name.trim() == parts[0])
        .collect();
    let collection = match collections.as_slice() {
        [one] => one,
        [] => return Err(no_target(db)?),
        several => {
            return Err(DocumentError::Ambiguous {
                path: target.to_string(),
                count: several.len(),
            }
            .into());
        }
    };
    let Some(tree) = collections::tree(db, collection.id)? else {
        return Err(no_target(db)?);
    };
    let mut folder = None;
    let mut ids = Vec::new();
    let mut single = false;
    for (index, part) in parts.iter().enumerate().skip(1) {
        let folders: Vec<_> = tree
            .folders
            .iter()
            .filter(|f| f.parent_id == folder && f.name.trim() == part)
            .collect();
        let requests: Vec<_> = if index + 1 == parts.len() {
            tree.requests
                .iter()
                .filter(|r| r.folder_id == folder && r.name.trim() == part)
                .collect()
        } else {
            Vec::new()
        };
        match (folders.as_slice(), requests.as_slice()) {
            ([one], []) => folder = Some(one.id),
            ([], [one]) => {
                ids.push(one.id);
                single = true;
            }
            ([], []) => return Err(no_target(db)?),
            (several, others) => {
                return Err(DocumentError::Ambiguous {
                    path: target.to_string(),
                    count: several.len() + others.len(),
                }
                .into());
            }
        }
    }
    if !single {
        ordered(&tree, folder, &mut ids);
    }
    let chosen = chosen_environment(db, environment, Purpose::Run)?;
    let mut selected = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(request) = collections::request(db, id)? else {
            continue;
        };
        let mut place = vec![collection.name.clone()];
        place.extend(folder_names(&tree, request.folder_id));
        let names: Vec<&str> = place.iter().map(String::as_str).collect();
        let path = RequestPath::new(&names, &request.template.name);
        let mut inputs = read_inputs(db, &request, EnvironmentChoice::Chosen(chosen))?;
        inputs.writes = Writes::Run;
        inputs.snapshot = inputs.snapshot.with_inputs(values);
        selected.push(Selected {
            path,
            request,
            inputs,
        });
    }
    Ok(selected)
}
