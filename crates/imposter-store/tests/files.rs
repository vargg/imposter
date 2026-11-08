//! Collections as files behind the store: an edit writes only its own file, a file changed
//! on disk refuses the edit and rolls the whole command back, ids survive reopening, local
//! values never reach a file, the folder can be switched, and a version-7 database moves
//! its collections into the folder once, keeping History's links.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reads the folder the store writes

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use imposter_core::ids::RequestId;
use imposter_core::model::{SavedValues, Variables};
use imposter_core::variables::{Scope, SecretScope};
use imposter_store::schema::MIGRATIONS;
use imposter_store::{Store, StoreError, collections, environments, files, kv, scopes, secrets};
use support::{template, vars};

fn folder(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    files::format::read_dir(&dir.join("collections")).unwrap()
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("imposter.db")).unwrap()
}

#[test]
fn an_edit_writes_only_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let (a, _) = store
        .call_blocking(|db| {
            let api = collections::create_collection(db, "Shop API", "", &vars(&[("base", "x")]))?;
            let a = collections::create_request(db, api.id, None, &template("Create user"))?;
            let b = collections::create_request(db, api.id, None, &template("Delete user"))?;
            Ok((a.id.unwrap(), b.id.unwrap()))
        })
        .unwrap();
    let before = folder(dir.path());
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        [
            "shop-api/create-user.yaml",
            "shop-api/delete-user.yaml",
            "shop-api/imposter.yaml"
        ]
    );
    store
        .call_blocking(move |db| {
            let mut edited = collections::request(db, a)?.unwrap().template;
            edited.url = "https://example.com/users".into();
            collections::update_request(db, a, &edited)
        })
        .unwrap();
    let after = folder(dir.path());
    let changed: Vec<&String> = after
        .keys()
        .filter(|path| before.get(*path) != after.get(*path))
        .collect();
    assert_eq!(changed, ["shop-api/create-user.yaml"]);

    // A rename moves its one file.
    store
        .call_blocking(move |db| collections::rename_request(db, a, "Add user"))
        .unwrap();
    let renamed = folder(dir.path());
    assert!(renamed.contains_key("shop-api/add-user.yaml"));
    assert!(!renamed.contains_key("shop-api/create-user.yaml"));
    assert_eq!(renamed.len(), 3);
}

#[test]
fn a_file_changed_on_disk_refuses_the_edit_and_rolls_everything_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let a = store
        .call_blocking(|db| {
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            Ok(
                collections::create_request(db, api.id, None, &template("Login"))?
                    .id
                    .unwrap(),
            )
        })
        .unwrap();
    let file = dir.path().join("collections/api/login.yaml");
    let outside = std::fs::read_to_string(&file)
        .unwrap()
        .replace("GET", "POST");
    std::fs::write(&file, &outside).unwrap();

    let result = store.call_blocking(move |db| {
        kv::set(db, kv::Table::UiState, "touched", &true)?;
        let mut edited = collections::request(db, a)?.unwrap().template;
        edited.url = "https://edited".into();
        collections::update_request(db, a, &edited)
    });
    assert!(
        matches!(&result, Err(StoreError::Files(message)) if message.contains("api/login.yaml") && message.contains("changed on disk")),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        outside,
        "the outside edit stays"
    );
    let (url, touched) = store
        .call_blocking(move |db| {
            Ok((
                collections::request(db, a)?.unwrap().template.url,
                kv::get::<bool>(db, kv::Table::UiState, "touched")?,
            ))
        })
        .unwrap();
    assert_eq!(
        url, "https://example.com/Login",
        "the store kept what it had"
    );
    assert_eq!(touched, None, "SQLite rolled back with the files");
}

#[test]
fn ids_survive_reopening_and_a_folder_can_be_switched() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let (collection, request) = store
        .call_blocking(|db| {
            let api = collections::create_collection(db, "API", "", &Variables::new())?;
            let r = collections::create_request(db, api.id, None, &template("Login"))?;
            Ok((api.id, r.id.unwrap()))
        })
        .unwrap();
    store.close().unwrap();
    let store = open(dir.path());
    let read = store
        .call_blocking(move |db| collections::request(db, request))
        .unwrap()
        .unwrap();
    assert_eq!(
        (read.collection_id, read.template.name.as_str()),
        (Some(collection), "Login")
    );

    // Another folder: its collections replace these, and the profile remembers it.
    let other = tempfile::tempdir().unwrap();
    let path = other.path().to_owned();
    store
        .call_blocking(move |db| files::switch(db, &path))
        .unwrap();
    assert_eq!(
        store.call_blocking(collections::collections).unwrap(),
        [] as [collections::Collection; 0]
    );
    store
        .call_blocking(|db| collections::create_collection(db, "Other", "", &Variables::new()))
        .unwrap();
    assert!(other.path().join("other/imposter.yaml").exists());
    store.close().unwrap();
    let store = open(dir.path());
    let names: Vec<String> = store
        .call_blocking(collections::collections)
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["Other"]);
}

#[test]
fn local_values_stay_on_this_computer_and_saving_shares_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let (api, dev) = store
        .call_blocking(|db| {
            let api = collections::create_collection(db, "API", "", &vars(&[("id", "1")]))?;
            let dev =
                environments::create_environment(db, "Dev", &vars(&[("base", "http://dev")]))?;
            secrets::write(
                db,
                SecretScope::Environment(dev.id),
                &vars(&[("token", "tok-1234")]),
            )?;
            environments::set_globals(db, &vars(&[("global_only", "g-value")]))?;
            Ok((api.id, dev.id))
        })
        .unwrap();
    let before = folder(dir.path());

    // A capture changes the value in effect, never the file.
    store
        .call_blocking(move |db| {
            scopes::set_current(
                db,
                Scope::Collection(api),
                &[("id".into(), "captured-id-42".into())],
            )?;
            scopes::choose_value(db, Scope::Environment(dev), "base", "http://captured")
        })
        .unwrap();
    assert_eq!(folder(dir.path()), before, "a send never changes a file");
    let (collection, environment) = store
        .call_blocking(move |db| {
            Ok((
                collections::collection(db, api)?.unwrap().variables,
                environments::environment(db, dev)?.unwrap().variables,
            ))
        })
        .unwrap();
    assert_eq!(collection, vars(&[("id", "captured-id-42")]));
    assert_eq!(environment, vars(&[("base", "http://captured")]));

    // Saving makes what the screen shows the shared value, and clears the local one.
    store
        .call_blocking(move |db| {
            scopes::write(
                db,
                Scope::Collection(api),
                &vars(&[("id", "7")]),
                &SavedValues::default(),
            )
        })
        .unwrap();
    let text = String::from_utf8(folder(dir.path())["api/imposter.yaml"].clone()).unwrap();
    assert!(text.contains("id: \"7\""), "{text}");
    store
        .call_blocking(move |db| {
            scopes::set_current(db, Scope::Collection(api), &[("id".into(), "8".into())])?;
            scopes::write(
                db,
                Scope::Collection(api),
                &vars(&[("id", "7")]),
                &SavedValues::default(),
            )
        })
        .unwrap();
    let after_save = store
        .call_blocking(move |db| Ok(collections::collection(db, api)?.unwrap().variables))
        .unwrap();
    assert_eq!(after_save, vars(&[("id", "7")]), "no local value left over");

    // Not sharing: the file has the names, the values stay here.
    store
        .call_blocking(move |db| environments::set_share_values(db, dev, false))
        .unwrap();
    let text = String::from_utf8(folder(dir.path())["environments/dev.yaml"].clone()).unwrap();
    assert!(
        text.contains("base: \"\"") && text.contains("share_values: false"),
        "{text}"
    );
    let environment = store
        .call_blocking(move |db| Ok(environments::environment(db, dev)?.unwrap()))
        .unwrap();
    assert_eq!(environment.variables, vars(&[("base", "http://captured")]));
    assert!(!environment.share_values);

    let all: String = folder(dir.path())
        .values()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .collect();
    for private in ["tok-1234", "http://captured", "captured-id-42"] {
        assert!(!all.contains(private), "{private} reached the folder");
    }
    // Globals are the folder's shared variables, in their own file.
    let globals = String::from_utf8(folder(dir.path())["globals.yaml"].clone()).unwrap();
    assert_eq!(globals, "variables:\n  global_only: g-value\n");
}

#[test]
fn globals_live_in_the_folder_and_a_capture_changes_only_this_computer() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    store
        .call_blocking(|db| {
            environments::set_globals(db, &vars(&[("region", "eu")]))?;
            secrets::write(db, SecretScope::Global, &vars(&[("key", "k-secret-1")]))
        })
        .unwrap();
    let file = || String::from_utf8(folder(dir.path())["globals.yaml"].clone()).unwrap();
    assert_eq!(file(), "variables:\n  region: eu\n");

    store
        .call_blocking(|db| {
            scopes::set_current(db, Scope::Global, &[("region".into(), "us".into())])
        })
        .unwrap();
    assert_eq!(
        file(),
        "variables:\n  region: eu\n",
        "a capture leaves the file"
    );
    let in_effect = store.call_blocking(environments::globals).unwrap();
    assert_eq!(in_effect, vars(&[("region", "us")]));

    // Not shared: names only in the file, the values here.
    store
        .call_blocking(|db| environments::set_globals_shared(db, false))
        .unwrap();
    assert_eq!(file(), "share_values: false\nvariables:\n  region: \"\"\n");
    let (in_effect, shared) = store
        .call_blocking(|db| Ok((environments::globals(db)?, environments::globals_shared(db))))
        .unwrap();
    assert_eq!(in_effect, vars(&[("region", "us")]));
    assert!(!shared);
    assert!(!file().contains("k-secret-1"));

    // Another folder brings its own globals.
    let other = tempfile::tempdir().unwrap();
    let path = other.path().to_owned();
    store
        .call_blocking(move |db| files::switch(db, &path))
        .unwrap();
    assert!(
        store
            .call_blocking(environments::globals)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_environment_cannot_take_the_global_name_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for name in ["Global", "global", " GLOBAL "] {
        let result = store
            .call_blocking(move |db| environments::create_environment(db, name, &Variables::new()));
        assert!(
            matches!(result, Err(StoreError::Conflict(_))),
            "{name:?}: {result:?}"
        );
    }
}

#[test]
fn an_unreadable_folder_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("collections/api");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("imposter.yaml"), "format: nope\n").unwrap();
    let store = open(dir.path());
    let error = store.call_blocking(|db| Ok(db.folder_error())).unwrap();
    assert!(error.is_some_and(|e| e.contains("api/imposter.yaml")));
    let result =
        store.call_blocking(|db| collections::create_collection(db, "New", "", &Variables::new()));
    assert!(matches!(result, Err(StoreError::Files(_))), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(broken.join("imposter.yaml")).unwrap(),
        "format: nope\n"
    );
}

#[test]
fn a_version_7_database_moves_its_collections_into_the_folder_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("imposter.db");
    let v7 = &MIGRATIONS[..7];
    let old = Store::open_with(&path, v7).unwrap();
    old.call_blocking(|db| {
        db.execute_batch(
            r#"INSERT INTO collections (id, name, variables) VALUES (5, 'Shop API', '{"base":"x"}');
               INSERT INTO folders (id, collection_id, name, position) VALUES (8, 5, 'Users', 1);
               INSERT INTO requests (id, collection_id, folder_id, template)
                   VALUES (13, 5, 8, '{"name":"List users","method":"GET","url":"{{base}}/users","position":0}');
               INSERT INTO environments (id, name, variables, color) VALUES (3, 'Prod', '{"base":"p"}', 'red');
               INSERT INTO secrets (environment_id, name, value) VALUES (3, 'token', 'tok-5678');
               UPDATE globals SET variables = '{"region":"eu"}' WHERE id = 1;"#,
        )?;
        Ok(())
    })
    .unwrap();
    old.close().unwrap();

    let store = Store::open(&path).unwrap();
    let files = folder(dir.path());
    assert!(files.contains_key("shop-api/imposter.yaml"));
    assert!(files.contains_key("shop-api/users/list-users.yaml"));
    assert!(files.contains_key("environments/prod.yaml"));
    assert_eq!(
        String::from_utf8(files["globals.yaml"].clone()).unwrap(),
        "variables:\n  region: eu\n",
        "globals move with their values shared"
    );
    let (request, environment, secret) = store
        .call_blocking(|db| {
            Ok((
                collections::request(db, RequestId(13))?,
                environments::environment(db, imposter_core::ids::EnvironmentId(3))?,
                secrets::read(
                    db,
                    SecretScope::Environment(imposter_core::ids::EnvironmentId(3)),
                )?,
            ))
        })
        .unwrap();
    let request = request.expect("History's request id still opens the request");
    assert_eq!(request.template.name, "List users");
    assert_eq!(request.collection_id.map(|c| c.0), Some(5));
    let environment = environment.unwrap();
    assert_eq!(environment.color, Some(imposter_core::model::EnvColor::Red));
    assert_eq!(
        secret,
        vars(&[("token", "tok-5678")]),
        "its secrets follow it"
    );

    // Once: a second open reads the folder and moves nothing again.
    store
        .call_blocking(|db| collections::delete_request(db, RequestId(13)))
        .unwrap();
    store.close().unwrap();
    let store = Store::open(&path).unwrap();
    assert!(
        store
            .call_blocking(|db| collections::request(db, RequestId(13)))
            .unwrap()
            .is_none()
    );
}
