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
use support::{click, open_settings_category, save_state, settle, start, start_with, type_into};

const AUTOSAVE: &str = "Automatically save changes";

fn switch<'a>(app: &'a support::App, label: &'a str) -> egui_kittest::Node<'a> {
    app.harness.get_by_role_and_label(Role::CheckBox, label)
}

fn click_switch(app: &mut support::App, label: &str) {
    switch(app, label).click();
    app.harness.run_steps(2);
}

fn open_settings(app: &mut support::App) {
    support::open_screen(&mut app.harness, "Settings");
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
fn categories_show_one_group_and_remember_the_selection_for_the_session() {
    let mut app = start();
    open_settings(&mut app);
    let fields = [
        ("Appearance", Role::Group, "Theme"),
        ("Collections", Role::Button, "Change collections folder"),
        ("General", Role::CheckBox, AUTOSAVE),
        ("Network", Role::TextInput, "Proxy server"),
        ("TLS", Role::TextInput, "CA bundle"),
        ("History", Role::Label, "History storage"),
        (
            "Command line",
            Role::CheckBox,
            "Allow production environments",
        ),
    ];
    app.harness.get_by_role_and_label(Role::Group, "Theme");
    for (category, _, _) in fields {
        open_settings_category(&mut app.harness, category);
        for (other, role, field) in fields {
            let label = format!("{other} settings");
            let item = app.harness.get_by_role_and_label(Role::Button, &label);
            assert_eq!(item.accesskit_node().is_selected(), Some(category == other));
            assert_eq!(
                app.harness.query_by_role_and_label(role, field).is_some(),
                category == other,
                "{field} while viewing {category}"
            );
        }
    }
    support::open_screen(&mut app.harness, "Requests");
    open_settings(&mut app);
    app.harness
        .get_by_role_and_label(Role::CheckBox, "Allow production environments");
}

#[test]
fn settings_explanations_are_tooltips_and_history_usage_is_visible() {
    let mut app = start();
    open_settings(&mut app);
    open_settings_category(&mut app.harness, "Network");
    let explanations = [
        "For connecting and for each read, not the whole exchange.",
        "Every request goes through it; empty for none. System proxies are never used.",
        "Larger bodies are cut at this size; 0 for no limit.",
        "Follow redirects returned by servers.",
        "Cookies servers set are sent back on later requests, per environment. They are listed on the Variables screen.",
    ];
    let help: Vec<_> = app.harness.query_all_by_label("Help").collect();
    assert_eq!(help.len(), explanations.len());
    for (node, explanation) in help.iter().zip(explanations) {
        assert_eq!(
            node.accesskit_node().description().as_deref(),
            Some(explanation)
        );
        assert!(
            app.harness.query_by_label(explanation).is_none(),
            "{explanation}"
        );
    }
    help[0].hover();
    app.harness.run_steps(12);
    app.harness.get_by_label(explanations[0]);

    open_settings_category(&mut app.harness, "History");
    app.harness
        .get_by_label_contains("History holds 0 executions, ");
    assert!(
        app.harness
            .query_by_label("The oldest runs go first when either limit is reached.")
            .is_none()
    );
}

#[test]
fn edits_are_unsaved_until_saved_and_saving_applies_them() {
    let mut app = start();
    open_settings(&mut app);
    assert_eq!(save_state(&app.harness, "Save settings"), "Saved");
    assert!(!save_enabled(&app), "nothing to save yet");

    open_settings_category(&mut app.harness, "Network");
    type_into(&mut app.harness, "Proxy server", "http://proxy:8080");
    open_settings_category(&mut app.harness, "General");
    support::assert_help(
        &app.harness,
        "Automatically saves changes to requests, variables and mocks. You can also use Save or Cmd/Ctrl+S.",
    );
    assert_eq!(
        switch(&app, AUTOSAVE).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    switch(&app, AUTOSAVE).focus();
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::Space);
    app.harness.run_steps(2);
    assert_eq!(
        switch(&app, AUTOSAVE).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    for label in ["General settings", "Network settings"] {
        assert_eq!(
            app.harness
                .get_by_role_and_label(Role::Button, label)
                .accesskit_node()
                .description()
                .as_deref(),
            Some("Unsaved changes"),
        );
    }
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Appearance settings")
            .accesskit_node()
            .description()
            .is_none()
    );
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
    for label in ["General settings", "Network settings"] {
        assert!(
            app.harness
                .get_by_role_and_label(Role::Button, label)
                .accesskit_node()
                .description()
                .is_none()
        );
    }
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
    open_settings_category(&mut app.harness, "Network");
    click_switch(&mut app, "Follow redirects");
    assert_eq!(save_state(&app.harness, "Save settings"), "Unsaved");
    assert!(app.harness.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Circle(dot) if dot.fill == imposter_ui::theme::palette().warning
        && app.harness.get_by_role_and_label(Role::Button, "Network settings").rect().contains(dot.center)
    )));
    click_switch(&mut app, "Follow redirects");
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
    open_settings_category(&mut app.harness, "TLS");
    click_switch(&mut app, "Verify TLS certificates");
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
    open_settings_category(&mut app.harness, "Network");
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
    open_settings_category(&mut app.harness, "Network");
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
    open_settings_category(&mut app.harness, "TLS");
    support::assert_icon_button(&app.harness, "Add host certificate");
    click(&mut app.harness, "Add host certificate");
    support::assert_icon_button(&app.harness, "Remove Host certificate 1");
    assert_eq!(
        app.harness
            .state()
            .state
            .settings_screen
            .form
            .host_identities
            .len(),
        1
    );
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
    app.harness
        .get_by_label("Add host certificate")
        .scroll_to_me();
    app.harness.run_steps(12);
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
fn host_certificate_headings_use_names_or_addresses_and_names_survive_restart() {
    let mut app = start();
    let size = egui::vec2(1200.0, 1500.0);
    app.harness.set_size(size);
    app.harness.run_steps(2);
    open_settings(&mut app);
    open_settings_category(&mut app.harness, "TLS");
    click(&mut app.harness, "Add host certificate");
    assert!(
        app.harness
            .query_by_role_and_label(Role::TextInput, "Host certificate 1 Name")
            .is_some(),
        "the certificate needs an optional name field"
    );
    type_into(
        &mut app.harness,
        "Host certificate 1 Name",
        "  Staging API  ",
    );
    app.harness.get_by_label("Staging API");
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.key_press(egui::Key::Backspace);
    app.harness.run_steps(2);
    type_into(
        &mut app.harness,
        "Host certificate 1 Host",
        "staging.example.test",
    );
    app.harness.get_by_label("staging.example.test:443");
    type_into(&mut app.harness, "Host certificate 1 Name", "  QA API  ");
    type_into(
        &mut app.harness,
        "Host certificate 1 Certificate",
        "client.pem",
    );
    assert_eq!(stored(&app).host_identities, []);
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);
    assert_eq!(
        serde_json::to_value(stored(&app)).unwrap()["host_identities"][0]["name"],
        "QA API"
    );

    drop(app.harness);
    let mut again = support::harness_at(app.dir.path(), size);
    support::open_screen(&mut again, "Settings");
    settle(&mut again);
    open_settings_category(&mut again, "TLS");
    again.get_by_label("QA API");
    assert_eq!(
        support::text_input(&again, "Host certificate 1 Name")
            .value()
            .as_deref(),
        Some("QA API")
    );
}

#[test]
fn a_host_certificate_is_removed_from_its_header_after_two_clicks() {
    let mut app = start();
    app.harness.set_size(egui::vec2(1200.0, 900.0));
    open_settings(&mut app);
    open_settings_category(&mut app.harness, "TLS");
    click(&mut app.harness, "Add host certificate");
    let remove = app.harness.get_by_label("Remove Host certificate 1");
    let title = app.harness.get_by_label("Host certificate 1");
    assert!((remove.rect().center().y - title.rect().center().y).abs() < 4.0);
    click(&mut app.harness, "Remove Host certificate 1");
    support::text_input(&app.harness, "Host certificate 1 Host");
    click(&mut app.harness, "Click again to remove Host certificate 1");
    assert!(
        app.harness
            .query_by_label("Host certificate 1 Host")
            .is_none()
    );
    assert_eq!(save_state(&app.harness, "Save settings"), "Saved");
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
    app.harness.get_by_role_and_label(Role::Group, "Theme");
    click(&mut app.harness, "Theme: Light");
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
    open_settings_category(&mut app.harness, "TLS");
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
    // Tall enough for the TLS group and the certificate block.
    app.harness.set_size(egui::vec2(1200.0, 900.0));
    app.harness.run_steps(2);
    open_settings(&mut app);
    open_settings_category(&mut app.harness, "Network");
    let proxy_left = support::text_input(&app.harness, "Proxy server")
        .rect()
        .left();
    open_settings_category(&mut app.harness, "TLS");
    click(&mut app.harness, "Add host certificate");
    let mut lefts = vec![("Proxy server", proxy_left)];
    lefts.extend(["CA bundle", "Host certificate 1 Host"].map(|label| {
        (
            label,
            support::text_input(&app.harness, label).rect().left(),
        )
    }));
    assert!(
        lefts
            .iter()
            .all(|(_, left)| (left - lefts[0].1).abs() < 0.5),
        "{lefts:?}"
    );
    switch(&app, "Verify TLS certificates");
}

#[test]
fn production_from_the_command_line_is_allowed_only_here() {
    let mut app = start();
    open_settings(&mut app);
    assert!(!stored(&app).allow_production_from_cli, "off until allowed");
    open_settings_category(&mut app.harness, "Command line");
    switch(&app, "Allow production environments").click_accesskit();
    app.harness.run_steps(2);
    assert!(
        app.harness
            .state()
            .state
            .settings_screen
            .form
            .allow_production_from_cli,
        "the click reached the switch"
    );
    assert!(save_enabled(&app), "the switch is an unsaved edit");
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);
    assert!(stored(&app).allow_production_from_cli);
}
