//! A collections folder read straight from disk, without any profile: what `--project DIR`
//! runs, for CI and a fresh checkout. Each collection becomes a native document carrying
//! every environment of the folder, so `document::select` and the catalog work on it as on
//! an exported file.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use imposter_core::check::Check;
use imposter_core::model::{RequestTemplate, Variables};
use imposter_core::run_result::Source;
use imposter_store::files::format::{self, Folder, Project};
use imposter_store::files::workspace;
use imposter_transfer::native::{
    Name, NativeDocument, TransferCollection, TransferEnvironment, TransferFolder,
};

use crate::catalog::{self, Listed, RequestView};
use crate::document::{self, DocumentError, RequestPath, Selected};

/// A folder as read: one document per collection, and the files they came from, whose
/// bytes are the revision that ran.
#[derive(Debug, Clone)]
pub struct ProjectFolder {
    pub documents: Vec<NativeDocument>,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// An agent's view of one snapshot. Targets refer to file identities, including when
/// two siblings have the same display name.
pub struct AgentProject {
    pub folder: ProjectFolder,
    model: Project,
    paths: HashMap<String, String>,
    targets: BTreeMap<String, Target>,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Collection,
    Folder,
    Request,
}

pub struct Target {
    pub path: RequestPath,
    pub kind: TargetKind,
    pub request: Option<Listed>,
    collection: usize,
    folders: Vec<usize>,
    index: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEdit {
    pub method: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub checks: Option<Vec<Check>>,
}

pub struct SavedEdit {
    pub path: String,
    pub before: RequestTemplate,
    pub after: RequestTemplate,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("Cannot read the collections folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Format(#[from] format::FormatError),
    #[error("The collections folder {0} holds no collection")]
    Empty(String),
    #[error("No target has that stable ID")]
    NoTarget,
    #[error("This operation needs a request ID")]
    NotRequest,
    #[error("The project changed since that revision; nothing was saved")]
    Stale,
    #[error("Request method must be a nonempty HTTP token")]
    InvalidMethod,
    #[error(transparent)]
    Selection(#[from] DocumentError),
}

/// Read every collection and environment under `dir`.
pub fn read(dir: &Path) -> Result<ProjectFolder, ProjectError> {
    let files = format::read_dir(dir)?;
    let project = format::read(&files)?;
    if project.collections.is_empty() {
        return Err(ProjectError::Empty(dir.display().to_string()));
    }
    let mut documents = documents(&project);
    // By name, as the profile lists its collections.
    documents.sort_by(|a, b| a.collection.name.as_str().cmp(b.collection.name.as_str()));
    Ok(ProjectFolder { documents, files })
}

fn name(text: &str) -> Name {
    // The format refuses empty names, so this never needs its fallback.
    Name::new(text).unwrap_or_else(|| Name::new("Unnamed").expect("a fixed name is not empty"))
}

fn folder(folder: &Folder) -> TransferFolder {
    TransferFolder {
        name: name(&folder.name),
        description: folder.description.clone(),
        position: folder.position,
        folders: folder.folders.iter().map(self::folder).collect(),
        requests: folder.requests.iter().map(|r| r.template.clone()).collect(),
        scripts: folder.scripts.clone(),
    }
}

fn documents(project: &Project) -> Vec<NativeDocument> {
    let environments: Vec<TransferEnvironment> = project
        .environments
        .iter()
        .map(|e| TransferEnvironment {
            name: name(&e.name),
            variables: e.variables.clone(),
            saved_values: e.saved_values.clone(),
            color: e.color,
        })
        .collect();
    project
        .collections
        .iter()
        .map(|c| {
            let mut document = NativeDocument::new(TransferCollection {
                name: name(&c.name),
                description: c.description.clone(),
                variables: c.variables.clone(),
                folders: c.folders.iter().map(folder).collect(),
                requests: c.requests.iter().map(|r| r.template.clone()).collect(),
                saved_values: c.saved_values.clone(),
                scripts: c.scripts.clone(),
            });
            document.environments.clone_from(&environments);
            // Shared globals only: names whose values stay on someone's computer are left
            // out, so a request using one says it is unresolved rather than send it empty.
            if let Some(globals) = project.globals.as_ref().filter(|g| g.share_values) {
                document.globals.clone_from(&globals.variables);
                document.saved_globals.clone_from(&globals.saved_values);
            }
            document.fit_version();
            document
        })
        .collect()
}

impl AgentProject {
    pub fn read(dir: &Path) -> Result<Self, ProjectError> {
        let files = format::read_agent_dir(dir)?;
        let (mut model, paths) = format::read_owned(&files)?;
        if model.collections.is_empty() {
            return Err(ProjectError::Empty(dir.display().to_string()));
        }
        model.collections.sort_by(|a, b| a.name.cmp(&b.name));
        let documents = documents(&model);
        let mut targets = BTreeMap::new();
        for (collection, item) in model.collections.iter().enumerate() {
            let path = RequestPath::new(&[], &item.name);
            targets.insert(
                item.id.clone(),
                Target {
                    path: path.clone(),
                    kind: TargetKind::Collection,
                    request: None,
                    collection,
                    folders: Vec::new(),
                    index: None,
                },
            );
            index_targets(
                &mut targets,
                collection,
                &[],
                &path,
                &item.folders,
                &item.requests,
                !item.scripts.is_empty(),
            );
        }
        Ok(Self {
            folder: ProjectFolder { documents, files },
            model,
            paths,
            targets,
        })
    }

    pub fn targets(&self) -> &BTreeMap<String, Target> {
        &self.targets
    }

    pub fn select(
        &self,
        id: &str,
        environment: Option<&str>,
        values: &Variables,
    ) -> Result<Vec<Selected>, ProjectError> {
        let target = self.targets.get(id).ok_or(ProjectError::NoTarget)?;
        Ok(document::select_location(
            &self.folder.documents[target.collection],
            &target.folders,
            target.index,
            environment,
            values,
        )?)
    }

    pub fn inspect(
        &self,
        id: &str,
        environment: Option<&str>,
        redactor: &mut imposter_core::redact::Redactor,
    ) -> Result<RequestView, ProjectError> {
        let target = self.targets.get(id).ok_or(ProjectError::NoTarget)?;
        if target.index.is_none() {
            return Err(ProjectError::NotRequest);
        }
        let [selected] = self
            .select(id, environment, &Variables::new())?
            .try_into()
            .map_err(|_| ProjectError::NotRequest)?;
        redactor.definition_credentials(&selected.request.template);
        Ok(catalog::view(
            Source::Project,
            &selected.path,
            environment,
            &selected.request.template,
            &selected.inputs,
            usize::MAX,
        ))
    }

    /// Preserve the path on disk and edit only the requested definition. The desktop's
    /// file watcher applies the existing guard if this meets an unsaved draft.
    pub fn save(
        mut self,
        root: &Path,
        id: &str,
        edit: RequestEdit,
    ) -> Result<SavedEdit, ProjectError> {
        let target = self.targets.get(id).ok_or(ProjectError::NoTarget)?;
        let index = target.index.ok_or(ProjectError::NotRequest)?;
        let path = self.paths.remove(id).ok_or(ProjectError::NoTarget)?;
        let collection = &mut self.model.collections[target.collection];
        let (mut folders, mut requests) = (&mut collection.folders, &mut collection.requests);
        for &index in &target.folders {
            let folder = &mut folders[index];
            (folders, requests) = (&mut folder.folders, &mut folder.requests);
        }
        let request = &mut requests[index].template;
        let before = request.clone();
        if let Some(method) = edit.method {
            if !imposter_core::model::is_method_token(&method) {
                return Err(ProjectError::InvalidMethod);
            }
            request.method = method;
        }
        if let Some(url) = edit.url {
            request.url = url;
        }
        if let Some(description) = edit.description {
            request.description = description;
        }
        if let Some(checks) = edit.checks {
            request.checks = checks;
        }
        let after = request.clone();
        let mut files = self.folder.files;
        let _lock = workspace::lock_project(root)?;
        if files != format::read_agent_dir(root)? {
            return Err(ProjectError::Stale);
        }
        if before != after {
            let (_, bytes) = format::render_owned(&self.model)
                .remove(id)
                .ok_or(ProjectError::NoTarget)?;
            workspace::atomic_write(&root.join(&path), &bytes)?;
            files.insert(path.clone(), bytes);
        }
        Ok(SavedEdit {
            path,
            before,
            after,
            files,
        })
    }
}

fn index_targets(
    targets: &mut BTreeMap<String, Target>,
    collection: usize,
    location: &[usize],
    parent: &RequestPath,
    folders: &[Folder],
    requests: &[format::Request],
    inherited: bool,
) {
    let parts: Vec<&str> = parent.parts().iter().map(String::as_str).collect();
    for (index, request) in requests.iter().enumerate() {
        let path = RequestPath::new(&parts, &request.template.name);
        let mut listing = catalog::listed(&path, &request.template, inherited);
        let mut redactor = imposter_core::redact::Redactor::default();
        redactor.definition_credentials(&request.template);
        listing.redact(&redactor);
        targets.insert(
            request.id.clone(),
            Target {
                request: Some(listing),
                path,
                kind: TargetKind::Request,
                collection,
                folders: location.to_vec(),
                index: Some(index),
            },
        );
    }
    for (index, folder) in folders.iter().enumerate() {
        let path = RequestPath::new(&parts, &folder.name);
        let mut location = location.to_vec();
        location.push(index);
        index_targets(
            targets,
            collection,
            &location,
            &path,
            &folder.folders,
            &folder.requests,
            inherited || !folder.scripts.is_empty(),
        );
        targets.insert(
            folder.id.clone(),
            Target {
                path,
                kind: TargetKind::Folder,
                request: None,
                collection,
                folders: location,
                index: None,
            },
        );
    }
}

impl ProjectFolder {
    /// The collection a path names: by its first part, or the only one when the folder
    /// holds a single collection and the path leaves its name out.
    pub fn document(&self, path: &RequestPath) -> Result<&NativeDocument, DocumentError> {
        let first = path.parts().first().map(String::as_str).unwrap_or_default();
        let named: Vec<&NativeDocument> = self
            .documents
            .iter()
            .filter(|d| d.collection.name.as_str().trim() == first)
            .collect();
        match (named.as_slice(), self.documents.as_slice()) {
            ([one], _) => Ok(one),
            ([], [one]) => Ok(one),
            ([], _) => Err(DocumentError::NoTarget {
                path: path.to_string(),
                available: self.names(),
            }),
            (many, _) => Err(DocumentError::Ambiguous {
                path: path.to_string(),
                count: many.len(),
            }),
        }
    }

    /// The collections' names, in the folder's order.
    pub fn names(&self) -> Vec<String> {
        self.documents
            .iter()
            .map(|d| d.collection.name.as_str().to_owned())
            .collect()
    }
}
