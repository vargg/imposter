//! Where the app keeps its data.

use std::path::PathBuf;

pub const DATA_DIR_VARIABLE: &str = "IMPOSTER_DATA_DIR";
pub const DATABASE_FILE: &str = "imposter.db";

/// Log directives for the binary; legacy environment variables intentionally have no alias.
pub fn log_filter() -> String {
    std::env::var("IMPOSTER_LOG").unwrap_or_else(|_| "info".to_owned())
}

/// Open the selected profile before the UI starts. Explicit directories never consult home.
pub fn open_profile() -> Result<imposter_store::Store, OpenError> {
    open_profile_from(std::env::var_os(DATA_DIR_VARIABLE), std::env::home_dir())
}

fn open_profile_from(
    explicit: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Result<imposter_store::Store, OpenError> {
    if let Some(dir) = explicit {
        if dir.is_empty() {
            return Err(OpenError::EmptyDirectory);
        }
        return open_store(std::path::Path::new(&dir));
    }
    let home = home.ok_or(OpenError::NoHome)?;
    let dir = home.join(".imposter");
    imposter_store::profile::copy_legacy(
        &home.join(".poster/poster.db"),
        &dir.join(DATABASE_FILE),
    )?;
    open_store(&dir)
}

/// Create `dir` if needed and open the database in it.
pub fn open_store(dir: &std::path::Path) -> Result<imposter_store::Store, OpenError> {
    std::fs::create_dir_all(dir).map_err(|source| OpenError::Directory {
        dir: dir.to_owned(),
        source,
    })?;
    Ok(imposter_store::Store::open(&dir.join(DATABASE_FILE))?)
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("No home directory; set IMPOSTER_DATA_DIR")]
    NoHome,
    #[error("IMPOSTER_DATA_DIR is empty; set it to a directory or unset it")]
    EmptyDirectory,
    #[error(transparent)]
    Copy(#[from] imposter_store::profile::CopyError),
    #[error("Cannot create the data directory {dir}: {source}")]
    Directory {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] imposter_store::StoreError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_copies_once_and_explicit_profiles_are_isolated() {
        let home = tempfile::tempdir().unwrap();
        let legacy = home.path().join(".poster");
        std::fs::create_dir(&legacy).unwrap();
        let store = imposter_store::Store::open(&legacy.join("poster.db")).unwrap();
        store
            .handle()
            .call_blocking(|db| {
                imposter_store::kv::set(db, imposter_store::kv::Table::Settings, "marker", &true)
            })
            .unwrap();
        store.close().unwrap();
        let explicit = home.path().join("explicit");
        let isolated =
            open_profile_from(Some(explicit.into_os_string()), Some(home.path().into())).unwrap();
        assert!(!home.path().join(".imposter").exists());
        isolated.close().unwrap();
        for _ in 0..2 {
            let store = open_profile_from(None, Some(home.path().into())).unwrap();
            assert_eq!(
                store
                    .handle()
                    .call_blocking(|db| imposter_store::kv::get::<bool>(
                        db,
                        imposter_store::kv::Table::Settings,
                        "marker"
                    ))
                    .unwrap(),
                Some(true)
            );
            store.close().unwrap();
        }
    }

    #[test]
    fn fresh_profile_and_invalid_configuration() {
        let home = tempfile::tempdir().unwrap();
        let store = open_profile_from(None, Some(home.path().into())).unwrap();
        assert_eq!(store.path(), home.path().join(".imposter/imposter.db"));
        store.close().unwrap();
        assert!(matches!(
            open_profile_from(None, None),
            Err(OpenError::NoHome)
        ));
        assert!(matches!(
            open_profile_from(Some("".into()), Some(home.path().into())),
            Err(OpenError::EmptyDirectory)
        ));
    }

    #[test]
    fn invalid_legacy_data_is_reported_unless_a_new_or_explicit_profile_wins() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".poster")).unwrap();
        std::fs::write(home.path().join(".poster/poster.db"), "corrupt").unwrap();
        assert!(matches!(
            open_profile_from(None, Some(home.path().into())),
            Err(OpenError::Copy(_))
        ));
        assert!(!home.path().join(".imposter/imposter.db").exists());
        open_profile_from(
            Some(home.path().join("explicit").into_os_string()),
            Some(home.path().into()),
        )
        .unwrap()
        .close()
        .unwrap();
        open_store(&home.path().join(".imposter"))
            .unwrap()
            .close()
            .unwrap();
        open_profile_from(None, Some(home.path().into()))
            .unwrap()
            .close()
            .unwrap();
    }

    #[test]
    fn open_store_creates_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        let store = open_store(&nested).unwrap();
        assert_eq!(store.path(), nested.join(DATABASE_FILE));
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(matches!(
            open_store(&file),
            Err(OpenError::Directory { .. })
        ));
    }
}
