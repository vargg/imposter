//! A collections folder read straight from disk, without any profile: what `--project DIR`
//! runs, for CI and a fresh checkout. Each collection becomes a native document carrying
//! every environment of the folder, so `document::select` and the catalog work on it as on
//! an exported file.

use std::collections::BTreeMap;
use std::path::Path;

use imposter_store::files::format::{self, Folder, Project};
use imposter_transfer::native::{
    Name, NativeDocument, TransferCollection, TransferEnvironment, TransferFolder,
};

use crate::document::{DocumentError, RequestPath};

/// A folder as read: one document per collection, and the files they came from, whose
/// bytes are the revision that ran.
#[derive(Debug, Clone)]
pub struct ProjectFolder {
    pub documents: Vec<NativeDocument>,
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
            document.fit_version();
            document
        })
        .collect()
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
