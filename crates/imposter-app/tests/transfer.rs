//! Import and export end to end: files, previews, atomic imports, Python compatibility.
#![cfg(test)]

use std::path::{Path, PathBuf};

use imposter_app::transfer::{
    ExportScope, ImportKind, TransferError, detect, export_native, import, postman_export,
    read_preview, write_postman,
};
use imposter_core::model::{BodyType, Request, RequestTemplate, Variables};
use imposter_core::prepare::{PreparedBody, prepare};
use imposter_core::variables::{ResolveError, resolve_template};
use imposter_store::{Store, collections, environments};
use imposter_transfer::native;
use serde_json::Value;

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    (dir, store)
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Positions and folder order, which exports normalize when siblings tie (as the Python app's
/// export does), set aside to compare content.
fn without_order(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("position");
            if let Some(Value::Array(folders)) = map.get_mut("folders") {
                folders.sort_by_key(|f| f["name"].as_str().unwrap_or_default().to_owned());
            }
            if let Some(Value::Array(environments)) = map.get_mut("environments") {
                environments.sort_by_key(|e| e["name"].as_str().unwrap_or_default().to_owned());
            }
            map.values_mut().for_each(without_order);
        }
        Value::Array(items) => items.iter_mut().for_each(without_order),
        _ => {}
    }
}

async fn round_trip(store: &Store, path: PathBuf, out: PathBuf) {
    let preview = read_preview(&store.handle(), path, Vec::new())
        .await
        .unwrap();
    let names = preview.environment_names.clone();
    let id = import(&store.handle(), preview, false).await.unwrap();
    let environment_ids: Vec<_> = store
        .call(|db| environments::environments(db))
        .await
        .unwrap()
        .into_iter()
        .filter(|e| names.contains(&e.name))
        .map(|e| e.id)
        .collect();
    let scope = ExportScope {
        collection: Some(id),
        environments: environment_ids,
        globals: true,
        draft: None,
    };
    export_native(&store.handle(), scope, out).await.unwrap();
}

#[tokio::test]
async fn a_python_export_imports_and_exports_again_with_the_same_content() {
    let original = fixtures().join("native/full.poster.json");
    let (dir, store) = open();
    let first = dir.path().join("first.poster.json");
    round_trip(&store, original.clone(), first.clone()).await;

    let mut exported = json(&first);
    let mut expected = json(&original);
    without_order(&mut exported);
    without_order(&mut expected);
    assert_eq!(
        (&exported["format"], &exported["version"]),
        (&Value::from("imposter"), &Value::from(2)),
        "Imposter writes version 2"
    );
    expected["format"] = Value::from("imposter");
    expected["version"] = Value::from(2);
    assert_eq!(exported, expected, "and keeps everything version 1 carried");

    // A second trip through a fresh database changes nothing at all.
    let (dir2, store2) = open();
    let second = dir2.path().join("second.poster.json");
    round_trip(&store2, first.clone(), second.clone()).await;
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
}

#[tokio::test]
async fn imports_never_overwrite_and_report_global_conflicts() {
    let (_dir, store) = open();
    store
        .call(|db| {
            collections::create_collection(db, "Full collection", "", &Variables::new())?;
            environments::create_environment(db, "Dev", &Variables::new())?;
            environments::set_globals(db, &vars(&[("token", "mine")]))
        })
        .await
        .unwrap();

    let preview = read_preview(
        &store.handle(),
        fixtures().join("native/full.poster.json"),
        Vec::new(),
    )
    .await
    .unwrap();
    assert_eq!(preview.collection_name, "Full collection (2)");
    assert_eq!(preview.environment_names, ["Dev (2)", "Empty env"]);
    assert_eq!(
        preview.warnings,
        ["Global 'token': existing value retained"]
    );
    import(&store.handle(), preview, false).await.unwrap();

    let globals = store.call(|db| environments::globals(db)).await.unwrap();
    assert_eq!(globals["token"], "mine");
    assert_eq!(globals["file"], "upload.bin");
    let names: Vec<String> = store
        .call(|db| collections::collections(db))
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["Full collection", "Full collection (2)"]);
}

#[tokio::test]
async fn exports_overlay_the_unsaved_draft_and_make_positions_unique() {
    let (dir, store) = open();
    let (collection, saved) = store
        .call(|db| {
            let c = collections::create_collection(db, "API", "", &Variables::new())?;
            let template = RequestTemplate {
                name: "Same".into(),
                position: 0,
                ..RequestTemplate::default()
            };
            let a = collections::create_request(db, c.id, None, &template)?;
            collections::create_request(db, c.id, None, &template)?;
            Ok((c.id, a))
        })
        .await
        .unwrap();
    let mut edited = saved.clone();
    edited.template.url = "http://unsaved".into();
    let draft = Request {
        template: RequestTemplate {
            name: "Draft".into(),
            position: 0,
            ..RequestTemplate::default()
        },
        ..Request::default()
    };
    let out = dir.path().join("out.poster.json");

    export_native(
        &store.handle(),
        ExportScope {
            collection: Some(collection),
            draft: Some(edited),
            ..ExportScope::default()
        },
        out.clone(),
    )
    .await
    .unwrap();
    export_native(
        &store.handle(),
        ExportScope {
            collection: Some(collection),
            draft: Some(draft),
            ..ExportScope::default()
        },
        dir.path().join("draft.poster.json"),
    )
    .await
    .unwrap();

    let document = native::decode(&std::fs::read(&out).unwrap()).unwrap();
    let requests = &document.collection.requests;
    assert_eq!(
        requests.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(requests[0].url, "http://unsaved");
    let with_draft =
        native::decode(&std::fs::read(dir.path().join("draft.poster.json")).unwrap()).unwrap();
    assert!(
        with_draft
            .collection
            .requests
            .iter()
            .any(|r| r.name == "Draft")
    );
    assert_eq!(with_draft.collection.requests.len(), 3);
}

#[tokio::test]
async fn postman_imports_need_consent_for_losses_that_change_requests() {
    let (dir, store) = open();
    let path = dir.path().join("scripted.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "info": {"name": "Scripted", "schema": imposter_transfer::postman::SCHEMA_URL},
            "item": [{"name": "R", "event": [{"listen": "test"}], "request": {"method": "GET", "url": "http://h/"}}]
        })
        .to_string(),
    )
    .unwrap();

    let preview = read_preview(&store.handle(), path, Vec::new())
        .await
        .unwrap();
    assert!(preview.requires_removal);
    assert_eq!(preview.warnings, ["Scripted/R: scripts will be removed"]);
    assert!(matches!(
        import(&store.handle(), preview.clone(), false).await,
        Err(TransferError::ConsentRequired)
    ));
    assert_eq!(
        store.call(|db| collections::collections(db)).await.unwrap(),
        []
    );

    import(&store.handle(), preview, true).await.unwrap();
    assert_eq!(
        store
            .call(|db| collections::collections(db))
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn postman_files_import_with_companions() {
    let (dir, store) = open();
    let environment = dir.path().join("dev.postman_environment.json");
    std::fs::write(
        &environment,
        r#"{"name": "Postman Dev", "values": [{"key": "base", "value": "http://h"}]}"#,
    )
    .unwrap();

    let preview = read_preview(
        &store.handle(),
        fixtures().join("postman-v2.1.json"),
        vec![environment],
    )
    .await
    .unwrap();
    assert_eq!(preview.collection_name, "Local API");
    assert_eq!(preview.environment_names, ["Postman Dev"]);
    let id = import(&store.handle(), preview, false).await.unwrap();

    let tree = store
        .call(move |db| collections::tree(db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tree.folders.len(), 1);
    assert_eq!(tree.requests.len(), 5);
}

#[tokio::test]
async fn bad_files_are_rejected_before_anything_is_written() {
    let (dir, store) = open();
    let array = dir.path().join("array.json");
    std::fs::write(&array, "[1, 2]").unwrap();
    let companion = dir.path().join("env.json");
    std::fs::write(&companion, r#"{"values": []}"#).unwrap();

    assert!(matches!(
        read_preview(&store.handle(), array, Vec::new()).await,
        Err(TransferError::NotAnObject)
    ));
    let native_with_companion = read_preview(
        &store.handle(),
        fixtures().join("native/minimal.poster.json"),
        vec![companion],
    )
    .await;
    assert!(matches!(
        native_with_companion,
        Err(TransferError::NativeCompanions)
    ));
    let missing = read_preview(&store.handle(), dir.path().join("missing.json"), Vec::new()).await;
    assert!(missing.is_err());
    assert_eq!(
        store.call(|db| collections::collections(db)).await.unwrap(),
        []
    );
}

#[tokio::test]
async fn postman_exports_write_a_file_or_a_new_directory_with_companions() {
    let (dir, store) = open();
    let (collection, environment) = store
        .call(|db| {
            let c = collections::create_collection(db, "API", "", &vars(&[("base", "http://h")]))?;
            let template = RequestTemplate {
                name: "Get".into(),
                url: "http://h/x".into(),
                body_type: BodyType::None,
                ..RequestTemplate::default()
            };
            collections::create_request(db, c.id, None, &template)?;
            let env = environments::create_environment(db, "Dev", &vars(&[("a", "1")]))?;
            environments::set_globals(db, &vars(&[("g", "1")]))?;
            Ok((c.id, env.id))
        })
        .await
        .unwrap();

    let (document, data, warnings) = postman_export(
        &store.handle(),
        ExportScope {
            collection: Some(collection),
            ..ExportScope::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(warnings, Vec::<String>::new());
    let single = dir.path().join("api.postman_collection.json");
    write_postman(document, data, single.clone()).await.unwrap();
    assert_eq!(json(&single)["info"]["name"], "API");

    let scope = ExportScope {
        collection: Some(collection),
        environments: vec![environment],
        globals: true,
        draft: None,
    };
    let (document, data, _) = postman_export(&store.handle(), scope).await.unwrap();
    let bundle = dir.path().join("bundle");
    write_postman(document.clone(), data.clone(), bundle.clone())
        .await
        .unwrap();
    let mut files: Vec<String> = std::fs::read_dir(&bundle)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "collection.postman_collection.json",
            "environment-1.postman_environment.json",
            "globals.postman_globals.json"
        ]
    );
    assert_eq!(
        json(&bundle.join("environment-1.postman_environment.json"))["name"],
        "Dev"
    );

    let again = write_postman(document, data, bundle).await;
    assert!(
        again.is_err(),
        "an existing destination is never overwritten"
    );
    let leftovers = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".imposter-export")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[tokio::test]
async fn files_are_recognised_by_content_not_name() {
    let (dir, _store) = open();
    let openapi_json = dir.path().join("collection.json");
    std::fs::write(&openapi_json, r#"{"openapi": "3.1.0", "paths": {}}"#).unwrap();
    let plain_yaml = dir.path().join("config.yaml");
    std::fs::write(&plain_yaml, "name: not an api\n").unwrap();
    for (path, kind) in [
        (
            fixtures().join("native/full.poster.json"),
            ImportKind::Native,
        ),
        (fixtures().join("postman-v2.1.json"), ImportKind::Postman),
        (
            fixtures().join("openapi/petstore.yaml"),
            ImportKind::OpenApi,
        ),
        (openapi_json, ImportKind::OpenApi),
    ] {
        assert_eq!(detect(path.clone()).await.unwrap(), kind, "{path:?}");
    }
    assert!(ImportKind::Postman.takes_companions());
    assert!(!ImportKind::OpenApi.takes_companions());
    assert!(!ImportKind::Native.takes_companions());
    assert!(
        matches!(detect(plain_yaml).await, Err(TransferError::Json(_))),
        "YAML that is not OpenAPI reports why it is not JSON"
    );
}

#[tokio::test]
async fn openapi_files_import_as_one_new_collection_without_consent() {
    let (dir, store) = open();
    let path = fixtures().join("openapi/petstore.yaml");
    let preview = read_preview(&store.handle(), path.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(preview.collection_name, "Pet Store");
    assert_eq!(preview.environment_names, Vec::<String>::new());
    assert!(!preview.requires_removal, "generating loses nothing");
    assert_eq!(preview.warnings.len(), 3, "{:?}", preview.warnings);
    let first = import(&store.handle(), preview, false).await.unwrap();

    let again = read_preview(&store.handle(), path.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(
        again.collection_name, "Pet Store (2)",
        "re-imports never merge"
    );
    import(&store.handle(), again, false).await.unwrap();

    let tree = store
        .call(move |db| collections::tree(db, first))
        .await
        .unwrap()
        .unwrap();
    let folders: Vec<_> = tree.folders.iter().map(|f| f.name.clone()).collect();
    assert_eq!(folders, ["store", "pets"]);
    assert_eq!(tree.requests.len(), 8);
    assert_eq!(
        tree.collection.variables.get("baseUrl").map(String::as_str),
        Some("https://eu.pets.example/v1")
    );

    let companion = dir.path().join("env.json");
    std::fs::write(&companion, r#"{"values": []}"#).unwrap();
    assert!(matches!(
        read_preview(&store.handle(), path, vec![companion]).await,
        Err(TransferError::OpenApiCompanions)
    ));
}

#[tokio::test]
async fn imported_openapi_requests_send_once_their_variables_are_set() {
    let (_dir, store) = open();
    let preview = read_preview(
        &store.handle(),
        fixtures().join("openapi/petstore.yaml"),
        Vec::new(),
    )
    .await
    .unwrap();
    let mut values = preview.document.collection.variables.clone();
    values.insert("api_key".into(), "k".into());
    values.insert("token".into(), "t".into());
    let by_name = |name: &str| {
        preview
            .document
            .requests()
            .into_iter()
            .find(|r| r.name == name)
            .unwrap()
            .clone()
    };
    let sent =
        |name: &str| prepare(&resolve_template(&by_name(name), &values).unwrap(), true).unwrap();

    let list = sent("List pets");
    assert_eq!(list.method, "GET");
    assert_eq!(list.url, "https://eu.pets.example/v1/pets?limit=20");
    assert_eq!(list.headers, [("X-API-Key".to_owned(), "k".to_owned())]);

    let delete = sent("Delete a pet");
    assert_eq!(delete.url, "https://eu.pets.example/v1/pets/42");
    assert_eq!(
        delete.headers,
        [("Authorization".to_owned(), "Bearer t".to_owned())]
    );

    let create = sent("Create a pet");
    let PreparedBody::Bytes(body) = create.body else {
        panic!("a JSON body");
    };
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["name"], "Rex");
    assert!(
        create
            .headers
            .contains(&("Content-Type".to_owned(), "application/json".to_owned()))
    );

    assert!(
        matches!(
            resolve_template(&by_name("Upload owner documents"), &values),
            Err(ResolveError::Unresolved(names)) if names.iter().eq(["ownerId"])
        ),
        "a path parameter without an example stops the send"
    );
}

#[tokio::test]
async fn saved_values_travel_with_their_scopes_and_existing_globals_keep_theirs() {
    use imposter_core::model::SavedValues;
    use imposter_core::variables::Scope;
    use imposter_store::scopes;

    let saved = |values: &[&str]| {
        let mut saved = SavedValues::default();
        saved.set("user", values.iter().map(|v| (*v).to_owned()));
        saved
    };
    let (dir, store) = open();
    let (api, dev) = store
        .call(move |db| {
            let api = collections::create_collection(db, "API", "", &vars(&[("user", "42")]))?;
            scopes::write(
                db,
                Scope::Collection(api.id),
                &vars(&[("user", "77")]),
                &saved(&["42", "77"]),
            )?;
            let dev = environments::create_environment(db, "Dev", &vars(&[]))?;
            scopes::write(
                db,
                Scope::Environment(dev.id),
                &vars(&[("user", "42")]),
                &saved(&["42", "91"]),
            )?;
            let mut globals = saved(&["1", "2"]);
            globals.set("region", ["eu".to_owned(), "us".to_owned()]);
            scopes::write(
                db,
                Scope::Global,
                &vars(&[("user", "1"), ("region", "eu")]),
                &globals,
            )?;
            Ok((api.id, dev.id))
        })
        .await
        .unwrap();
    let out = dir.path().join("api.imposter.json");
    let scope = ExportScope {
        collection: Some(api),
        environments: vec![dev],
        globals: true,
        draft: None,
    };
    export_native(&store.handle(), scope, out.clone())
        .await
        .unwrap();

    let raw = std::fs::read_to_string(&out).unwrap();
    let document = native::decode(raw.as_bytes()).unwrap();
    assert_eq!(document.collection.variables, vars(&[("user", "77")]));
    assert_eq!(
        document.collection.saved_values.values("user"),
        ["42", "77"]
    );
    assert_eq!(
        document.environments[0].saved_values.values("user"),
        ["42", "91"]
    );
    assert_eq!(document.globals, vars(&[("user", "1"), ("region", "eu")]));
    assert_eq!(document.saved_globals.values("region"), ["eu", "us"]);

    // Into a database whose globals already define `user` with other saved values.
    let (_dir2, store2) = open();
    store2
        .call(move |db| {
            scopes::write(
                db,
                Scope::Global,
                &vars(&[("user", "mine")]),
                &saved(&["mine", "x"]),
            )
        })
        .await
        .unwrap();
    let preview = read_preview(&store2.handle(), out, Vec::new())
        .await
        .unwrap();
    let id = import(&store2.handle(), preview, false).await.unwrap();
    let (collection, environment, globals) = store2
        .call(move |db| {
            let dev = environments::environments(db)?
                .into_iter()
                .find(|e| e.name == "Dev")
                .unwrap()
                .id;
            Ok((
                scopes::saved_values(db, Scope::Collection(id))?,
                scopes::saved_values(db, Scope::Environment(dev))?,
                scopes::read(db, Scope::Global)?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(collection.values("user"), ["42", "77"]);
    assert_eq!(environment.values("user"), ["42", "91"]);
    assert_eq!(globals.0, vars(&[("user", "mine"), ("region", "eu")]));
    assert_eq!(
        globals.1.values("user"),
        ["mine", "x"],
        "existing globals keep theirs"
    );
    assert_eq!(
        globals.1.values("region"),
        ["eu", "us"],
        "new globals bring theirs"
    );
}
