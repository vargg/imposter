//! The native format, `"format": "imposter"`: version 2 holds collections with their
//! requests, checks, captures, variables, saved values, environments and colors; version 3
//! adds scripts. Strict: the marker and version are required, unknown fields are rejected
//! at every level, a document may not use data of a later version, and a document keeps its
//! version when encoded again. New exports take the lowest version that holds them, so one
//! without scripts stays readable by Imposter 0.4.

use std::io::{Read as _, Write as _};
use std::path::Path;

use imposter_core::model::{
    EnvColor, RequestTemplate, SavedValues, Scripts, Variables, is_method_token,
};
use serde::{Deserialize, Deserializer, Serialize};

/// Imports larger than this are rejected before parsing.
pub const MAX_IMPORT_BYTES: u64 = 20 * 1024 * 1024;

/// A name that must not be empty (collections, folders, environments).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Name(String);

impl Name {
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        (!name.is_empty()).then_some(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("name must not be empty"))
    }
}

/// The format marker: a file without it, or with another, is not a native document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Imposter,
}

/// Versions 2 and 3; anything else is rejected rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    V2,
    V3,
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(match self {
            Self::V2 => 2,
            Self::V3 => 3,
        })
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u64::deserialize(deserializer)? {
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            other => Err(serde::de::Error::custom(format!(
                "unsupported version {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferFolder {
    pub name: Name,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub folders: Vec<TransferFolder>,
    #[serde(default)]
    pub requests: Vec<RequestTemplate>,
    /// Version 3: run around every request inside the folder.
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferCollection {
    pub name: Name,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub variables: Variables,
    #[serde(default)]
    pub folders: Vec<TransferFolder>,
    #[serde(default)]
    pub requests: Vec<RequestTemplate>,
    /// Version 2: values saved to swap the collection's variables with.
    #[serde(default, skip_serializing_if = "SavedValues::is_empty")]
    pub saved_values: SavedValues,
    /// Version 3: run around every request in the collection.
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferEnvironment {
    pub name: Name,
    #[serde(default)]
    pub variables: Variables,
    /// Version 2: values saved to swap the environment's variables with.
    #[serde(default, skip_serializing_if = "SavedValues::is_empty")]
    pub saved_values: SavedValues,
    /// Version 2: the color the environment is shown in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<EnvColor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDocument {
    pub format: Format,
    pub version: Version,
    pub collection: TransferCollection,
    #[serde(default)]
    pub environments: Vec<TransferEnvironment>,
    #[serde(default)]
    pub globals: Variables,
    /// Version 2: values saved to swap the globals with.
    #[serde(default, skip_serializing_if = "SavedValues::is_empty")]
    pub saved_globals: SavedValues,
}

impl NativeDocument {
    pub fn new(collection: TransferCollection) -> Self {
        Self {
            format: Format::Imposter,
            version: Version::V2,
            collection,
            environments: Vec::new(),
            globals: Variables::new(),
            saved_globals: SavedValues::default(),
        }
    }

    /// Version 3 when a request has scripts, else 2: the lowest that holds the document, so
    /// older Imposter versions read what they can. For new Imposter documents only.
    pub fn fit_version(&mut self) {
        self.version = if self.scripted().is_some() {
            Version::V3
        } else {
            Version::V2
        };
    }

    /// The first thing with scripts, named for errors: a request, a folder or the
    /// collection.
    pub fn scripted(&self) -> Option<String> {
        fn folder(folders: &[TransferFolder]) -> Option<String> {
            folders.iter().find_map(|f| {
                if f.scripts.is_empty() {
                    folder(&f.folders)
                } else {
                    Some(format!("Folder {:?}", f.name.as_str()))
                }
            })
        }
        if !self.collection.scripts.is_empty() {
            return Some(format!("Collection {:?}", self.collection.name.as_str()));
        }
        self.requests()
            .into_iter()
            .find(|r| !r.scripts.is_empty())
            .map(|r| format!("Request {:?}", r.name))
            .or_else(|| folder(&self.collection.folders))
    }

    /// How many requests, folders and collections have scripts.
    pub fn scripted_count(&self) -> usize {
        fn folders(list: &[TransferFolder]) -> usize {
            list.iter()
                .map(|f| usize::from(!f.scripts.is_empty()) + folders(&f.folders))
                .sum()
        }
        usize::from(!self.collection.scripts.is_empty())
            + folders(&self.collection.folders)
            + self
                .requests()
                .iter()
                .filter(|r| !r.scripts.is_empty())
                .count()
    }

    /// Every request in the document, depth first.
    pub fn requests(&self) -> Vec<&RequestTemplate> {
        fn visit<'a>(folders: &'a [TransferFolder], out: &mut Vec<&'a RequestTemplate>) {
            for folder in folders {
                out.extend(&folder.requests);
                visit(&folder.folders, out);
            }
        }
        let mut out: Vec<&RequestTemplate> = self.collection.requests.iter().collect();
        visit(&self.collection.folders, &mut out);
        out
    }

    /// How many folders the document holds, at any depth.
    pub fn folder_count(&self) -> usize {
        fn count(folders: &[TransferFolder]) -> usize {
            folders.iter().map(|f| 1 + count(&f.folders)).sum()
        }
        count(&self.collection.folders)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("Import exceeds the 20 MiB file limit")]
    TooLarge,
    #[error("Cannot read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Not a valid Imposter collection: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Request {request:?}: method must be a nonempty HTTP token")]
    Method { request: String },
    #[error("Request {request:?}: authentication is missing its configuration")]
    Auth { request: String },
    #[error("{owner}: scripts need format version 3")]
    ScriptsNeedVersion3 { owner: String },
    #[error("Request {request:?}: {source}")]
    Body {
        request: String,
        #[source]
        source: imposter_core::model::FormBodyError,
    },
}

/// Read at most [`MAX_IMPORT_BYTES`], rejecting larger files before decoding.
pub fn read_document(path: &Path) -> Result<Vec<u8>, NativeError> {
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_IMPORT_BYTES {
        return Err(NativeError::TooLarge);
    }
    Ok(data)
}

/// Decode and validate everything an import needs before any write.
pub fn decode(data: &[u8]) -> Result<NativeDocument, NativeError> {
    checked(serde_json::from_slice(data)?)
}

/// [`decode`] for a document already parsed as JSON.
pub fn decode_value(data: serde_json::Value) -> Result<NativeDocument, NativeError> {
    checked(serde_json::from_value(data)?)
}

/// The rule serde cannot express: each field appears only in a version that has it.
fn checked(document: NativeDocument) -> Result<NativeDocument, NativeError> {
    if document.version < Version::V3
        && let Some(owner) = document.scripted()
    {
        return Err(NativeError::ScriptsNeedVersion3 { owner });
    }
    validate_templates(&document)?;
    Ok(document)
}

/// Method tokens, selected auth configuration and structured bodies.
pub fn validate_templates(document: &NativeDocument) -> Result<(), NativeError> {
    for request in document.requests() {
        let label = || request.name.clone();
        if !is_method_token(&request.method) {
            return Err(NativeError::Method { request: label() });
        }
        if request.auth.active().is_err() {
            return Err(NativeError::Auth { request: label() });
        }
        request
            .validate_body()
            .map_err(|source| NativeError::Body {
                request: label(),
                source,
            })?;
    }
    Ok(())
}

/// UTF-8 JSON, two-space indented, newline-terminated, so a file diffs line by line.
///
/// # Panics
/// Never: the document holds only strings, numbers, booleans and string-keyed maps.
pub fn encode(document: &NativeDocument) -> Vec<u8> {
    // Plain data structures with string keys always serialize.
    let mut data = serde_json::to_vec_pretty(document).expect("documents always serialize");
    data.push(b'\n');
    data
}

/// Publish `data` at `path` atomically: write a temporary file in the same directory, flush
/// it to disk, then rename it over the destination. A crash never leaves a partial file.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!(
            ".{}.",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("export")
        ))
        .suffix(".tmp")
        .tempfile_in(directory)?;
    temporary.write_all(data)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `name`, or `name (2)`, `name (3)`, … — the first not in `existing`, which is then reserved.
pub fn unique_name<S: std::hash::BuildHasher>(
    name: &str,
    existing: &mut std::collections::HashSet<String, S>,
) -> String {
    let mut candidate = name.to_owned();
    let mut number = 2;
    while existing.contains(&candidate) {
        candidate = format!("{name} ({number})");
        number += 1;
    }
    existing.insert(candidate.clone());
    candidate
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn folder(name: &str, folders: Vec<TransferFolder>, requests: usize) -> TransferFolder {
        TransferFolder {
            scripts: Scripts::default(),
            name: Name::new(name).unwrap(),
            description: String::new(),
            position: 0,
            folders,
            requests: vec![RequestTemplate::default(); requests],
        }
    }

    #[test]
    fn requests_and_folders_are_counted_at_every_depth() {
        let mut document = NativeDocument::new(TransferCollection {
            scripts: Scripts::default(),
            name: Name::new("API").unwrap(),
            description: String::new(),
            variables: Variables::new(),
            folders: vec![
                folder("Users", vec![folder("Admin", vec![], 2)], 1),
                folder("Empty", vec![], 0),
            ],
            requests: vec![RequestTemplate::default()],
            saved_values: SavedValues::default(),
        });
        assert_eq!(document.requests().len(), 4);
        assert_eq!(document.folder_count(), 3);
        document.collection.folders.clear();
        assert_eq!(document.folder_count(), 0);
    }

    #[test]
    fn unique_names_are_numbered_from_two() {
        let mut existing: HashSet<String> = ["API".into(), "API (2)".into()].into_iter().collect();
        assert_eq!(unique_name("API", &mut existing), "API (3)");
        assert_eq!(unique_name("API", &mut existing), "API (4)");
        assert_eq!(unique_name("New", &mut existing), "New");
        assert_eq!(unique_name("New", &mut existing), "New (2)");
    }

    #[test]
    fn oversized_files_are_rejected_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        assert!(matches!(read_document(&path), Err(NativeError::TooLarge)));
        file.set_len(MAX_IMPORT_BYTES).unwrap();
        assert_eq!(read_document(&path).unwrap().len() as u64, MAX_IMPORT_BYTES);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.imposter.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["out.imposter.json"]);
    }

    #[test]
    fn atomic_write_into_a_missing_directory_fails_without_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(atomic_write(&dir.path().join("missing/out.json"), b"x").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
