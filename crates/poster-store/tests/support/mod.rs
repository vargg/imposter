//! A fresh database file per test.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use poster_core::model::{RequestTemplate, Variables};
use poster_store::Store;
use tempfile::TempDir;

pub fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("poster.db")).unwrap();
    (dir, store)
}

pub fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

pub fn template(name: &str) -> RequestTemplate {
    RequestTemplate {
        name: name.to_owned(),
        url: format!("https://example.com/{name}"),
        ..RequestTemplate::default()
    }
}
