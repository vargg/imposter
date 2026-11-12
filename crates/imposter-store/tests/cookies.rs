//! Cookie jars: one per collections folder and environment, replaced whole, an empty one
//! leaving nothing, and gone with their environment.
#![cfg(test)]

mod support;

use imposter_store::{cookies, environments, files};
use support::{store, vars};

#[test]
fn each_folder_and_environment_keeps_its_own_jar() {
    let (dir, store) = store();
    let dev = store
        .call_blocking(|db| Ok(environments::create_environment(db, "Dev", &vars(&[]))?.id))
        .unwrap();
    store
        .call_blocking(move |db| {
            cookies::write(db, None, "[\"none\"]")?;
            cookies::write(db, Some(dev), "[\"dev\"]")
        })
        .unwrap();
    let read = |environment| {
        store
            .call_blocking(move |db| cookies::read(db, environment))
            .unwrap()
    };
    assert_eq!(read(None), "[\"none\"]");
    assert_eq!(read(Some(dev)), "[\"dev\"]");

    let other = dir.path().join("other");
    store
        .call_blocking(move |db| files::switch(db, &other))
        .unwrap();
    assert_eq!(read(None), "", "another folder starts without cookies");
    store
        .call_blocking(|db| cookies::write(db, None, "[\"other\"]"))
        .unwrap();
    let first = dir.path().join("collections");
    store
        .call_blocking(move |db| files::switch(db, &first))
        .unwrap();
    assert_eq!(read(None), "[\"none\"]", "and keeps its own");
}

#[test]
fn an_empty_jar_leaves_no_row_and_an_environment_takes_its_jar_along() {
    let (_dir, store) = store();
    let dev = store
        .call_blocking(|db| Ok(environments::create_environment(db, "Dev", &vars(&[]))?.id))
        .unwrap();
    let rows = |store: &imposter_store::Store| {
        store
            .call_blocking(|db| {
                Ok(db.query_row("SELECT count(*) FROM cookie_jars", [], |row| {
                    row.get::<_, i64>(0)
                })?)
            })
            .unwrap()
    };
    store
        .call_blocking(move |db| {
            cookies::write(db, None, "[\"kept\"]")?;
            cookies::write(db, Some(dev), "[\"dev\"]")
        })
        .unwrap();
    assert_eq!(rows(&store), 2);
    store
        .call_blocking(|db| cookies::write(db, None, "[]"))
        .unwrap();
    assert_eq!(rows(&store), 1);
    store
        .call_blocking(move |db| environments::delete_environment(db, dev))
        .unwrap();
    assert_eq!(rows(&store), 0);
}
