//! Import and export through the real dialogs. File dialogs are scripted: native ones cannot be
//! driven by tests.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{ParamItem, RequestTemplate, Variables};
use imposter_app::formats::native;
use imposter_app::store::{collections, environments};
use imposter_ui::app::Action;
use support::{click, click_button, collection, request, settle, start, start_with, type_into};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn import(app: &mut support::App, answers: Vec<Option<PathBuf>>) {
    app.harness.state_mut().script_file_dialogs(answers);
    click(&mut app.harness, "New");
    click(&mut app.harness, "Import Collection…");
    settle(&mut app.harness);
}

fn collection_names(app: &support::App) -> Vec<String> {
    app.harness
        .state()
        .store()
        .call_blocking(|db| collections::collections(db))
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

fn import_button_enabled(app: &support::App) -> bool {
    !app.harness
        .get_by_role_and_label(Role::Button, "Import")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn a_native_file_is_reviewed_then_imported_and_shown() {
    let mut app = start();
    import(
        &mut app,
        vec![Some(fixtures().join("native/full.poster.json"))],
    );

    app.harness.get_by_label("Review collection import");
    app.harness.get_by_label("New collection: Full collection");
    app.harness.get_by_label("New environments: Dev, Empty env");
    assert_eq!(
        collection_names(&app),
        Vec::<String>::new(),
        "nothing is written before review"
    );
    click_button(&mut app.harness, "Import");
    settle(&mut app.harness);

    assert_eq!(collection_names(&app), ["Full collection"]);
    let state = &app.harness.state().state;
    assert_eq!(
        state
            .collections
            .iter()
            .map(|c| &c.name)
            .collect::<Vec<_>>(),
        ["Full collection"]
    );
    assert!(state.tree.is_some(), "the imported collection is selected");
    assert_eq!(
        state
            .environments
            .iter()
            .map(|e| &e.name)
            .collect::<Vec<_>>(),
        ["Dev", "Empty env"]
    );
}

#[test]
fn postman_imports_offer_companions_and_require_consent_for_losses() {
    let mut app = start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("scripted.postman_collection.json");
    std::fs::write(
        &file,
        serde_json::json!({
            "info": {"name": "Scripted", "schema": imposter_app::formats::postman::SCHEMA_URL},
            "item": [{"name": "R", "event": [{"listen": "test"}],
                      "request": {"method": "GET", "url": "http://h/"}}]
        })
        .to_string(),
    )
    .unwrap();
    let environment = dir.path().join("dev.postman_environment.json");
    std::fs::write(
        &environment,
        r#"{"name": "Postman Dev", "values": [{"key": "base", "value": "http://h"}]}"#,
    )
    .unwrap();
    import(&mut app, vec![Some(file), Some(environment), None]);

    app.harness.get_by_label("Companion files");
    click(&mut app.harness, "Yes");
    settle(&mut app.harness);
    app.harness
        .get_by_label("Scripted/R: scripts will be removed");
    app.harness.get_by_label("New environments: Postman Dev");
    assert!(!import_button_enabled(&app), "losses need consent");

    click(
        &mut app.harness,
        "Remove the listed unsupported execution features",
    );
    assert!(import_button_enabled(&app));
    click_button(&mut app.harness, "Import");
    settle(&mut app.harness);

    assert_eq!(collection_names(&app), ["Scripted"]);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| environments::environments(db))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["Postman Dev"]);
}

#[test]
fn a_cancelled_review_or_picker_writes_nothing() {
    let mut app = start();
    import(&mut app, vec![None]);
    assert!(app.harness.state().state.dialog.is_none());

    import(&mut app, vec![Some(fixtures().join("postman-v2.1.json"))]);
    click(&mut app.harness, "No");
    settle(&mut app.harness);
    app.harness.get_by_label("New collection: Local API");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);
    assert_eq!(collection_names(&app), Vec::<String>::new());
}

#[test]
fn a_bad_file_reports_why_it_cannot_be_imported() {
    let mut app = start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("list.poster.json");
    std::fs::write(&file, "[1, 2]").unwrap();
    import(&mut app, vec![Some(file)]);

    app.harness.get_by_label("Import failed");
    assert_eq!(collection_names(&app), Vec::<String>::new());
}

#[test]
fn a_native_export_includes_unsaved_edits_and_opted_in_scopes() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h/a");
        store
            .call_blocking(|db| {
                environments::create_environment(db, "Dev", &Variables::new())?;
                environments::create_environment(db, "Prod", &Variables::new())?;
                environments::set_globals(db, &[("g".to_owned(), "1".to_owned())].into())
            })
            .unwrap();
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/edited");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("api.poster.json");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(out.clone())]);

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Export Collection…");
    app.harness.get_by_label("Export collection");
    click(&mut app.harness, "Dev");
    click_button(&mut app.harness, "Export");
    settle(&mut app.harness);

    app.harness
        .get_by_label(&format!("Saved {}", out.display()));
    let document = native::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(document.collection.requests[0].url, "http://h/a/edited");
    assert_eq!(
        document
            .environments
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["Dev"]
    );
    assert!(document.globals.is_empty(), "globals are opt-in");
}

#[test]
fn a_postman_export_with_limitations_asks_then_writes_a_bundle() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        store
            .call_blocking(move |db| {
                let template = RequestTemplate {
                    name: "Cookies".into(),
                    url: "http://h/".into(),
                    cookies: vec![ParamItem::new("a", "1")],
                    ..RequestTemplate::default()
                };
                collections::create_request(db, api.id, None, &template)?;
                environments::create_environment(db, "Dev", &Variables::new())
            })
            .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("export");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(bundle.clone())]);

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Export Collection…");
    click(&mut app.harness, "Postman v2.1");
    click(&mut app.harness, "Dev");
    click_button(&mut app.harness, "Export");
    settle(&mut app.harness);
    app.harness.get_by_label("Postman export limitations");
    assert!(
        !bundle.exists(),
        "nothing is written before the warnings are accepted"
    );
    click_button(&mut app.harness, "Export");
    settle(&mut app.harness);

    app.harness
        .get_by_label(&format!("Saved {}", bundle.display()));
    let mut files: Vec<String> = std::fs::read_dir(&bundle)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "collection.postman_collection.json",
            "environment-1.postman_environment.json"
        ]
    );
}

#[test]
fn a_lone_postman_collection_is_one_file() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h/a");
    });
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("api.postman_collection.json");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(out.clone())]);

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Export Collection…");
    click(&mut app.harness, "Postman v2.1");
    click_button(&mut app.harness, "Export");
    settle(&mut app.harness);

    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(data["info"]["name"], "API");
}

#[test]
fn exporting_needs_a_collection_and_reports_write_failures() {
    let mut app = start();
    app.harness.state_mut().handle(Action::Export, 0.0);
    app.harness.run_steps(2);
    app.harness.get_by_label("Select a collection first");

    let mut app = start_with(|store| {
        collection(store, "API");
    });
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("missing").join("api.poster.json");
    app.harness.state_mut().script_file_dialogs([Some(out)]);
    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Export Collection…");
    click_button(&mut app.harness, "Export");
    settle(&mut app.harness);
    app.harness.get_by_label("Export failed");
}

#[test]
fn importing_over_unsaved_changes_asks_first() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h/a");
        store
            .call_blocking(|db| {
                imposter_app::store::kv::set_app_settings(
                    db,
                    &imposter_app::core::settings::AppSettings {
                        autosave: false,
                        ..Default::default()
                    },
                )
            })
            .unwrap();
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/edited");
    import(
        &mut app,
        vec![Some(fixtures().join("native/minimal.poster.json"))],
    );
    click_button(&mut app.harness, "Import");
    settle(&mut app.harness);

    app.harness
        .get_by_label("Save changes to \"Get\" before continuing?");
    assert_eq!(collection_names(&app), ["API"]);
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(collection_names(&app).len(), 2);
}

#[test]
fn the_new_native_extension_skips_postman_companions() {
    let mut app = start();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.imposter.json");
    std::fs::copy(fixtures().join("native/full.poster.json"), &path).unwrap();
    import(&mut app, vec![Some(path)]);
    app.harness.get_by_label("Review collection import");
    click_button(&mut app.harness, "Import");
    settle(&mut app.harness);
    assert_eq!(collection_names(&app), ["Full collection"]);
}
