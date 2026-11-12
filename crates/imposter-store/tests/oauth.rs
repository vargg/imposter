//! OAuth 2.0 tokens: cached per folder and environment by key, replaced by key, removed by
//! key, and gone with their environment.
#![cfg(test)]

mod support;

use imposter_store::oauth::{self, StoredToken};
use imposter_store::{environments, files};
use support::{store, vars};

fn token(key: &str, access: &str) -> StoredToken {
    StoredToken {
        key: key.into(),
        access: access.into(),
        refresh: Some("r".into()),
        token_type: "Bearer".into(),
        expires_at: Some(1_000),
    }
}

#[test]
fn tokens_are_kept_per_folder_environment_and_key() {
    let (dir, store) = store();
    let dev = store
        .call_blocking(|db| Ok(environments::create_environment(db, "Dev", &vars(&[]))?.id))
        .unwrap();
    store
        .call_blocking(move |db| {
            oauth::put(db, None, &token("a", "none-1"))?;
            oauth::put(db, None, &token("a", "none-2"))?;
            oauth::put(db, None, &token("b", "none-b"))?;
            oauth::put(db, Some(dev), &token("a", "dev"))
        })
        .unwrap();
    let read = |environment| {
        store
            .call_blocking(move |db| oauth::read(db, environment))
            .unwrap()
    };
    assert_eq!(read(None), [token("a", "none-2"), token("b", "none-b")]);
    assert_eq!(read(Some(dev)), [token("a", "dev")]);

    store
        .call_blocking(|db| oauth::remove(db, None, "b"))
        .unwrap();
    assert_eq!(read(None), [token("a", "none-2")]);

    let other = dir.path().join("other");
    store
        .call_blocking(move |db| files::switch(db, &other))
        .unwrap();
    assert_eq!(read(None), [], "another folder has its own tokens");
    let first = dir.path().join("collections");
    store
        .call_blocking(move |db| files::switch(db, &first))
        .unwrap();

    store
        .call_blocking(move |db| environments::delete_environment(db, dev))
        .unwrap();
    let left: i64 = store
        .call_blocking(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM oauth_tokens WHERE environment_id IS NOT NULL",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(left, 0, "an environment's tokens go with it");
}
