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
