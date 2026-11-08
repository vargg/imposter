//! The collections folder as the database thread holds it: the model, the bytes each file
//! had when read or last written, and the numeric ids the rest of the app uses.
//!
//! A command's changes are staged on a copy and written when it commits: only files whose
//! bytes changed, each atomically, and only if nobody changed them on disk since they
//! were read. A refused write rolls the whole command back, SQLite included, so the
//! database never gets ahead of the files.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use super::format;
use super::model::Model;
use crate::StoreError;

/// What an id names, so ids of different kinds never mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Collection,
    Folder,
    Request,
    Environment,
}

impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::Folder => "folder",
            Self::Request => "request",
            Self::Environment => "environment",
        }
    }
}

/// The numeric id of each file's `id`, kept in the profile (`ids`), so a request keeps its
/// id across restarts and History's links hold.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    by_uid: HashMap<String, (Kind, i64)>,
    by_id: HashMap<(Kind, i64), String>,
}

impl Ids {
    pub fn load(db: &Connection) -> Result<Self, StoreError> {
        let mut ids = Self::default();
        let mut statement = db.prepare("SELECT uid, kind, id FROM ids")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (uid, kind, id) = row?;
            let kind = match kind.as_str() {
                "collection" => Kind::Collection,
                "folder" => Kind::Folder,
                "request" => Kind::Request,
                _ => Kind::Environment,
            };
            ids.by_uid.insert(uid.clone(), (kind, id));
            ids.by_id.insert((kind, id), uid);
        }
        Ok(ids)
    }

    pub fn uid(&self, kind: Kind, id: i64) -> Option<&str> {
        self.by_id.get(&(kind, id)).map(String::as_str)
    }

    pub fn id(&self, uid: &str) -> Option<i64> {
        self.by_uid.get(uid).map(|(_, id)| *id)
    }

    /// The id of `uid`, given one now if it has none: the next after every id of its
    /// kind, recorded in the same transaction.
    pub fn assign(&mut self, db: &Connection, kind: Kind, uid: &str) -> Result<i64, StoreError> {
        if let Some((_, id)) = self.by_uid.get(uid) {
            return Ok(*id);
        }
        let id: i64 = db.query_row(
            "SELECT coalesce(max(id), 0) + 1 FROM ids WHERE kind = ?1",
            [kind.name()],
            |row| row.get(0),
        )?;
        self.record(db, kind, uid, id)?;
        Ok(id)
    }

    /// Give `uid` exactly `id` (a migrated row keeps its old id).
    pub fn record(
        &mut self,
        db: &Connection,
        kind: Kind,
        uid: &str,
        id: i64,
    ) -> Result<(), StoreError> {
        db.execute(
            "INSERT INTO ids (uid, kind, id) VALUES (?1, ?2, ?3)",
            params![uid, kind.name(), id],
        )?;
        self.by_uid.insert(uid.to_owned(), (kind, id));
        self.by_id.insert((kind, id), uid.to_owned());
        Ok(())
    }
}

/// A new file id: a random UUID v4.
pub fn new_uid() -> String {
    let random = || getrandom::u64().unwrap_or_else(|_| fallback_random());
    let (a, b) = (random(), random());
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.iter().fold(String::with_capacity(32), |mut hex, b| {
        use std::fmt::Write as _;
        // Writing to a String cannot fail.
        let _ = write!(hex, "{b:02x}");
        hex
    });
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Without operating-system randomness: the clock and a counter, still unique per process.
fn fallback_random() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    #[expect(
        clippy::cast_possible_truncation,
        reason = "only the low bits are wanted"
    )]
    let nanos = nanos as u64;
    nanos
        ^ COUNTER
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The folder as the database thread holds it between commands.
#[derive(Debug, Clone)]
pub struct State {
    pub root: PathBuf,
    pub model: Model,
    /// Each file's bytes as read or last written, by its `/`-separated path.
    pub files: BTreeMap<String, Vec<u8>>,
    /// The path of each id's file as it is on disk, which a hand-named file keeps until it
    /// changes.
    pub paths: HashMap<String, String>,
    pub ids: Ids,
    /// Why the folder could not be read; nothing is written to it until it can be.
    pub error: Option<String>,
}

impl State {
    /// No folder to work on: nothing reads from it and nothing is written to it.
    pub fn unavailable(root: &Path, why: &str) -> Self {
        Self {
            root: root.to_owned(),
            model: Model::default(),
            files: BTreeMap::new(),
            paths: HashMap::new(),
            ids: Ids::default(),
            error: Some(why.to_owned()),
        }
    }

    /// Read the folder at `root` (empty when missing) and give every file an id.
    pub fn load(db: &Connection, root: &Path) -> Result<Self, StoreError> {
        let mut ids = Ids::load(db)?;
        let read = format::read_dir(root)
            .map_err(|e| e.to_string())
            .and_then(|files| {
                let (project, paths) = format::read_owned(&files).map_err(|e| e.to_string())?;
                Ok((files, project, paths))
            });
        let (files, model, paths, error) = match read {
            Ok((files, project, paths)) => (files, Model::from_project(project), paths, None),
            Err(error) => (
                BTreeMap::new(),
                Model::default(),
                HashMap::new(),
                Some(error),
            ),
        };
        for c in &model.collections {
            ids.assign(db, Kind::Collection, &c.uid)?;
        }
        for f in &model.folders {
            ids.assign(db, Kind::Folder, &f.uid)?;
        }
        for r in &model.requests {
            ids.assign(db, Kind::Request, &r.uid)?;
        }
        for e in &model.environments {
            ids.assign(db, Kind::Environment, &e.uid)?;
        }
        Ok(Self {
            root: root.to_owned(),
            model,
            files,
            paths,
            ids,
            error,
        })
    }

    /// Write what changed since `before`. Each id's file is compared as `before` and now
    /// would render it: only those that differ (or moved) are written, at their new path,
    /// and the old file goes; files of removed ids go too. Every touched file must still
    /// be on disk as it was read or written, else nothing is written.
    pub fn write_changes(&mut self, before: &Self) -> Result<(), StoreError> {
        if let Some(error) = &self.error {
            return Err(StoreError::Files(format!(
                "The collections folder could not be read, so nothing is written to it: {error}"
            )));
        }
        let old = format::render_owned(&before.model.to_project());
        let new = format::render_owned(&self.model.to_project());
        let mut paths = before.paths.clone();
        let mut writes: Vec<(String, Vec<u8>)> = Vec::new();
        let mut removals: Vec<String> = Vec::new();
        for (uid, (path, bytes)) in &new {
            if old.get(uid) == Some(&(path.clone(), bytes.clone())) {
                continue;
            }
            if let Some(current) = before.paths.get(uid)
                && current != path
            {
                removals.push(current.clone());
            }
            writes.push((path.clone(), bytes.clone()));
            paths.insert(uid.clone(), path.clone());
        }
        for uid in old.keys().filter(|uid| !new.contains_key(*uid)) {
            if let Some(current) = before.paths.get(uid) {
                removals.push(current.clone());
            }
            paths.remove(uid);
        }
        // A file another id keeps must not be overwritten (a hand-named file in the way).
        for (path, _) in &writes {
            let owner = paths.iter().find(|(_, p)| *p == path).map(|(uid, _)| uid);
            if let Some(owner) = owner
                && new.get(owner).is_none_or(|(p, _)| p != path)
            {
                return Err(StoreError::Files(format!(
                    "{path} belongs to another item; rename one of them"
                )));
            }
        }
        removals.retain(|path| !writes.iter().any(|(p, _)| p == path));
        for path in writes.iter().map(|(p, _)| p).chain(&removals) {
            let on_disk = std::fs::read(self.root.join(path)).ok();
            if on_disk.as_ref() != before.files.get(path) {
                return Err(StoreError::Files(format!(
                    "{path} changed on disk since Imposter read it; nothing was saved"
                )));
            }
        }
        for (path, bytes) in &writes {
            atomic_write(&self.root.join(path), bytes)
                .map_err(|e| StoreError::Files(format!("Cannot write {path}: {e}")))?;
        }
        for path in &removals {
            let file = self.root.join(path);
            std::fs::remove_file(&file)
                .map_err(|e| StoreError::Files(format!("Cannot remove {path}: {e}")))?;
            remove_empty_parents(&self.root, &file);
        }
        let mut files = before.files.clone();
        for path in &removals {
            files.remove(path);
        }
        for (path, bytes) in writes {
            files.insert(path, bytes);
        }
        self.files = files;
        self.paths = paths;
        Ok(())
    }
}

/// Write to a temporary file beside `path`, then rename it over: a crash leaves the old
/// file or the new one, never half of one.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Remove directories left empty by a removal, up to (not including) `root`.
fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(current) = dir {
        if current == root || !current.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

/// The collections folder the profile names, or the default beside the database.
pub fn folder(db: &Connection, database: &Path) -> Result<PathBuf, StoreError> {
    let named: Option<String> = db
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [FOLDER_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(text) = named
        && let Ok(path) = serde_json::from_str::<String>(&text)
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    Ok(database
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(DEFAULT_FOLDER))
}

/// The settings key naming the collections folder.
pub const FOLDER_KEY: &str = "collections_dir";
/// The folder's name beside the database when none is set.
pub const DEFAULT_FOLDER: &str = "collections";
