//! The native format. Version 1 (`"format": "poster"`) is the full-fidelity contract shared
//! with the Python app; version 2 (`"format": "imposter"`) adds request checks, captures and
//! saved variable values. Strict:
//! unknown fields are rejected at every level, a version 1 document may not use version 2
//! data, and a document keeps its version when encoded again.

use std::io::{Read as _, Write as _};
use std::path::Path;

use imposter_core::model::{RequestTemplate, SavedValues, Variables, is_method_token};
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

/// The format marker. `poster` (version 1) is the contract shared with the Python app;
/// `imposter` (version 2) adds what only Imposter has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Poster,
    Imposter,
}

/// Versions 1 and 2; anything else is rejected rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Version {
    #[default]
    V1,
    V2,
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
        })
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u64::deserialize(deserializer)? {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDocument {
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
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
    #[error("A {format:?} document must be version {expected}")]
    Marker { format: &'static str, expected: u8 },
    #[error("Request {request:?}: checks and captures need format version 2")]
    ChecksInVersion1 { request: String },
    #[error("Saved values need format version 2")]
    SavedValuesInVersion1,
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
    let document: NativeDocument = serde_json::from_slice(data)?;
    match (document.format, document.version) {
        (Format::Poster, Version::V1) | (Format::Imposter, Version::V2) => {}
        (Format::Poster, Version::V2) => {
            return Err(NativeError::Marker {
                format: "poster",
                expected: 1,
            });
        }
        (Format::Imposter, Version::V1) => {
            return Err(NativeError::Marker {
                format: "imposter",
                expected: 2,
            });
        }
    }
    if document.version == Version::V1 {
        if let Some(request) = document
            .requests()
            .into_iter()
            .find(|r| !r.checks.is_empty() || !r.captures.is_empty())
        {
            return Err(NativeError::ChecksInVersion1 {
                request: request.name.clone(),
            });
        }
        if !document.collection.saved_values.is_empty()
            || !document.saved_globals.is_empty()
            || document
                .environments
                .iter()
                .any(|e| !e.saved_values.is_empty())
        {
            return Err(NativeError::SavedValuesInVersion1);
        }
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

/// UTF-8 JSON, two-space indented, newline-terminated, as the Python app writes it.
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
        let path = dir.path().join("out.poster.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["out.poster.json"]);
    }

    #[test]
    fn atomic_write_into_a_missing_directory_fails_without_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(atomic_write(&dir.path().join("missing/out.json"), b"x").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
