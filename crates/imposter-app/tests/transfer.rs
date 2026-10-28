//! Import and export end to end: files, previews, atomic imports, Python compatibility.
#![cfg(test)]

use std::path::{Path, PathBuf};

use imposter_app::transfer::{
    ExportScope, TransferError, export_native, import, postman_export, read_preview, write_postman,
};
use imposter_core::model::{BodyType, Request, RequestTemplate, Variables};
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
    assert_eq!(exported, expected);

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
