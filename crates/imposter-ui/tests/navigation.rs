//! Getting around: the command palette, through the real UI and database.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use imposter_app::core::model::Variables;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::{environments, kv};
use imposter_ui::app::Screen;
use support::{click, collection, request, settle, start_with, type_into};

fn palette(app: &mut support::App, query: &str) {
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
    settle(&mut app.harness);
    support::text_input(&app.harness, "Command palette search").type_text(query);
    app.harness.run_steps(2);
}

fn enter(app: &mut support::App) {
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
}

/// Two collections: API (Health, Users/List users) and Admin (Stats), API's Health open.
fn two_collections() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &AppSettings {
                        autosave: false,
                        ..AppSettings::default()
                    },
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Health", "http://h/health");
        let admin = collection(store, "Admin");
        request(store, admin.id, "Stats", "http://h/stats");
        store
            .call_blocking(|db| environments::create_environment(db, "Dev", &Variables::new()))
            .unwrap();
    });
    app.harness.state_mut().state.settings.autosave = false;
    settle(&mut app.harness);
    app
}

fn open_name(app: &support::App) -> Option<String> {
    app.harness
        .state()
        .state
        .editor
        .as_ref()
        .map(|p| p.editor.name().to_owned())
}

fn collection_name(app: &support::App) -> String {
    let state = &app.harness.state().state;
    state
        .collections
        .iter()
        .find(|c| Some(c.id) == state.collection)
        .unwrap()
        .name
        .clone()
}

#[test]
fn the_palette_opens_a_request_from_another_collection() {
    let mut app = two_collections();
    let start = collection_name(&app);
    let (other, wanted) = if start == "API" {
        ("Admin", "stats")
    } else {
        ("API", "health")
    };

    palette(&mut app, wanted);
    enter(&mut app);

    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(collection_name(&app), other);
    assert_eq!(
        open_name(&app).map(|n| n.to_lowercase()),
        Some(wanted.to_owned())
    );
}

#[test]
fn unsaved_edits_are_guarded_before_the_palette_navigates() {
    let mut app = two_collections();
    palette(&mut app, "health");
    enter(&mut app);
    type_into(&mut app.harness, "Request URL", "/edited");

    palette(&mut app, "stats");
    enter(&mut app);

    app.harness
        .get_by_label("Save changes to \"Health\" before continuing?");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(open_name(&app).as_deref(), Some("Stats"));
}

#[test]
fn the_palette_runs_actions_and_switches_environments() {
    let mut app = two_collections();
    palette(&mut app, "history");
    click(&mut app.harness, "Go to History");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::History);

    palette(&mut app, "dev");
    enter(&mut app);
    let state = &app.harness.state().state;
    let dev = state.environments.iter().find(|e| e.name == "Dev").unwrap();
    assert_eq!(state.active_environment, Some(dev.id));
}

#[test]
fn arrows_choose_and_escape_closes_without_acting() {
    let mut app = two_collections();
    palette(&mut app, "go to");
    app.harness.key_press(Key::ArrowDown);
    app.harness.run_steps(1);
    app.harness.key_press(Key::ArrowUp);
    app.harness.run_steps(1);
    app.harness.key_press(Key::ArrowUp);
    app.harness.run_steps(1);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(app.harness.state().state.screen, Screen::Requests);

    palette(&mut app, "zzzz");
    app.harness.get_by_label("Nothing matches");
    enter(&mut app);
    assert!(
        app.harness.state().state.dialog.is_some(),
        "Enter with no match keeps the palette open"
    );
}

/// One collection with requests A, B and C, opened in that order (C is open).
fn opened_abc() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        for name in ["A", "B", "C"] {
            request(store, api.id, name, &format!("http://h/{name}"));
        }
    });
    for name in ["A", "B", "C"] {
        click(&mut app.harness, &format!("GET {name}"));
        settle(&mut app.harness);
    }
    app
}

fn hold_ctrl(app: &mut support::App) {
    app.harness
        .event(egui::Event::ModifiersChanged(Modifiers::CTRL));
}

fn tab_while_holding(app: &mut support::App) {
    app.harness.event(egui::Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::CTRL,
    });
    settle(&mut app.harness);
}

fn release_ctrl(app: &mut support::App) {
    app.harness
        .event(egui::Event::ModifiersChanged(Modifiers::NONE));
    settle(&mut app.harness);
}

fn recent_names(app: &support::App) -> Vec<String> {
    let Some(imposter_ui::dialogs::Dialog::Palette(palette)) = &app.harness.state().state.dialog
    else {
        panic!("the palette is not open");
    };
    palette.items().into_iter().map(|i| i.label).collect()
}

#[test]
fn recent_requests_come_first_and_survive_a_restart() {
    let mut app = opened_abc();
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut app.harness);
    assert_eq!(recent_names(&app), ["API › C", "API › B", "API › A"]);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    let dir = app.dir;
    drop(app.harness);
    let mut reopened = support::harness_at(dir.path(), egui::vec2(1400.0, 900.0));
    reopened.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut reopened);
    let mut app = support::App {
        dir,
        harness: reopened,
    };
    assert_eq!(recent_names(&app), ["API › C", "API › B", "API › A"]);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
}

#[test]
fn a_quick_ctrl_tab_goes_back_and_forth() {
    let mut app = opened_abc();
    app.harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("B"));

    app.harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    settle(&mut app.harness);
    assert_eq!(open_name(&app).as_deref(), Some("C"));
}

#[test]
fn holding_ctrl_steps_through_recent_requests_until_it_is_released() {
    let mut app = opened_abc();
    hold_ctrl(&mut app);
    tab_while_holding(&mut app);
    assert!(
        app.harness.state().state.dialog.is_some(),
        "open while held"
    );
    tab_while_holding(&mut app);
    release_ctrl(&mut app);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("A"));

    app.harness
        .key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
    settle(&mut app.harness);
    assert_eq!(
        open_name(&app).as_deref(),
        Some("B"),
        "Shift starts from the oldest"
    );

    hold_ctrl(&mut app);
    tab_while_holding(&mut app);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
    release_ctrl(&mut app);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("B"), "Esc cancels");
}

#[test]
fn deleted_requests_leave_the_recent_list() {
    let mut app = opened_abc();
    let b = app
        .harness
        .state()
        .state
        .tree
        .as_ref()
        .unwrap()
        .source
        .requests
        .iter()
        .find(|r| r.name == "B")
        .unwrap()
        .id;
    app.harness
        .state()
        .store()
        .call_blocking(move |db| imposter_app::store::collections::delete_request(db, b))
        .unwrap();

    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut app.harness);
    assert_eq!(recent_names(&app), ["API › C", "API › A"]);
    let stored: Vec<imposter_app::core::ids::RequestId> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "recent_requests"))
        .unwrap()
        .unwrap();
    assert!(!stored.contains(&b), "the stored list is pruned too");
}
