//! OAuth 2.0 tokens, cached per collections folder and environment by what they are for
//! (`imposter_app::oauth` computes the key and decides when one is still good).

use imposter_core::ids::EnvironmentId;
use rusqlite::params;

use crate::{StoreError, Tx};

/// A token as cached: what it is for, and what the token endpoint gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToken {
    pub key: String,
    pub access: String,
    pub refresh: Option<String>,
    pub token_type: String,
    /// Milliseconds since the Unix epoch, UTC; `None` when the server gave no lifetime.
    pub expires_at: Option<i64>,
}

fn folder(db: &Tx<'_>) -> String {
    db.folder().display().to_string()
}

/// Every token cached for `environment` (none: requests sent without one) in this folder.
pub fn read(
    db: &Tx<'_>,
    environment: Option<EnvironmentId>,
) -> Result<Vec<StoredToken>, StoreError> {
    let mut statement = db.prepare_cached(
        "SELECT key, access, refresh, token_type, expires_at FROM oauth_tokens \
         WHERE folder = ?1 AND environment_id IS ?2 ORDER BY key",
    )?;
    let rows = statement.query_map(params![folder(db), environment.map(|id| id.0)], |row| {
        Ok(StoredToken {
            key: row.get(0)?,
            access: row.get(1)?,
            refresh: row.get(2)?,
            token_type: row.get(3)?,
            expires_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Keep `token`, replacing the one cached under its key.
pub fn put(
    db: &Tx<'_>,
    environment: Option<EnvironmentId>,
    token: &StoredToken,
) -> Result<(), StoreError> {
    let (folder, id) = (folder(db), environment.map(|id| id.0));
    remove(db, environment, &token.key)?;
    db.execute(
        "INSERT INTO oauth_tokens (folder, environment_id, key, access, refresh, token_type, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![folder, id, token.key, token.access, token.refresh, token.token_type, token.expires_at],
    )?;
    Ok(())
}

/// Forget the token cached under `key`.
pub fn remove(
    db: &Tx<'_>,
    environment: Option<EnvironmentId>,
    key: &str,
) -> Result<(), StoreError> {
    db.execute(
        "DELETE FROM oauth_tokens WHERE folder = ?1 AND environment_id IS ?2 AND key = ?3",
        params![folder(db), environment.map(|id| id.0), key],
    )?;
    Ok(())
}

/// Forget an environment's tokens, in every folder, as the environment goes.
pub(crate) fn forget(db: &Tx<'_>, environment: EnvironmentId) -> Result<(), StoreError> {
    db.execute(
        "DELETE FROM oauth_tokens WHERE environment_id = ?1",
        [environment.0],
    )?;
    Ok(())
}
