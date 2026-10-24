//! Where the app keeps its data.

use std::path::PathBuf;

pub const DATA_DIR_VARIABLE: &str = "POSTER_DATA_DIR";
pub const DATABASE_FILE: &str = "poster.db";

/// `POSTER_DATA_DIR` when set (tests and smoke runs), otherwise `~/.poster`.
/// `None` only when no home directory is known.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(std::env::var_os(DATA_DIR_VARIABLE), std::env::home_dir())
}

fn data_dir_from(explicit: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    match explicit {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => home.map(|home| home.join(".poster")),
    }
}

/// Create `dir` if needed and open the database in it.
pub fn open_store(dir: &std::path::Path) -> Result<poster_store::Store, OpenError> {
    std::fs::create_dir_all(dir).map_err(|source| OpenError::Directory {
        dir: dir.to_owned(),
        source,
    })?;
    Ok(poster_store::Store::open(&dir.join(DATABASE_FILE))?)
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("Cannot create the data directory {dir}: {source}")]
    Directory {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] poster_store::StoreError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_wins_and_home_is_the_fallback() {
        assert_eq!(
            data_dir_from(Some("/tmp/x".into()), Some("/home/u".into())),
            Some("/tmp/x".into())
        );
        assert_eq!(
            data_dir_from(Some("".into()), Some("/home/u".into())),
            Some("/home/u/.poster".into())
        );
        assert_eq!(data_dir_from(None, None), None);
        assert!(data_dir().is_some());
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
