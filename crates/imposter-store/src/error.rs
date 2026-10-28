use imposter_core::settings::SettingsError;

/// Rule violations the user can fix; the messages are shown as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Conflict {
    #[error("Cannot move a folder into itself or its descendants")]
    MoveIntoItself,
    #[error("Destination must belong to the same collection")]
    CrossCollection,
    #[error("Parent folder must belong to the same collection")]
    ParentOutsideCollection,
    #[error("Environment name must be nonempty and unique")]
    EnvironmentName,
    #[error("Names must not be empty")]
    EmptyName,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Stored data is not readable: {0}")]
    Json(#[from] serde_json::Error),
    #[error("The {0} no longer exists")]
    NotFound(&'static str),
    #[error(transparent)]
    Conflict(#[from] Conflict),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("Execution exceeds the history storage budget")]
    HistoryBudget,
    #[error("Cannot back up the database before upgrading it: {0}")]
    Backup(#[source] rusqlite::Error),
    #[error(
        "The database was created by a newer Imposter (schema {found}, this version knows {supported})"
    )]
    TooNew { found: usize, supported: usize },
    #[error("Upgrade step {version} left broken references; the database was not changed")]
    ForeignKeys { version: usize },
    #[error("Cannot start the database thread: {0}")]
    Thread(#[source] std::io::Error),
    #[error("The database is closed")]
    Closed,
    #[error("A database operation failed unexpectedly and was rolled back")]
    Panicked,
}
