//! The collections folder in memory, as flat rows like the tables it replaced: the store's
//! functions read and change these, and [`Model::to_project`] gives the files.

use imposter_core::model::{EnvColor, RequestTemplate, SavedValues, Scripts, Variables};

use super::format;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CollectionRow {
    pub uid: String,
    pub name: String,
    pub description: String,
    /// As in its file: the shared values.
    pub variables: Variables,
    pub saved_values: SavedValues,
    pub scripts: Scripts,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FolderRow {
    pub uid: String,
    pub collection: String,
    pub parent: Option<String>,
    pub name: String,
    pub description: String,
    pub position: i64,
    pub scripts: Scripts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestRow {
    pub uid: String,
    pub collection: String,
    pub folder: Option<String>,
    pub template: RequestTemplate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentRow {
    pub uid: String,
    pub name: String,
    pub color: Option<EnvColor>,
    pub share_values: bool,
    /// As in its file: the shared values, or names with empty values when not shared.
    pub variables: Variables,
    pub saved_values: SavedValues,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub collections: Vec<CollectionRow>,
    pub folders: Vec<FolderRow>,
    pub requests: Vec<RequestRow>,
    pub environments: Vec<EnvironmentRow>,
    /// As in `globals.yaml`: the shared values, or names when not shared; `None` without
    /// the file.
    pub globals: Option<format::Globals>,
}

impl Model {
    pub fn from_project(project: format::Project) -> Self {
        let mut model = Self::default();
        for collection in project.collections {
            let uid = collection.id.clone();
            model.add_level(&uid, None, collection.folders, collection.requests);
            model.collections.push(CollectionRow {
                uid,
                name: collection.name,
                description: collection.description,
                variables: collection.variables,
                saved_values: collection.saved_values,
                scripts: collection.scripts,
            });
        }
        model.globals = project.globals;
        model.environments = project
            .environments
            .into_iter()
            .map(|e| EnvironmentRow {
                uid: e.id,
                name: e.name,
                color: e.color,
                share_values: e.share_values,
                variables: e.variables,
                saved_values: e.saved_values,
            })
            .collect();
        model
    }

    fn add_level(
        &mut self,
        collection: &str,
        parent: Option<&str>,
        folders: Vec<format::Folder>,
        requests: Vec<format::Request>,
    ) {
        for request in requests {
            self.requests.push(RequestRow {
                uid: request.id,
                collection: collection.to_owned(),
                folder: parent.map(str::to_owned),
                template: request.template,
            });
        }
        for folder in folders {
            let uid = folder.id.clone();
            self.add_level(collection, Some(&uid), folder.folders, folder.requests);
            self.folders.push(FolderRow {
                uid,
                collection: collection.to_owned(),
                parent: parent.map(str::to_owned),
                name: folder.name,
                description: folder.description,
                position: folder.position,
                scripts: folder.scripts,
            });
        }
    }

    pub fn to_project(&self) -> format::Project {
        format::Project {
            collections: self
                .collections
                .iter()
                .map(|c| format::Collection {
                    id: c.uid.clone(),
                    name: c.name.clone(),
                    description: c.description.clone(),
                    variables: c.variables.clone(),
                    saved_values: c.saved_values.clone(),
                    scripts: c.scripts.clone(),
                    folders: self.folders_under(&c.uid, None),
                    requests: self.requests_under(&c.uid, None),
                })
                .collect(),
            environments: self
                .environments
                .iter()
                .map(|e| format::Environment {
                    id: e.uid.clone(),
                    name: e.name.clone(),
                    color: e.color,
                    share_values: e.share_values,
                    variables: e.variables.clone(),
                    saved_values: e.saved_values.clone(),
                })
                .collect(),
            globals: self.globals.clone(),
        }
    }

    fn folders_under(&self, collection: &str, parent: Option<&str>) -> Vec<format::Folder> {
        self.folders
            .iter()
            .filter(|f| f.collection == collection && f.parent.as_deref() == parent)
            .map(|f| format::Folder {
                id: f.uid.clone(),
                name: f.name.clone(),
                position: f.position,
                description: f.description.clone(),
                scripts: f.scripts.clone(),
                folders: self.folders_under(collection, Some(&f.uid)),
                requests: self.requests_under(collection, Some(&f.uid)),
            })
            .collect()
    }

    fn requests_under(&self, collection: &str, folder: Option<&str>) -> Vec<format::Request> {
        self.requests
            .iter()
            .filter(|r| r.collection == collection && r.folder.as_deref() == folder)
            .map(|r| format::Request {
                id: r.uid.clone(),
                template: r.template.clone(),
            })
            .collect()
    }

    /// `folder` and every folder inside it.
    pub fn folder_and_descendants(&self, folder: &str) -> Vec<String> {
        let mut out = vec![folder.to_owned()];
        let mut index = 0;
        while index < out.len() {
            let parent = out[index].clone();
            out.extend(
                self.folders
                    .iter()
                    .filter(|f| f.parent.as_deref() == Some(parent.as_str()))
                    .map(|f| f.uid.clone()),
            );
            index += 1;
        }
        out
    }
}
