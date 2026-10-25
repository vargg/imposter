//! The Settings screen: load, edit against the saved baseline, save, retention and the close
//! guard.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use poster_app::core::execution::{ExecutionRecord, Response, Timestamp};
use poster_app::core::model::Request;
use poster_app::core::settings::AppSettings;
use poster_app::store::{executions, kv};
use poster_ui::app::{Action, CloseState};
use support::{click, settle, start, start_with, type_into};

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
    app.harness.get_by_label("Saved");
    assert!(!save_enabled(&app), "nothing to save yet");

    type_into(&mut app.harness, "Proxy server", "http://proxy:8080");
    click(&mut app.harness, "Automatically save request changes");
    app.harness.get_by_label("Unsaved");
    assert!(save_enabled(&app));
    assert_eq!(
        stored(&app),
        AppSettings::default(),
        "nothing is written before Save"
    );

    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);

    app.harness.get_by_label("Saved");
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
    app.harness.get_by_label("Unsaved");
    click(&mut app.harness, "Follow redirects");
    app.harness.get_by_label("Saved");
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
    click(&mut app.harness, "Verify SSL certificates");
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
