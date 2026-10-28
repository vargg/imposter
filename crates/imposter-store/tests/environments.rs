//! Environments, globals and the active environment.
#![cfg(test)]

mod support;

use imposter_core::ids::EnvironmentId;
use imposter_store::environments::*;
use imposter_store::{Conflict, StoreError};
use support::{store, vars};

#[test]
fn names_are_unique_nonempty_and_never_global() {
    let (_dir, store) = store();
    let dev = store
        .call_blocking(|db| create_environment(db, "Dev", &vars(&[])))
        .unwrap();

    for name in ["Dev", "", " ", "Global"] {
        let result = store.call_blocking(move |db| create_environment(db, name, &vars(&[])));
        assert!(
            matches!(result, Err(StoreError::Conflict(Conflict::EnvironmentName))),
            "{name:?}"
        );
    }
    store
        .call_blocking(move |db| rename_environment(db, dev.id, "Dev"))
        .unwrap();
    let prod = store
        .call_blocking(|db| create_environment(db, "Prod", &vars(&[])))
        .unwrap();
    assert!(
        store
            .call_blocking(move |db| rename_environment(db, prod.id, "Dev"))
            .is_err()
    );
}

#[test]
fn variables_keep_their_order() {
    let (_dir, store) = store();
    let values = vars(&[("z", "1"), ("a", ""), ("m", "x")]);
    let env = store
        .call_blocking({
            let v = values.clone();
            move |db| create_environment(db, "Dev", &v)
        })
        .unwrap();
    let swapped = vars(&[("m", "x"), ("z", "2")]);
    store
        .call_blocking({
            let v = swapped.clone();
            move |db| set_environment_variables(db, env.id, &v)
        })
        .unwrap();

    let read = store
        .call_blocking(move |db| environment(db, env.id))
        .unwrap()
        .unwrap();
    assert_eq!(read.variables, swapped);
    assert_eq!(read.variables.keys().collect::<Vec<_>>(), ["m", "z"]);

    store
        .call_blocking({
            let v = values.clone();
            move |db| set_globals(db, &v)
        })
        .unwrap();
    assert_eq!(
        store
            .call_blocking(|db| globals(db))
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["z", "a", "m"]
    );
}

#[test]
fn the_active_environment_feeds_the_variable_context() {
    let (_dir, store) = store();
    let env = store
        .call_blocking(|db| {
            set_globals(db, &vars(&[("g", "1")]))?;
            create_environment(db, "Dev", &vars(&[("e", "2")]))
        })
        .unwrap();

    let before = store.call_blocking(|db| variable_context(db)).unwrap();
    assert_eq!(before.environment, None);
    assert_eq!(before.environment_name(), "Global");

    store
        .call_blocking(move |db| set_active_environment(db, Some(env.id)))
        .unwrap();
    let after = store.call_blocking(|db| variable_context(db)).unwrap();
    assert_eq!(after.environment_name(), "Dev");
    assert_eq!(after.environment.unwrap().values, vars(&[("e", "2")]));
    assert_eq!(after.globals, vars(&[("g", "1")]));
}

#[test]
fn deleting_the_active_environment_falls_back_to_globals() {
    let (_dir, store) = store();
    let env = store
        .call_blocking(|db| {
            let env = create_environment(db, "Dev", &vars(&[]))?;
            set_active_environment(db, Some(env.id))?;
            Ok(env)
        })
        .unwrap();

    store
        .call_blocking(move |db| delete_environment(db, env.id))
        .unwrap();

    assert_eq!(
        store.call_blocking(|db| active_environment(db)).unwrap(),
        None
    );
    assert!(matches!(
        store.call_blocking(|db| set_active_environment(db, Some(EnvironmentId(99)))),
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        store.call_blocking(move |db| delete_environment(db, env.id)),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn environments_are_listed_by_name() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            for name in ["b", "a"] {
                create_environment(db, name, &vars(&[]))?;
            }
            Ok(())
        })
        .unwrap();
    let names: Vec<_> = store
        .call_blocking(|db| environments(db))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["a", "b"]);
    let mut all = store.call_blocking(|db| environment_names(db)).unwrap();
    all.sort();
    assert_eq!(all, ["a", "b"]);
}
