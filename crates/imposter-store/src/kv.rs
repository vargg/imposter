//! Key/JSON tables: application settings and UI state.

use imposter_core::settings::AppSettings;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Settings,
    UiState,
}

impl Table {
    const fn name(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::UiState => "ui_state",
        }
    }
}

const APP_SETTINGS_KEY: &str = "application";

pub fn get<T: DeserializeOwned>(
    db: &Connection,
    table: Table,
    key: &str,
) -> Result<Option<T>, StoreError> {
    let text: Option<String> = db
        .query_row(
            &format!("SELECT value FROM {} WHERE key = ?1", table.name()),
            [key],
            |row| row.get(0),
        )
        .optional()?;
    text.map(|text| serde_json::from_str(&text))
        .transpose()
        .map_err(Into::into)
}

pub fn set<T: Serialize + ?Sized>(
    db: &Connection,
    table: Table,
    key: &str,
    value: &T,
) -> Result<(), StoreError> {
    db.execute(
        &format!(
            "INSERT INTO {} (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            table.name()
        ),
        params![key, serde_json::to_string(value)?],
    )?;
    Ok(())
}

pub fn remove(db: &Connection, table: Table, key: &str) -> Result<(), StoreError> {
    db.execute(
        &format!("DELETE FROM {} WHERE key = ?1", table.name()),
        [key],
    )?;
    Ok(())
}

/// Saved settings, or the defaults when none were saved yet. Fields added in later versions
/// take their defaults.
pub fn app_settings(db: &Connection) -> Result<AppSettings, StoreError> {
    Ok(get(db, Table::Settings, APP_SETTINGS_KEY)?.unwrap_or_default())
}

pub fn set_app_settings(db: &Connection, settings: &AppSettings) -> Result<(), StoreError> {
    settings.validate()?;
    set(db, Table::Settings, APP_SETTINGS_KEY, settings)
}
