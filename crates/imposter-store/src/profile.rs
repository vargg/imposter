//! One-time adoption of a legacy profile, with an atomic, no-clobber SQLite snapshot.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};

use crate::schema;

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error(
        "Cannot copy the legacy profile: {0}. Check file permissions and available disk space; the original database is unchanged"
    )]
    Io(#[from] std::io::Error),
    #[error(
        "Cannot read the legacy profile: {0}. Restore a valid backup or set IMPOSTER_DATA_DIR to a separate directory"
    )]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "The legacy profile uses unsupported schema {0}; install a compatible Imposter version"
    )]
    Version(i64),
    #[error(
        "The legacy profile failed validation; restore a valid backup or set IMPOSTER_DATA_DIR to a separate directory"
    )]
    Invalid,
    #[error("The legacy profile stayed busy for five seconds; close Poster and retry")]
    Busy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    Copied,
    Existing,
    NoSource,
}

/// Copy committed data (including WAL pages) without modifying the source or replacing a
/// destination another process has created. Both paths must be explicit; this layer never
/// discovers a personal profile.
pub fn copy_legacy(source: &Path, destination: &Path) -> Result<CopyOutcome, CopyError> {
    copy_with(source, destination, Duration::from_secs(5), || {})
}

fn copy_with(
    source: &Path,
    destination: &Path,
    timeout: Duration,
    before_publish: impl FnOnce(),
) -> Result<CopyOutcome, CopyError> {
    if destination.try_exists()? {
        return Ok(CopyOutcome::Existing);
    }
    if !source.try_exists()? {
        return Ok(CopyOutcome::NoSource);
    }
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "database path needs a parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        source.busy_timeout(Duration::ZERO)?;
        let mut target = Connection::open(temporary.path())?;
        {
            let backup = Backup::new(&source, &mut target)?;
            let deadline = Instant::now() + timeout;
            loop {
                match backup.step(128)? {
                    StepResult::Done => break,
                    StepResult::Busy | StepResult::Locked => {
                        if Instant::now() >= deadline {
                            return Err(CopyError::Busy);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    _ => {
                        if Instant::now() >= deadline {
                            return Err(CopyError::Busy);
                        }
                    }
                }
            }
        }
        validate(&target)?;
        target.close().map_err(|(_, error)| error)?;
        source.close().map_err(|(_, error)| error)?;
    }
    temporary.as_file().sync_all()?;
    before_publish();
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(CopyOutcome::Copied),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(CopyOutcome::Existing)
        }
        Err(error) => Err(error.error.into()),
    }
}

fn validate(db: &Connection) -> Result<(), CopyError> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 || usize::try_from(version).map_or(true, |v| v > schema::MIGRATIONS.len()) {
        return Err(CopyError::Version(version));
    }
    let integrity: String = db.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" || db.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(CopyError::Invalid);
    }
    // A version number alone must not turn an unrelated SQLite file into an empty profile.
    for table in [
        "collections",
        "folders",
        "requests",
        "environments",
        "globals",
        "settings",
        "ui_state",
        "executions",
    ] {
        db.prepare(&format!("SELECT * FROM {table} LIMIT 0"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(path: &Path) -> Connection {
        let mut db = schema::connect(path).unwrap();
        schema::migrate(&mut db, path, schema::MIGRATIONS).unwrap();
        db.execute("INSERT INTO settings VALUES ('mocks', '{\"port\":0}')", [])
            .unwrap();
        db.execute("INSERT INTO ui_state VALUES ('layout', '\"stacked\"')", [])
            .unwrap();
        db
    }

    #[test]
    fn snapshot_includes_wal_and_leaves_source_and_existing_destination_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("poster.db");
        let target = dir.path().join("new/imposter.db");
        let db = legacy(&source);
        db.execute_batch(r#"
            INSERT INTO collections(name,variables) VALUES('API','{"host":"local"}');
            INSERT INTO folders(collection_id,name) VALUES(1,'Folder');
            INSERT INTO requests(collection_id,folder_id,template) VALUES(1,1,'{"name":"Get","method":"GET","position":0}');
            INSERT INTO environments(name,variables) VALUES('Dev','{"token":"saved"}');
            UPDATE globals SET variables='{"global":"value"}';
            INSERT INTO executions(timestamp,name,method,url,status,outcome,elapsed,environment,truncated,payload,payload_size)
                VALUES(123,'Get','GET','http://local',200,'success',1,'Dev',0,'{}',2);
        "#).unwrap();
        assert!(source.with_extension("db-wal").exists());
        assert_eq!(copy_legacy(&source, &target).unwrap(), CopyOutcome::Copied);
        let copied = Connection::open(&target).unwrap();
        for table in [
            "collections",
            "folders",
            "requests",
            "environments",
            "globals",
            "settings",
            "ui_state",
            "executions",
        ] {
            let rows = |connection: &Connection| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM {table}"))
                    .unwrap();
                let columns = statement.column_count();
                statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|i| row.get::<_, rusqlite::types::Value>(i))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(rows(&db), rows(&copied), "{table}");
        }
        let dump = |db: &Connection| {
            db.query_row("SELECT group_concat(key || value) FROM settings", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
        };
        assert_eq!(dump(&db), dump(&copied));
        assert_eq!(
            copied
                .query_row("SELECT value FROM ui_state", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "\"stacked\""
        );
        db.execute("INSERT INTO settings VALUES ('later', 'true')", [])
            .unwrap();
        assert_eq!(
            copy_legacy(&source, &target).unwrap(),
            CopyOutcome::Existing
        );
        assert_ne!(dump(&db), dump(&copied));
        assert_eq!(
            copy_legacy(&dir.path().join("missing"), &target).unwrap(),
            CopyOutcome::Existing
        );
    }

    #[test]
    fn missing_invalid_newer_and_locked_sources_do_not_publish_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("poster.db");
        let target = dir.path().join("new/imposter.db");
        assert_eq!(
            copy_legacy(&source, &target).unwrap(),
            CopyOutcome::NoSource
        );
        std::fs::write(&source, "not a database").unwrap();
        assert!(matches!(
            copy_legacy(&source, &target),
            Err(CopyError::Sqlite(_))
        ));
        std::fs::remove_file(&source).unwrap();
        let db = legacy(&source);
        db.pragma_update(None, "user_version", 999).unwrap();
        assert!(matches!(
            copy_legacy(&source, &target),
            Err(CopyError::Version(999))
        ));
        db.pragma_update(None, "user_version", 1).unwrap();
        db.pragma_update(None, "journal_mode", "DELETE").unwrap();
        db.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert!(matches!(
            copy_with(&source, &target, Duration::ZERO, || {}),
            Err(CopyError::Busy)
        ));
        db.execute_batch("ROLLBACK").unwrap();
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            0
        );
    }

    #[test]
    fn concurrent_destination_creation_wins_and_temporary_copy_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("poster.db");
        let target = dir.path().join("new/imposter.db");
        let _db = legacy(&source);
        assert_eq!(
            copy_with(&source, &target, Duration::from_secs(5), || {
                std::fs::write(&target, "concurrent profile").unwrap();
            })
            .unwrap(),
            CopyOutcome::Existing
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "concurrent profile"
        );
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn broken_references_and_unrelated_schemas_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("poster.db");
        let target = dir.path().join("imposter.db");
        let db = legacy(&source);
        db.pragma_update(None, "foreign_keys", false).unwrap();
        db.execute(
            "INSERT INTO folders(collection_id,name) VALUES(999,'bad')",
            [],
        )
        .unwrap();
        assert!(matches!(
            copy_legacy(&source, &target),
            Err(CopyError::Invalid)
        ));
        db.execute("DELETE FROM folders", []).unwrap();
        db.execute("DROP TABLE executions", []).unwrap();
        assert!(matches!(
            copy_legacy(&source, &target),
            Err(CopyError::Sqlite(_))
        ));
        assert!(!target.exists());
    }

    #[test]
    fn inaccessible_source_and_destination_errors_are_actionable() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new/imposter.db");
        assert!(matches!(
            copy_legacy(dir.path(), &target),
            Err(CopyError::Sqlite(_))
        ));
        let source = dir.path().join("poster.db");
        let _db = legacy(&source);
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "blocked").unwrap();
        let error = copy_legacy(&source, &blocker.join("imposter.db")).unwrap_err();
        assert!(matches!(error, CopyError::Io(_)));
        assert!(error.to_string().contains("permissions"));
        assert!(!target.exists());
    }
}
