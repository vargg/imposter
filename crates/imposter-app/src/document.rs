//! Running a request straight from a collection file: the request is chosen by its path of
//! names, and its inputs come from the file and the values given for the run, never from
//! the database. The run itself is `executor::run`, the same as Send.

use imposter_core::model::{Request, RequestTemplate, Scripts, Variables};
use imposter_core::settings::AppSettings;
use imposter_core::variables::{ActiveEnvironment, CollectionScope, Snapshot, VariableContext};
use imposter_transfer::native::{NativeDocument, TransferFolder};

use crate::executor::{Inputs, Origin};
use crate::scripts::{Level, ScriptChain};

/// A request in a file, named by its folders and its own name: `Users/Admin/Create user`.
/// A `/` inside a name is written `\/`, and a `\` as `\\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPath(Vec<String>);

impl RequestPath {
    pub fn parse(text: &str) -> Result<Self, DocumentError> {
        let mut parts = vec![String::new()];
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(escaped @ ('/' | '\\')) => push(&mut parts, escaped),
                    _ => return Err(DocumentError::BadPath(text.to_owned())),
                },
                '/' => parts.push(String::new()),
                other => push(&mut parts, other),
            }
        }
        if parts.iter().any(String::is_empty) {
            return Err(DocumentError::BadPath(text.to_owned()));
        }
        Ok(Self(parts))
    }

    fn of(folders: &[&str], name: &str) -> Self {
        Self(
            folders
                .iter()
                .copied()
                .chain([name])
                .map(str::to_owned)
                .collect(),
        )
    }
}

fn push(parts: &mut [String], c: char) {
    if let Some(last) = parts.last_mut() {
        last.push(c);
    }
}

impl std::fmt::Display for RequestPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let escaped: Vec<String> = self
            .0
            .iter()
            .map(|part| part.replace('\\', "\\\\").replace('/', "\\/"))
            .collect();
        f.write_str(&escaped.join("/"))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DocumentError {
    #[error("{0:?} is not a request path: names are separated by \"/\", and none is empty")]
    BadPath(String),
    #[error("No request at {path}. Requests in this file: {}", list(.available))]
    NoRequest {
        path: String,
        available: Vec<String>,
    },
    #[error("{count} requests are at {path}; rename one so its path names one request")]
    Ambiguous { path: String, count: usize },
    #[error("No environment named {name:?}. Environments in this file: {}", list(.available))]
    NoEnvironment {
        name: String,
        available: Vec<String>,
    },
    #[error("{count} environments are named {name:?}; rename one")]
    AmbiguousEnvironment { name: String, count: usize },
}

fn list(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// A request found in a file, and the folders around it, outermost first.
struct Found<'a> {
    folders: Vec<&'a TransferFolder>,
    template: &'a RequestTemplate,
}

fn walk<'a>(
    folders: &mut Vec<&'a TransferFolder>,
    within: &'a [TransferFolder],
    requests: &'a [RequestTemplate],
    found: &mut Vec<Found<'a>>,
) {
    for template in requests {
        found.push(Found {
            folders: folders.clone(),
            template,
        });
    }
    for folder in within {
        folders.push(folder);
        walk(folders, &folder.folders, &folder.requests, found);
        folders.pop();
    }
}

fn all(document: &NativeDocument) -> Vec<(RequestPath, Found<'_>)> {
    let mut found = Vec::new();
    walk(
        &mut Vec::new(),
        &document.collection.folders,
        &document.collection.requests,
        &mut found,
    );
    found
        .into_iter()
        .map(|f| {
            let names: Vec<&str> = f
                .folders
                .iter()
                .map(|folder| folder.name.as_str())
                .collect();
            (RequestPath::of(&names, &f.template.name), f)
        })
        .collect()
}

/// Every request's path, in the file's order.
pub fn request_paths(document: &NativeDocument) -> Vec<String> {
    all(document)
        .into_iter()
        .map(|(path, _)| path.to_string())
        .collect()
}

/// A request chosen from a file, with everything running it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    pub path: RequestPath,
    pub request: Request,
    pub inputs: Inputs,
}

/// Choose the request at `path` and read its inputs: the file's globals, collection
/// variables and the named `environment` (none: globals and collection only), with
/// `values` above them all. Settings are the application defaults; the request's own
/// options still apply.
pub fn select(
    document: &NativeDocument,
    path: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
) -> Result<Selected, DocumentError> {
    let mut matches: Vec<Found<'_>> = all(document)
        .into_iter()
        .filter(|(candidate, _)| candidate == path)
        .map(|(_, found)| found)
        .collect();
    let found = match matches.len() {
        0 => {
            return Err(DocumentError::NoRequest {
                path: path.to_string(),
                available: request_paths(document),
            });
        }
        1 => matches.remove(0),
        count => {
            return Err(DocumentError::Ambiguous {
                path: path.to_string(),
                count,
            });
        }
    };
    let environment = match environment {
        None => None,
        Some(name) => {
            let named: Vec<_> = document
                .environments
                .iter()
                .filter(|e| e.name.as_str() == name)
                .collect();
            match named.as_slice() {
                [one] => Some(ActiveEnvironment {
                    id: None,
                    name: one.name.as_str().to_owned(),
                    values: one.variables.clone(),
                    saved: one.saved_values.clone(),
                }),
                [] => {
                    return Err(DocumentError::NoEnvironment {
                        name: name.to_owned(),
                        available: document
                            .environments
                            .iter()
                            .map(|e| e.name.as_str().to_owned())
                            .collect(),
                    });
                }
                several => {
                    return Err(DocumentError::AmbiguousEnvironment {
                        name: name.to_owned(),
                        count: several.len(),
                    });
                }
            }
        }
    };
    let context = VariableContext {
        globals: document.globals.clone(),
        global_saved: document.saved_globals.clone(),
        environment,
    };
    let collection = &document.collection;
    let snapshot = Snapshot::build(
        &context,
        Some(CollectionScope::Live {
            id: None,
            name: collection.name.as_str(),
            values: &collection.variables,
            saved: &collection.saved_values,
        }),
    )
    .with_inputs(values);
    let mut levels = vec![level(
        format!("Collection \"{}\"", collection.name.as_str()),
        &collection.scripts,
    )];
    levels.extend(found.folders.iter().map(|folder| {
        level(
            format!("Folder \"{}\"", folder.name.as_str()),
            &folder.scripts,
        )
    }));
    Ok(Selected {
        path: path.clone(),
        request: Request {
            template: found.template.clone(),
            ..Request::default()
        },
        inputs: Inputs {
            settings: AppSettings::default(),
            context,
            snapshot,
            scripts: ScriptChain(levels),
            origin: Origin::File,
        },
    })
}

fn level(name: String, scripts: &Scripts) -> Level {
    Level {
        name,
        scripts: scripts.clone(),
    }
}
