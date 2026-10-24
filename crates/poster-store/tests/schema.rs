//! Migrations: frozen once released, backed up before upgrades, checked for broken references.
#![cfg(test)]

mod support;

use poster_store::schema::{MIGRATIONS, connect, migrate, user_version};
use poster_store::{Store, StoreError};

/// FNV-1a: a stable fingerprint of each released migration.
fn fingerprint(sql: &str) -> u64 {
    sql.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Released migrations never change: users' databases already ran them. To change the
/// schema, append a migration and add its fingerprint here.
const RELEASED: &[u64] = &[0xefda_02ad_5e13_5c6f];

#[test]
fn released_migrations_are_frozen() {
    let actual: Vec<u64> = MIGRATIONS.iter().map(|sql| fingerprint(sql)).collect();
    assert!(
        actual.len() >= RELEASED.len(),
        "a released migration was removed"
    );
    for (index, (actual, expected)) in actual.iter().zip(RELEASED).enumerate() {
        assert_eq!(
            actual,
            expected,
            "migration {} changed after release: {actual:#018x}",
            index + 1
        );
    }
}

#[test]
fn a_new_database_is_fully_migrated_without_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("poster.db")).unwrap();

    assert_eq!(
        (store.migrated().from, store.migrated().to),
        (0, MIGRATIONS.len())
    );
    assert_eq!(store.migrated().backup, None);
}

#[test]
fn connections_enforce_foreign_keys_in_wal_mode() {
    let dir = tempfile::tempdir().unwrap();
    let db = connect(&dir.path().join("poster.db")).unwrap();
    let foreign_keys: bool = db
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert!(foreign_keys);
    assert_eq!(mode, "wal");
}

#[test]
fn upgrading_backs_up_the_database_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("poster.db");
    let v1 = &MIGRATIONS[..1];
    Store::open_with(&path, v1)
        .unwrap()
        .call_blocking(|db| {
            poster_store::collections::create_collection(
                db,
                "before",
                "",
                &poster_core::model::Variables::new(),
            )
        })
        .unwrap();
    let mut next = v1.to_vec();
    next.push("CREATE TABLE added_later (id INTEGER PRIMARY KEY);");

    let store = Store::open_with(&path, &next).unwrap();

    let backup = store.migrated().backup.clone().unwrap();
    assert_eq!(backup, dir.path().join("poster.db.bak-1"));
    let copy = connect(&backup).unwrap();
    assert_eq!(user_version(&copy).unwrap(), 1);
    let name: String = copy
        .query_row("SELECT name FROM collections", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "before");
    assert!(
        copy.prepare("SELECT * FROM added_later").is_err(),
        "the backup predates the upgrade"
    );
}

#[test]
fn a_database_from_a_newer_app_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("poster.db");
    let mut later = MIGRATIONS.to_vec();
    later.push("SELECT 1;");
    drop(Store::open_with(&path, &later).unwrap());

    assert!(
        matches!(Store::open(&path), Err(StoreError::TooNew { found, supported }) if found == later.len() && supported == MIGRATIONS.len())
    );
}

#[test]
fn a_migration_that_breaks_references_is_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("poster.db");
    let mut db = connect(&path).unwrap();
    migrate(&mut db, &path, MIGRATIONS).unwrap();
    let mut broken = MIGRATIONS.to_vec();
    broken.push("PRAGMA defer_foreign_keys = ON; INSERT INTO folders (collection_id, name) VALUES (999, 'orphan');");

    let result = migrate(&mut db, &path, &broken);

    assert!(matches!(result, Err(StoreError::ForeignKeys { version }) if version == broken.len()));
    assert_eq!(user_version(&db).unwrap(), MIGRATIONS.len());
    let folders: i64 = db
        .query_row("SELECT count(*) FROM folders", [], |row| row.get(0))
        .unwrap();
    assert_eq!(folders, 0);
}
