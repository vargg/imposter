//! Secrets: their own table, global and per environment, written whole or one at a time,
//! gone with their environment, and read into the variable context beside the variables.
#![cfg(test)]

mod support;

use imposter_core::variables::SecretScope;
use imposter_store::{environments, scopes, secrets};
use support::{store, vars};

#[test]
fn each_scope_keeps_its_own_secrets_in_order() {
    let (_dir, store) = store();
    let (dev, prod) = store
        .call_blocking(|db| {
            Ok((
                environments::create_environment(db, "Dev", &vars(&[]))?.id,
                environments::create_environment(db, "Prod", &vars(&[]))?.id,
            ))
        })
        .unwrap();
    for (scope, value) in [
        (SecretScope::Global, "global-token"),
        (SecretScope::Environment(dev), "dev-token"),
        (SecretScope::Environment(prod), "prod-token"),
    ] {
        store
            .call_blocking(move |db| {
                secrets::write(db, scope, &vars(&[("token", value), ("key", "k-1234")]))
            })
            .unwrap();
        let read = store
            .call_blocking(move |db| secrets::read(db, scope))
            .unwrap();
        assert_eq!(read, vars(&[("token", value), ("key", "k-1234")]));
    }
    // Writing replaces the whole scope; setting changes one value in place and adds new
    // ones last.
    store
        .call_blocking(move |db| {
            secrets::write(db, SecretScope::Global, &vars(&[("only", "o-1234")]))?;
            secrets::set(
                db,
                SecretScope::Environment(dev),
                &[
                    ("token".into(), "dev-new".into()),
                    ("fresh".into(), "f-1234".into()),
                ],
            )
        })
        .unwrap();
    let (global, dev_secrets) = store
        .call_blocking(move |db| {
            Ok((
                secrets::read(db, SecretScope::Global)?,
                secrets::read(db, SecretScope::Environment(dev))?,
            ))
        })
        .unwrap();
    assert_eq!(global, vars(&[("only", "o-1234")]));
    assert_eq!(
        dev_secrets,
        vars(&[("token", "dev-new"), ("key", "k-1234"), ("fresh", "f-1234")])
    );
    let names = store.call_blocking(|db| secrets::names(db)).unwrap();
    assert_eq!(
        names.first(),
        Some(&(SecretScope::Global, "only".to_owned()))
    );
    assert_eq!(names.len(), 6);

    // An environment's secrets go with it; nothing else does.
    store
        .call_blocking(move |db| environments::delete_environment(db, prod))
        .unwrap();
    let names = store.call_blocking(|db| secrets::names(db)).unwrap();
    assert!(
        names
            .iter()
            .all(|(scope, _)| *scope != SecretScope::Environment(prod))
    );
    assert_eq!(names.len(), 4);
}

#[test]
fn a_global_name_is_unique_and_variables_never_hold_secrets() {
    let (_dir, store) = store();
    let duplicate = store.call_blocking(|db| {
        secrets::set(
            db,
            SecretScope::Global,
            &[("token".into(), "a-1234".into())],
        )?;
        db.execute(
            "INSERT INTO secrets (environment_id, name, value) VALUES (NULL, 'token', 'b')",
            [],
        )?;
        Ok(())
    });
    assert!(duplicate.is_err(), "two global secrets of one name");
    let globals = store
        .call_blocking(|db| scopes::read(db, imposter_core::variables::Scope::Global))
        .unwrap();
    assert!(globals.0.is_empty());
}

#[test]
fn the_variable_context_brings_the_secrets_that_apply() {
    let (_dir, store) = store();
    let context = store
        .call_blocking(|db| {
            let dev = environments::create_environment(db, "Dev", &vars(&[]))?.id;
            secrets::write(db, SecretScope::Global, &vars(&[("token", "global-1")]))?;
            secrets::write(
                db,
                SecretScope::Environment(dev),
                &vars(&[("token", "dev-1")]),
            )?;
            environments::variable_context_for(db, Some(dev))
        })
        .unwrap();
    assert_eq!(context.global_secrets, vars(&[("token", "global-1")]));
    assert_eq!(
        context.environment.unwrap().secrets,
        vars(&[("token", "dev-1")])
    );
}
