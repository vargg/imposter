//! Cookie jars, one per collections folder and environment: the jar's JSON, kept and
//! changed by the app (`imposter_app::cookies`), which knows what a cookie is. Keyed by the
//! folder too, since requests sent without an environment have a jar of their own in each
//! folder.

use imposter_core::ids::EnvironmentId;
use rusqlite::{OptionalExtension, params};

use crate::{StoreError, Tx};

fn folder(db: &Tx<'_>) -> String {
    db.folder().display().to_string()
}

/// The jar for `environment` (none: requests sent without one) in this folder; empty text
/// when nothing was kept.
pub fn read(db: &Tx<'_>, environment: Option<EnvironmentId>) -> Result<String, StoreError> {
    Ok(db
        .query_row(
            "SELECT jar FROM cookie_jars WHERE folder = ?1 AND environment_id IS ?2",
            params![folder(db), environment.map(|id| id.0)],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

/// Replace the jar for `environment`; an empty one (`""` or `[]`) leaves no row.
pub fn write(db: &Tx<'_>, environment: Option<EnvironmentId>, jar: &str) -> Result<(), StoreError> {
    let folder = folder(db);
    let id = environment.map(|id| id.0);
    db.execute(
        "DELETE FROM cookie_jars WHERE folder = ?1 AND environment_id IS ?2",
        params![folder, id],
    )?;
    if !matches!(jar.trim(), "" | "[]") {
        db.execute(
            "INSERT INTO cookie_jars (folder, environment_id, jar) VALUES (?1, ?2, ?3)",
            params![folder, id, jar],
        )?;
    }
    Ok(())
}

/// Remove an environment's jars, in every folder, as the environment goes.
pub(crate) fn forget(db: &Tx<'_>, environment: EnvironmentId) -> Result<(), StoreError> {
    db.execute(
        "DELETE FROM cookie_jars WHERE environment_id = ?1",
        [environment.0],
    )?;
    Ok(())
}
