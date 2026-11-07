//! The Settings screen: load, edit against the saved baseline, save, retention and the close
//! guard.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::execution::{ExecutionRecord, Response, Timestamp};
use imposter_app::core::model::Request;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::{executions, kv};
use imposter_ui::app::{Action, CloseState};
use support::{click, save_state, settle, start, start_with, type_into};

fn open_settings(app: &mut support::App) {
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
}

fn stored(app: &support::App) -> AppSettings {
    app.harness
        .state()
        .store()
        .call_blocking(|db| kv::app_settings(db))
        .unwrap()
}

fn save_enabled(app: &support::App) -> bool {
    !app.harness
        .get_by_role_and_label(Role::Button, "Save settings")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn edits_are_unsaved_until_saved_and_saving_applies_them() {
    let mut app = start();
    open_settings(&mut app);
    assert_eq!(save_state(&app.harness, "Save settings"), "Saved");
    assert!(!save_enabled(&app), "nothing to save yet");

    type_into(&mut app.harness, "Proxy server", "http://proxy:8080");
    click(&mut app.harness, "Automatically save request changes");
    assert_eq!(save_state(&app.harness, "Save settings"), "Unsaved");
    assert!(
        app.harness.query_by_label("Unsaved").is_none(),
        "the highlighted icon already says it"
    );
    assert!(save_enabled(&app));
    assert_eq!(
        stored(&app),
        AppSettings::default(),
        "nothing is written before Save"
    );

    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);

    assert_eq!(save_state(&app.harness, "Save settings"), "Saved");
    assert!(!save_enabled(&app));
    let saved = stored(&app);
    assert_eq!(saved.proxy_url, "http://proxy:8080");
    assert!(!saved.autosave);
    assert_eq!(
        app.harness.state().state.settings,
        saved,
        "the app uses them at once"
    );
}

#[test]
fn undoing_an_edit_is_not_a_change() {
    let mut app = start();
    open_settings(&mut app);
    click(&mut app.harness, "Follow redirects");
    assert_eq!(save_state(&app.harness, "Save settings"), "Unsaved");
    click(&mut app.harness, "Follow redirects");
    assert_eq!(save_state(&app.harness, "Save settings"), "Saved");
    assert!(!save_enabled(&app));
}

#[test]
fn settings_the_screen_does_not_show_are_kept() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &AppSettings {
                        attach_cookies: false,
                        ..AppSettings::default()
                    },
                )
            })
            .unwrap();
    });
    open_settings(&mut app);
    click(&mut app.harness, "Verify TLS certificates");
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);

    let saved = stored(&app);
    assert!(!saved.verify_ssl);
    assert!(!saved.attach_cookies);
}

#[test]
fn lower_history_limits_apply_when_saved() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                for i in 0..5 {
                    let record = ExecutionRecord {
                        timestamp: Timestamp(1_700_000_000_000 + i),
                        request_id: None,
                        template: Request::default(),
                        resolved: Request::default(),
                        settings: AppSettings::default(),
                        environment_id: None,
                        environment: "Global".into(),
                        response: Response::default(),
                        via: imposter_app::core::execution::Via::App,
                    };
                    executions::record(db, &record, 1000, u64::MAX)?;
                }
                Ok(())
            })
            .unwrap();
    });
    open_settings(&mut app);
    app.harness
        .state_mut()
        .state
        .settings_screen
        .form
        .history_max_entries = 2;
    app.harness.state_mut().handle(Action::SettingsEdited, 0.0);
    app.harness.state_mut().handle(Action::Save, 0.0);
    settle(&mut app.harness);

    let count = app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap();
    assert_eq!(count, 2, "the oldest executions are deleted at once");
}

#[test]
fn closing_with_unsaved_settings_asks_first() {
    let mut app = start();
    open_settings(&mut app);
    type_into(&mut app.harness, "Proxy server", "http://p");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness
        .get_by_label("Save application settings before exiting?");
    click(&mut app.harness, "Cancel");
    assert_eq!(app.harness.state().close_state(), CloseState::Open);

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    assert_eq!(stored(&app).proxy_url, "http://p");
}

#[test]
fn discarding_settings_on_close_keeps_the_stored_ones() {
    let mut app = start();
    open_settings(&mut app);
    type_into(&mut app.harness, "Proxy server", "http://p");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    assert_eq!(stored(&app), AppSettings::default());
}

#[test]
fn host_certificates_save_only_on_save_and_validate_duplicates() {
    let mut app = start();
    app.harness.set_size(egui::vec2(1200.0, 1500.0));
    app.harness.run_steps(2);
    open_settings(&mut app);
    click(&mut app.harness, "Add host certificate");
    type_into(&mut app.harness, "Host certificate 1 Host", "Example.COM");
    type_into(
        &mut app.harness,
        "Host certificate 1 Certificate",
        "/client.pem",
    );
    type_into(&mut app.harness, "Host certificate 1 Password", "secret");
    assert_eq!(stored(&app).host_identities, []);
    let password = app
        .harness
        .get_by_role_and_label(Role::PasswordInput, "Host certificate 1 Password");
    assert!(
        !password
            .accesskit_node()
            .value()
            .is_some_and(|value| value.contains("secret"))
    );
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);
    assert_eq!(stored(&app).host_identities[0].port, 443);
    assert_eq!(stored(&app).host_identities[0].password, "secret");
    click(&mut app.harness, "Add host certificate");
    type_into(&mut app.harness, "Host certificate 2 Host", "example.com");
    type_into(
        &mut app.harness,
        "Host certificate 2 Certificate",
        "/other.pem",
    );
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);
    assert_eq!(stored(&app).host_identities.len(), 1);
    // A refused form is invalid, not a failed write; the status bar says why.
    app.harness.get_by_label("Invalid fields");
    app.harness.get_by_label_contains("Settings not saved:");
    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness.get_by_label("Unsaved application settings");
    click(&mut app.harness, "Cancel");
    assert_eq!(app.harness.state().close_state(), CloseState::Open);
}

#[test]
fn the_theme_applies_at_once_and_is_remembered() {
    let mut app = start();
    open_settings(&mut app);
    assert_eq!(
        app.harness.ctx.theme(),
        egui::Theme::Dark,
        "dark by default"
    );
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Theme")
        .click();
    app.harness.run_steps(2);
    click(&mut app.harness, "Light");
    settle(&mut app.harness);

    assert_eq!(app.harness.ctx.theme(), egui::Theme::Light);
    assert_eq!(imposter_ui::theme::palette(), &imposter_ui::theme::LIGHT);
    assert!(!save_enabled(&app), "a preference, not an unsaved setting");
    let stored: serde_json::Value = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "appearance"))
        .unwrap()
        .unwrap();
    assert_eq!(stored["theme"], "light");

    let dir = app.dir;
    drop(app.harness);
    let mut again = support::harness_at(dir.path(), egui::vec2(1400.0, 900.0));
    settle(&mut again);
    assert_eq!(
        again.ctx.theme(),
        egui::Theme::Light,
        "restored at the next start"
    );
}

#[test]
fn paths_are_chosen_with_browse_and_a_missing_file_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let pem = dir.path().join("ca.pem");
    std::fs::write(&pem, b"pem").unwrap();
    let mut app = start();
    open_settings(&mut app);
    app.harness
        .state_mut()
        .script_file_dialogs([Some(pem.clone())]);
    click(&mut app.harness, "Browse for ca bundle");
    settle(&mut app.harness);
    app.harness.run_steps(2);
    settle(&mut app.harness);
    app.harness.run_steps(2);
    assert_eq!(
        app.harness.state().state.settings_screen.form.ca_bundle,
        pem.display().to_string()
    );
    assert!(save_enabled(&app), "a chosen path is an edit");
    assert!(app.harness.query_by_label("File not found").is_none());

    type_into(&mut app.harness, "CA bundle", ".missing");
    settle(&mut app.harness);
    app.harness.run_steps(2);
    app.harness.get_by_label("File not found");
}

#[test]
fn every_section_puts_its_fields_in_one_column() {
    let mut app = start();
    // Tall enough that every section is on screen at once.
    app.harness.set_size(egui::vec2(1200.0, 1500.0));
    app.harness.run_steps(2);
    open_settings(&mut app);
    click(&mut app.harness, "Add host certificate");
    let lefts: Vec<(String, f32)> = ["Proxy server", "CA bundle", "Host certificate 1 Host"]
        .into_iter()
        .map(|label| {
            let rect = support::text_input(&app.harness, label).rect();
            (label.to_owned(), rect.left())
        })
        .collect();
    assert!(
        lefts
            .iter()
            .all(|(_, left)| (left - lefts[0].1).abs() < 0.5),
        "{lefts:?}"
    );
    app.harness.get_by_label("Verify TLS certificates");
}
