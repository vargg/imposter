//! Schema migrations. Released migrations are frozen: change the schema only by appending a
//! new migration (`tests/schema.rs` pins the released ones by hash).

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::StoreError;

/// Every migration in order; the database's `user_version` counts how many have run.
pub const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7, V8];

const V1: &str = r"
CREATE TABLE collections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    variables TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    parent_folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX folders_by_collection ON folders(collection_id);

-- The template JSON is the single source of truth; the tree reads the generated columns
-- without decoding bodies.
CREATE TABLE requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    template TEXT NOT NULL,
    name TEXT GENERATED ALWAYS AS (json_extract(template, '$.name')) VIRTUAL,
    method TEXT GENERATED ALWAYS AS (json_extract(template, '$.method')) VIRTUAL,
    position INTEGER GENERATED ALWAYS AS (json_extract(template, '$.position')) VIRTUAL
);
CREATE INDEX requests_by_collection ON requests(collection_id);

CREATE TABLE environments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    variables TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE globals (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    variables TEXT NOT NULL DEFAULT '{}'
);
INSERT INTO globals (id) VALUES (1);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE ui_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- History has no foreign key to requests: deleting a request keeps its executions.
CREATE TABLE executions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER,
    timestamp INTEGER NOT NULL,
    name TEXT NOT NULL,
    method TEXT NOT NULL,
    url TEXT NOT NULL,
    status INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    elapsed REAL NOT NULL,
    environment TEXT NOT NULL,
    truncated INTEGER NOT NULL,
    payload TEXT NOT NULL,
    payload_size INTEGER NOT NULL
);
CREATE INDEX executions_by_request ON executions(request_id);
";

/// Saved values: per scope, beside its variables (see `scopes`).
const V2: &str = r"
ALTER TABLE globals ADD COLUMN saved_values TEXT NOT NULL DEFAULT '{}';
ALTER TABLE environments ADD COLUMN saved_values TEXT NOT NULL DEFAULT '{}';
ALTER TABLE collections ADD COLUMN saved_values TEXT NOT NULL DEFAULT '{}';
";

/// The URL beside name and method, for the sidebar filter, without decoding templates.
const V3: &str = r"
ALTER TABLE requests ADD COLUMN url TEXT GENERATED ALWAYS AS (json_extract(template, '$.url')) VIRTUAL;
";

/// A color per environment (`EnvColor` by name), or none.
const V4: &str = r"
ALTER TABLE environments ADD COLUMN color TEXT
    CHECK (color IN ('red', 'yellow', 'green', 'blue', 'purple'));
";

/// Scripts on collections and folders (`model::Scripts` as JSON), run around every request
/// inside them.
const V5: &str = r"
ALTER TABLE collections ADD COLUMN scripts TEXT NOT NULL DEFAULT '{}';
ALTER TABLE folders ADD COLUMN scripts TEXT NOT NULL DEFAULT '{}';
";

/// Which interface started an execution (`execution::Via`), for History's list to show.
const V6: &str = r"
ALTER TABLE executions ADD COLUMN via TEXT NOT NULL DEFAULT 'app' CHECK (via IN ('app', 'command_line'));
";

/// Secrets, apart from variables so no export or file ever reads them: global ones
/// (`environment_id` NULL) and each environment's, which go with it. Rows keep their order
/// by id.
const V7: &str = r"
CREATE TABLE secrets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    environment_id INTEGER REFERENCES environments(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL
);
CREATE UNIQUE INDEX secrets_by_name ON secrets (coalesce(environment_id, 0), name);
";

/// Collections and environments move to files (`files`): `ids` gives each file's id a
/// stable number, `local_scopes` holds the values that stay on this computer (what captures
/// set, and an environment's values when it does not share them), and secrets lose their
/// reference to the environments table, which files replace.
const V8: &str = r"
CREATE TABLE ids (
    uid TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('collection', 'folder', 'request', 'environment')),
    id INTEGER NOT NULL,
    UNIQUE (kind, id)
);
CREATE TABLE local_scopes (
    scope TEXT PRIMARY KEY,
    variables TEXT NOT NULL DEFAULT '{}',
    saved_values TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE secrets_v8 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    environment_id INTEGER,
    name TEXT NOT NULL,
    value TEXT NOT NULL
);
INSERT INTO secrets_v8 (id, environment_id, name, value)
    SELECT id, environment_id, name, value FROM secrets;
DROP TABLE secrets;
ALTER TABLE secrets_v8 RENAME TO secrets;
CREATE UNIQUE INDEX secrets_by_name ON secrets (coalesce(environment_id, 0), name);
";

/// How long a write waits for another process (the app, or the command line beside it)
/// to finish its own before failing.
pub const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Open a connection with the pragmas every connection needs.
pub fn connect(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub from: usize,
    pub to: usize,
    /// The copy taken before upgrading an existing database.
    pub backup: Option<PathBuf>,
}

pub fn user_version(connection: &Connection) -> Result<usize, StoreError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(usize::try_from(version).unwrap_or(0))
}

/// Bring the database at `path` up to date. An existing database is first copied to
/// `<path>.bak-<version>` with SQLite's backup API; each migration runs in its own
/// transaction and must leave no foreign key violations.
pub fn migrate(
    connection: &mut Connection,
    path: &Path,
    migrations: &[&str],
) -> Result<Migrated, StoreError> {
    let from = user_version(connection)?;
    if from > migrations.len() {
        return Err(StoreError::TooNew {
            found: from,
            supported: migrations.len(),
        });
    }
    let mut backup = None;
    if from > 0 && from < migrations.len() {
        let mut target = path.as_os_str().to_owned();
        target.push(format!(".bak-{from}"));
        let target = PathBuf::from(target);
        connection
            .backup(
                rusqlite::MAIN_DB,
                &target,
                None::<fn(rusqlite::backup::Progress)>,
            )
            .map_err(StoreError::Backup)?;
        tracing::info!(from, to = migrations.len(), backup = %target.display(), "upgrading database");
        backup = Some(target);
    }
    for (index, sql) in migrations.iter().enumerate().skip(from) {
        let version = index + 1;
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        let violations: i64 =
            transaction.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations > 0 {
            return Err(StoreError::ForeignKeys { version });
        }
        transaction.pragma_update(
            None,
            "user_version",
            i64::try_from(version).unwrap_or(i64::MAX),
        )?;
        transaction.commit()?;
    }
    Ok(Migrated {
        from,
        to: migrations.len(),
        backup,
    })
}
