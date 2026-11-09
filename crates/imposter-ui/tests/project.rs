//! Collections as files in the app: a save lands in the request's file, a file changed
//! outside the app refuses the save and keeps the edit, Settings switches the collections
//! folder, and an environment can keep its values on this computer.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reads and edits the collections folder

mod support;

use egui_kittest::kittest::Queryable;
use imposter_ui::app::Screen;
use support::{
    click, click_button, collection, request, save_state, settle, start_with, type_into,
};

fn file(app: &support::App, path: &str) -> String {
    std::fs::read_to_string(app.dir.path().join("collections").join(path)).unwrap()
}

#[test]
fn a_save_lands_in_the_request_file_and_an_outside_change_refuses_it() {
    let mut app = start_with(|store| {
        let api = collection(store, "Shop API");
        request(store, api.id, "Create user", "http://localhost/users");
    });
    // Without a reload in between, as when the outside change came just before the save.
    app.harness.state_mut().stop_watching();
    click(&mut app.harness, "GET Create user");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Request URL", "/v2");
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert!(
        file(&app, "shop-api/create-user.yaml").contains("url: http://localhost/users/v2"),
        "{}",
        file(&app, "shop-api/create-user.yaml")
    );

    // Someone edits the file (a git pull); the next save refuses rather than overwrite it.
    let path = app.dir.path().join("collections/shop-api/create-user.yaml");
    let outside = std::fs::read_to_string(&path)
        .unwrap()
        .replace("method: GET", "method: POST");
    std::fs::write(&path, &outside).unwrap();
    type_into(&mut app.harness, "Request URL", "/again");
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert_eq!(save_state(&app.harness, "Retry save"), "Save failed");
    app.harness.get_by_label_contains("changed on disk");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        outside,
        "the outside edit stays"
    );
    let url = app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .url
        .clone();
    assert_eq!(url, "http://localhost/users/v2/again", "the edit is kept");
}

#[test]
fn settings_switches_the_collections_folder() {
    let mut app = start_with(|store| {
        collection(store, "Shop API");
    });
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join("admin")).unwrap();
    std::fs::write(
        other.path().join("admin/imposter.yaml"),
        "format: imposter-project\nversion: 1\nid: c-admin\nname: Admin\n",
    )
    .unwrap();
    std::fs::write(
        other.path().join("admin/stats.yaml"),
        "id: r-stats\nname: Stats\nurl: http://localhost/stats\n",
    )
    .unwrap();

    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Settings);
    app.harness
        .get_by_label_contains(&app.dir.path().join("collections").display().to_string());
    app.harness
        .state_mut()
        .script_file_dialogs([Some(other.path().to_owned())]);
    click_button(&mut app.harness, "Change collections folder");
    settle(&mut app.harness);
    let state = &app.harness.state().state;
    assert_eq!(state.collections_folder, other.path());
    let names: Vec<&str> = state.collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Admin"]);
    click(&mut app.harness, "Requests");
    settle(&mut app.harness);
    app.harness.get_by_label("GET Stats");
}

#[test]
fn an_environment_can_keep_its_values_on_this_computer() {
    let mut app = start_with(|_| {});
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "New environment");
    app.harness.run_steps(2);
    support::text_input(&app.harness, "Name").type_text("Prod");
    app.harness.run_steps(1);
    click(&mut app.harness, "Share values");
    click_button(&mut app.harness, "Create");
    settle(&mut app.harness);
    let shared = app
        .harness
        .state()
        .state
        .environments
        .iter()
        .find(|e| e.name == "Prod")
        .map(|e| e.share_values);
    assert_eq!(shared, Some(false));
    assert!(file(&app, "environments/prod.yaml").contains("share_values: false"));
    assert!(
        app.harness.query_by_label("Share values").is_none(),
        "the dialog closed"
    );
}

#[test]
fn the_global_page_can_keep_its_values_on_this_computer() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                imposter_app::store::environments::set_globals(
                    db,
                    &[("region".to_owned(), "eu".to_owned())]
                        .into_iter()
                        .collect(),
                )
            })
            .unwrap();
    });
    assert_eq!(file(&app, "globals.yaml"), "variables:\n  region: eu\n");
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app.harness
        .get_by_label_contains("Saved to globals.yaml in the collections folder");

    click(&mut app.harness, "Variable actions");
    click(&mut app.harness, "Share values");
    settle(&mut app.harness);
    assert!(!app.harness.state().state.globals_shared);
    assert_eq!(
        file(&app, "globals.yaml"),
        "share_values: false\nvariables:\n  region: \"\"\n"
    );
    app.harness
        .get_by_label_contains("globals.yaml lists the names; the values stay on this computer");
    let in_effect = app
        .harness
        .state()
        .store()
        .call_blocking(imposter_app::store::environments::globals)
        .unwrap();
    assert_eq!(in_effect.get("region").map(String::as_str), Some("eu"));
}

/// "Shop API" with "Create user", open in the editor, and autosave off so an edit stays
/// unsaved until the test says.
fn open_request() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "Shop API");
        request(store, api.id, "Create user", "http://localhost/users");
    });
    app.harness.state_mut().state.settings.autosave = false;
    click(&mut app.harness, "GET Create user");
    settle(&mut app.harness);
    app
}

fn change_on_disk(app: &support::App, path: &str, from: &str, to: &str) {
    let file = app.dir.path().join("collections").join(path);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(from), "{path} has no {from:?}: {text}");
    std::fs::write(&file, text.replace(from, to)).unwrap();
}

fn reload(app: &mut support::App) {
    app.harness.state_mut().reload_folder();
    settle(&mut app.harness);
}

fn url(app: &support::App) -> String {
    app.harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .url
        .clone()
}

#[test]
fn a_clean_request_reloads_in_place_when_its_file_changes() {
    let mut app = open_request();
    app.harness.state_mut().state.editor.as_mut().unwrap().tab =
        imposter_ui::app::EditorTab::Headers;
    let tab = app.harness.state().state.editor.as_ref().unwrap().tab;
    change_on_disk(
        &app,
        "shop-api/create-user.yaml",
        "localhost/users",
        "localhost/people",
    );
    reload(&mut app);
    assert_eq!(url(&app), "http://localhost/people");
    assert_eq!(
        app.harness.state().state.editor.as_ref().unwrap().tab,
        tab,
        "same tab"
    );
    app.harness
        .get_by_label_contains("Reloaded Create user: changed on disk");
}

#[test]
fn unsaved_edits_wait_and_take_theirs_keeps_them_beside() {
    let mut app = open_request();
    type_into(&mut app.harness, "Request URL", "/mine");
    change_on_disk(
        &app,
        "shop-api/create-user.yaml",
        "localhost/users",
        "localhost/theirs",
    );
    reload(&mut app);
    app.harness
        .get_by_label_contains("Create user changed on disk");
    assert_eq!(
        url(&app),
        "http://localhost/users/mine",
        "the edit is still here"
    );
    // Saving waits for the choice: the file keeps the outside change.
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert!(file(&app, "shop-api/create-user.yaml").contains("localhost/theirs"));

    click_button(
        &mut app.harness,
        "Take the file's version; keep my edits beside it",
    );
    settle(&mut app.harness);
    assert_eq!(url(&app), "http://localhost/theirs");
    assert!(app.harness.state().state.disk_change.is_none());
    app.harness.get_by_label("GET Create user (your edits)");
    assert!(file(&app, "shop-api/create-user-your-edits.yaml").contains("localhost/users/mine"));
}

#[test]
fn keep_mine_lets_the_next_save_write_over_the_file() {
    let mut app = open_request();
    type_into(&mut app.harness, "Request URL", "/mine");
    change_on_disk(
        &app,
        "shop-api/create-user.yaml",
        "localhost/users",
        "localhost/theirs",
    );
    reload(&mut app);
    click_button(
        &mut app.harness,
        "Keep my edits; the next save writes over the file",
    );
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    assert!(file(&app, "shop-api/create-user.yaml").contains("localhost/users/mine"));
    assert!(
        app.harness
            .query_by_label_contains("changed on disk")
            .is_none()
    );
}

#[test]
fn a_request_deleted_on_disk_closes_or_stays_as_a_draft() {
    let mut app = open_request();
    let path = app.dir.path().join("collections/shop-api/create-user.yaml");
    std::fs::remove_file(&path).unwrap();
    reload(&mut app);
    assert!(
        app.harness.state().state.editor.is_none(),
        "a clean editor closes"
    );
    app.harness
        .get_by_label_contains("Create user was deleted on disk");

    let mut app = open_request();
    type_into(&mut app.harness, "Request URL", "/mine");
    std::fs::remove_file(app.dir.path().join("collections/shop-api/create-user.yaml")).unwrap();
    reload(&mut app);
    app.harness
        .get_by_label_contains("Create user was deleted on disk. Your unsaved edits are here.");
    click_button(&mut app.harness, "Keep the edits as a draft");
    settle(&mut app.harness);
    let editor = &app.harness.state().state.editor.as_ref().unwrap().editor;
    assert!(editor.is_draft());
    assert_eq!(editor.url, "http://localhost/users/mine");
}

#[test]
fn a_folder_that_does_not_read_keeps_the_tree_and_says_why() {
    let mut app = open_request();
    let path = app.dir.path().join("collections/shop-api/create-user.yaml");
    let good = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("<<<<<<< HEAD\n{good}")).unwrap();
    reload(&mut app);
    app.harness.get_by_label_contains("does not read now");
    app.harness.get_by_label("GET Create user");
    std::fs::write(&path, good).unwrap();
    reload(&mut app);
    app.harness
        .get_by_label_contains("The collections folder reads again");
}

#[test]
fn the_variables_screen_reloads_or_asks() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                imposter_app::store::environments::set_globals(
                    db,
                    &[("region".to_owned(), "eu".to_owned())]
                        .into_iter()
                        .collect(),
                )
            })
            .unwrap();
    });
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    change_on_disk(&app, "globals.yaml", "region: eu", "region: us");
    reload(&mut app);
    assert_eq!(app.harness.state().state.variables.rows[0].value, "us");

    type_into(&mut app.harness, "Variable 1 value", "-mine");
    change_on_disk(&app, "globals.yaml", "region: us", "region: ap");
    reload(&mut app);
    app.harness
        .get_by_label_contains("These variables changed on disk");
    assert_eq!(app.harness.state().state.variables.rows[0].value, "us-mine");
    click_button(&mut app.harness, "Reload the variables from the file");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.variables.rows[0].value, "ap");
}
