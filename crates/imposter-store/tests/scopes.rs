//! Saved values: kept per scope beside its variables, written together, chosen one at a time.
#![cfg(test)]

mod support;

use imposter_core::ids::{CollectionId, EnvironmentId};
use imposter_core::model::SavedValues;
use imposter_core::variables::Scope;
use imposter_store::schema::MIGRATIONS;
use imposter_store::{Store, StoreError, collections, environments, scopes};
use support::{store, vars};

fn saved(name: &str, values: &[&str]) -> SavedValues {
    let mut saved = SavedValues::default();
    saved.set(name, values.iter().map(|v| (*v).to_owned()));
    saved
}

#[test]
fn every_scope_keeps_its_own_saved_values() {
    let (_dir, store) = store();
    let (dev, prod, api) = store
        .call_blocking(|db| {
            Ok((
                environments::create_environment(db, "Dev", &vars(&[]))?.id,
                environments::create_environment(db, "Prod", &vars(&[]))?.id,
                collections::create_collection(db, "API", "", &vars(&[]))?.id,
            ))
        })
        .unwrap();
    let scopes = [
        (Scope::Global, "g"),
        (Scope::Environment(dev), "dev"),
        (Scope::Environment(prod), "prod"),
        (Scope::Collection(api), "api"),
    ];
    for (scope, value) in scopes {
        store
            .call_blocking(move |db| {
                scopes::write(
                    db,
                    scope,
                    &vars(&[("user", value)]),
                    &saved("user", &[value, "shared"]),
                )
            })
            .unwrap();
    }
    for (scope, value) in scopes {
        let (variables, saved_values) = store
            .call_blocking(move |db| scopes::read(db, scope))
            .unwrap();
        assert_eq!(variables, vars(&[("user", value)]));
        assert_eq!(saved_values, saved("user", &[value, "shared"]), "{scope:?}");
    }
}

#[test]
fn saved_values_follow_their_variables() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            let mut both = saved("a", &["1"]);
            both.set("gone", ["x".to_owned()]);
            scopes::write(db, Scope::Global, &vars(&[("a", "1")]), &both)
        })
        .unwrap();
    assert_eq!(
        store
            .call_blocking(|db| scopes::saved_values(db, Scope::Global))
            .unwrap(),
        saved("a", &["1"]),
        "lists of undefined names are not kept"
    );

    store
        .call_blocking(|db| environments::set_globals(db, &vars(&[("b", "2")])))
        .unwrap();
    assert!(
        store
            .call_blocking(|db| scopes::saved_values(db, Scope::Global))
            .unwrap()
            .is_empty(),
        "a variable removed by another writer takes its list along"
    );
}

#[test]
fn choosing_changes_only_that_value() {
    let (_dir, store) = store();
    let dev = store
        .call_blocking(|db| {
            let dev = environments::create_environment(
                db,
                "Dev",
                &vars(&[("first", "f"), ("user", "42"), ("last", "l")]),
            )?;
            scopes::write(
                db,
                Scope::Environment(dev.id),
                &dev.variables,
                &saved("user", &["42", "77"]),
            )?;
            Ok(dev.id)
        })
        .unwrap();

    store
        .call_blocking(move |db| scopes::choose_value(db, Scope::Environment(dev), "user", "77"))
        .unwrap();
    let (variables, saved_values) = store
        .call_blocking(move |db| scopes::read(db, Scope::Environment(dev)))
        .unwrap();
    assert_eq!(
        variables,
        vars(&[("first", "f"), ("user", "77"), ("last", "l")])
    );
    assert_eq!(saved_values, saved("user", &["42", "77"]));

    let gone = store
        .call_blocking(move |db| scopes::choose_value(db, Scope::Environment(dev), "missing", "x"));
    assert!(matches!(gone, Err(StoreError::NotFound("variable"))));
    for scope in [
        Scope::Environment(EnvironmentId(999)),
        Scope::Collection(CollectionId(999)),
    ] {
        let result = store.call_blocking(move |db| scopes::choose_value(db, scope, "user", "x"));
        assert!(matches!(result, Err(StoreError::NotFound(_))), "{scope:?}");
        let result = store
            .call_blocking(move |db| scopes::write(db, scope, &vars(&[]), &SavedValues::default()));
        assert!(matches!(result, Err(StoreError::NotFound(_))), "{scope:?}");
    }
}

#[test]
fn the_active_environment_brings_its_saved_values() {
    let (_dir, store) = store();
    let context = store
        .call_blocking(|db| {
            scopes::write(
                db,
                Scope::Global,
                &vars(&[("g", "1")]),
                &saved("g", &["1", "2"]),
            )?;
            let dev = environments::create_environment(db, "Dev", &vars(&[("id", "42")]))?;
            scopes::write(
                db,
                Scope::Environment(dev.id),
                &dev.variables,
                &saved("id", &["42", "77"]),
            )?;
            environments::set_active_environment(db, Some(dev.id))?;
            environments::variable_context(db)
        })
        .unwrap();
    assert_eq!(context.global_saved, saved("g", &["1", "2"]));
    assert_eq!(
        context.environment.unwrap().saved,
        saved("id", &["42", "77"])
    );
}

#[test]
fn a_version_1_database_gains_empty_saved_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("imposter.db");
    Store::open_with(&path, &MIGRATIONS[..1])
        .unwrap()
        .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
        .unwrap();

    let store = Store::open(&path).unwrap();
    assert_eq!(store.migrated().from, 1);
    assert!(store.migrated().backup.is_some());
    let (variables, saved_values) = store
        .call_blocking(|db| scopes::read(db, Scope::Global))
        .unwrap();
    assert_eq!(variables, vars(&[("a", "1")]));
    assert!(saved_values.is_empty());
}

#[test]
fn captured_values_replace_current_ones_and_keep_saved_values() {
    let (_dir, store) = store();
    let dev = store
        .call_blocking(|db| {
            let dev =
                environments::create_environment(db, "Dev", &vars(&[("user", "42"), ("x", "1")]))?;
            scopes::write(
                db,
                Scope::Environment(dev.id),
                &dev.variables,
                &saved("user", &["42", "77"]),
            )?;
            Ok(dev.id)
        })
        .unwrap();
    store
        .call_blocking(move |db| {
            scopes::set_current(
                db,
                Scope::Environment(dev),
                &[
                    ("user".to_owned(), "91".to_owned()),
                    ("token".to_owned(), "t".to_owned()),
                ],
            )
        })
        .unwrap();
    let (variables, saved_values) = store
        .call_blocking(move |db| scopes::read(db, Scope::Environment(dev)))
        .unwrap();
    assert_eq!(
        variables,
        vars(&[("user", "91"), ("x", "1"), ("token", "t")]),
        "names keep their place; new ones come last"
    );
    assert_eq!(saved_values, saved("user", &["42", "77"]));
    let gone = store
        .call_blocking(|db| scopes::set_current(db, Scope::Environment(EnvironmentId(999)), &[]));
    assert!(matches!(gone, Err(StoreError::NotFound(_))));
}
