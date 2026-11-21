//! Saved values: kept per scope beside a variable, chosen from the inspector to change what a
//! request sends, and edited on the Variables screen under its unsaved-changes guard.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{SavedValues, Variables};
use imposter_app::store::{environments, scopes};
use imposter_testkit::{Server, response};
use imposter_ui::app::{Action, Appearance, Screen};
use imposter_ui::theme::{Density, ThemeChoice};
use imposter_ui::vars::Scope;
use imposter_ui::widgets::Inspection;
use support::{click, collection, request, settle, start_with};

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn saved(name: &str, values: &[&str]) -> SavedValues {
    let mut saved = SavedValues::default();
    saved.set(name, values.iter().map(|v| (*v).to_owned()));
    saved
}

/// Globals `base` and `user` (42, with 42 and 77 saved) and an open request using them.
fn with_saved_user(base: String) -> support::App {
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                scopes::write(
                    db,
                    Scope::Global,
                    &vars(&[("base", &base), ("user", "42")]),
                    &saved("user", &["42", "77"]),
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "User", "{{base}}/users/{{user}}");
    });
    click(&mut app.harness, "GET User");
    settle(&mut app.harness);
    app
}

fn inspect(app: &mut support::App, name: &str, masked: bool) {
    let anchor = support::text_input(&app.harness, "Request URL")
        .rect()
        .left_bottom();
    app.harness.state_mut().state.inspect = Some(Inspection {
        name: name.into(),
        masked,
        anchor,
    });
    app.harness.run_steps(2);
}

fn stored(app: &support::App) -> (Variables, SavedValues) {
    app.harness
        .state()
        .store()
        .call_blocking(|db| scopes::read(db, Scope::Global))
        .unwrap()
}

fn disabled(app: &support::App, label: &str) -> bool {
    app.harness
        .get_by_label(label)
        .accesskit_node()
        .is_disabled()
}

fn variables_app() -> support::App {
    let mut app = with_saved_user("http://h".into());
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app
}

fn draft(app: &mut support::App, value: &str) {
    support::type_into(&mut app.harness, "Add value to variable 2", value);
}

fn press(app: &mut support::App, key: egui::Key) {
    app.harness.key_press(key);
    app.harness.run_steps(2);
}

fn open_menu(app: &mut support::App, chip: &str) {
    app.harness.get_by_label(chip).click_secondary();
    app.harness.run_steps(2);
}

#[test]
fn choosing_a_saved_value_changes_what_the_request_sends() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"")));
    let mut app = with_saved_user(server.url.clone());

    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    inspect(&mut app, "user", false);
    app.harness.get_by_label("Saved values");
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);
    assert!(
        app.harness.state().state.inspect.is_none(),
        "choosing closes the inspector"
    );
    app.harness.get_by_label("user: saved value chosen");
    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    let targets: Vec<String> = server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets, ["/users/42", "/users/77"]);
    assert_eq!(
        stored(&app),
        (
            vars(&[("base", &server.url), ("user", "77")]),
            saved("user", &["42", "77"])
        ),
        "the list stays; only the current value changed"
    );
}

#[test]
fn chips_select_and_save_pending_values_under_the_guard() {
    let mut app = variables_app();
    app.harness.get_by_label("Variable 1 value 1: http://h");
    assert!(!app.harness.state().state.variables.dirty());

    draft(&mut app, "91");
    click(&mut app.harness, "Variable 2 value 2: 77");
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.current(), Some("77"));
    assert_eq!(row.draft, "91", "clicking away retains the draft");
    assert!(app.harness.state().state.variables.dirty());
    assert_eq!(stored(&app).0["user"], "42");
    assert!(
        app.harness
            .query_by_label("Variable 2 value 3: 91")
            .is_none()
    );

    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "91");
    assert_eq!(stored(&app).1.values("user"), ["42", "77", "91"]);
    assert!(!app.harness.state().state.variables.dirty());
    app.harness.get_by_label("Variable 2 value 3: 91");

    click(&mut app.harness, "Variable 2 value 1: 42");
    draft(&mut app, "100");
    press(&mut app, egui::Key::Enter);
    click(&mut app.harness, "Revert unsaved changes");
    assert!(app.harness.state().state.variables.dirty());
    support::click_button(&mut app.harness, "Discard changes");
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.current(), Some("91"));
    assert_eq!(row.values(), ["42", "77", "91"]);
    assert_eq!(row.draft, "");
    assert!(!app.harness.state().state.variables.dirty());
}

#[test]
fn enter_adds_or_selects_a_value_and_escape_clears_the_draft() {
    let mut app = variables_app();
    draft(&mut app, "1001");
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("42")
    );
    press(&mut app, egui::Key::Enter);
    let chip = app.harness.get_by_label("Variable 2 value 3: 1001");
    assert_eq!(
        chip.accesskit_node().description().as_deref(),
        Some("The current value")
    );
    assert!(support::text_input(&app.harness, "Add value to variable 2").is_focused());
    assert_eq!(stored(&app).0["user"], "42");

    draft(&mut app, "77");
    press(&mut app, egui::Key::Enter);
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("77")
    );
    assert_eq!(
        app.harness.state().state.variables.rows[1].values(),
        ["42", "77", "1001"]
    );
    draft(&mut app, "discard me");
    press(&mut app, egui::Key::Escape);
    assert_eq!(app.harness.state().state.variables.rows[1].draft, "");
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("77")
    );

    support::text_input(&app.harness, "Add value to variable 2").click();
    app.harness.run_steps(2);
    press(&mut app, egui::Key::Enter);
    app.harness.get_by_label("Variable 2 value 4: Empty value");
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("")
    );
    support::type_into(&mut app.harness, "New variable name", "blank");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    app.harness.get_by_label("Variable 3 value 1: Empty value");
    assert_eq!(stored(&app).0["blank"], "");
}

#[test]
fn unsaved_edits_to_the_scope_block_choosing_and_a_clean_screen_follows_the_choice() {
    let mut app = with_saved_user("http://h".into());
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app.harness.state_mut().state.variables.rows[0].draft = "http://edited".into();
    app.harness.run_steps(2);
    app.harness.state_mut().state.screen = Screen::Requests;
    app.harness.run_steps(2);

    inspect(&mut app, "user", false);
    assert!(disabled(&app, "Use saved value 2: 77"));
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "42", "nothing was written");

    click(&mut app.harness, "Close");
    app.harness.state_mut().state.screen = Screen::Variables;
    app.harness.run_steps(2);
    click(&mut app.harness, "Revert unsaved changes");
    support::click_button(&mut app.harness, "Discard changes");
    app.harness.state_mut().state.screen = Screen::Requests;
    app.harness.run_steps(2);
    inspect(&mut app, "user", false);
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);

    assert_eq!(stored(&app).0["user"], "77");
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("77"),
        "the Variables screen shows the chosen value"
    );
}

#[test]
fn saved_values_inspected_from_a_password_field_stay_masked() {
    let mut app = with_saved_user("http://h".into());
    inspect(&mut app, "user", true);

    app.harness.get_by_label("Use saved value 2: ••••••••");
    assert!(app.harness.query_by_label_contains("77").is_none());
    click(&mut app.harness, "Reveal");
    app.harness.get_by_label("Use saved value 2: 77");
}

#[test]
fn a_value_removed_meanwhile_is_reported_and_not_recreated() {
    let mut app = with_saved_user("http://h".into());
    inspect(&mut app, "user", false);
    app.harness
        .state()
        .store()
        .call_blocking(|db| environments::set_globals(db, &vars(&[("base", "http://h")])))
        .unwrap();

    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);

    app.harness.get_by_label("Value not changed");
    assert_eq!(stored(&app).0, vars(&[("base", "http://h")]));
}

#[test]
fn the_current_chip_is_filled_and_chips_have_no_reserved_remove_space() {
    let app = variables_app();
    let area = |name| app.harness.get_by_label(name).rect();
    let current = area("Variable 2 value 1: 42");
    let next = area("Variable 2 value 2: 77");
    let composer = support::text_input(&app.harness, "Add value to variable 2").rect();
    assert!((next.left() - current.right() - 8.0).abs() < 0.5);
    assert!((composer.left() - next.right() - 8.0).abs() < 0.5);
    let palette = imposter_ui::theme::palette();
    let selected = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape)
                if shape.rect.intersects(current) && shape.fill == palette.selected =>
            {
                Some(shape)
            }
            _ => None,
        })
        .expect("a filled current chip");
    assert_eq!(selected.stroke, egui::Stroke::NONE);
    assert!(
        app.harness
            .query_by_label_contains("Pin variable")
            .is_none()
    );
    assert!(
        app.harness
            .query_by_label_contains("Remove variable 2 value")
            .is_none()
    );
}

#[test]
fn remove_stays_beside_the_values_column_in_a_wide_window() {
    let mut app = variables_app();
    app.harness.set_size(egui::vec2(1600.0, 900.0));
    settle(&mut app.harness);
    let values = app.harness.get_by_label("Values").rect();
    let remove = app.harness.get_by_label("Remove variable 2").rect();
    let gap = imposter_ui::theme::spacing().sm;
    assert!((remove.left() - values.left() - 640.0 - gap).abs() < 1.0);
    assert!(remove.right() < app.harness.ctx.content_rect().right() - 200.0);
}

#[test]
fn the_chip_menu_copies_the_full_value_and_removes_it() {
    let prefix = "x".repeat(60);
    let long = format!("{prefix}-long\nsecond line");
    let stored_value = long.clone();
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                scopes::write(
                    db,
                    Scope::Global,
                    &vars(&[("user", "42")]),
                    &saved("user", &["42", &stored_value]),
                )
            })
            .unwrap();
    });
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    let chip = format!("Variable 1 value 2: {prefix}…");
    open_menu(&mut app, &chip);
    app.harness.get_by_label(&format!("{prefix}…"));
    app.harness.get_by_label("Copy value").click();
    app.harness.step();
    assert!(
        app.harness
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| {
                matches!(command, egui::OutputCommand::CopyText(text) if text == &long)
            })
    );

    click(&mut app.harness, &chip);
    open_menu(&mut app, &chip);
    click(&mut app.harness, "Remove from list");
    assert!(app.harness.query_by_label(&chip).is_none());
    let row = &app.harness.state().state.variables.rows[0];
    assert_eq!(row.values(), ["42"]);
    assert_eq!(row.current(), Some("42"));
    open_menu(&mut app, "Variable 1 value 1: 42");
    assert!(disabled(&app, "Remove from list"));
}

#[test]
fn keyboard_and_accessibility_open_the_chip_menu() {
    let mut app = variables_app();
    let chip = "Variable 2 value 1: 42";
    app.harness.get_by_label(chip).focus();
    app.harness.run_steps(2);
    app.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
    app.harness.run_steps(2);
    assert!(app.harness.get_by_label("Copy value").is_focused());
    press(&mut app, egui::Key::Tab);
    assert!(app.harness.get_by_label("Remove from list").is_focused());
    press(&mut app, egui::Key::Enter);
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.current(), Some("77"));
    assert_eq!(row.values(), ["77"]);
    assert!(support::text_input(&app.harness, "Add value to variable 2").is_focused());

    let chip = app.harness.get_by_label("Variable 2 value 1: 77");
    let (target_node, target_tree) = chip.accesskit_node().locate();
    app.harness.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::ShowContextMenu,
            target_node,
            target_tree,
            data: None,
        },
    ));
    app.harness.run_steps(2);
    assert!(app.harness.get_by_label("Copy value").is_focused());
}

#[test]
fn long_chips_wrap_under_the_values_heading_and_row_remove_buttons_stay_aligned() {
    let mut app = variables_app();
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    let prefix = "x".repeat(60);
    let first = format!("{prefix}-one");
    let second = format!("{prefix}-two");
    for (index, value) in [&first, &second].into_iter().enumerate() {
        draft(&mut app, value);
        assert_eq!(&app.harness.state().state.variables.rows[1].draft, value);
        press(&mut app, egui::Key::Enter);
        assert_eq!(
            app.harness.state().state.variables.rows[1].values().len(),
            index + 3
        );
    }
    app.harness.hover_at(egui::pos2(5.0, 5.0));
    for density in [Density::Compact, Density::Comfortable] {
        app.harness
            .state_mut()
            .state
            .actions
            .push(Action::SetAppearance(Appearance {
                theme: ThemeChoice::Dark,
                density,
                ..Appearance::default()
            }));
        app.harness.run_steps(3);
        let area = |label: &str| app.harness.get_by_label(label).rect();
        assert!((area("Remove variable 1").left() - area("Remove variable 2").left()).abs() < 0.5);
        let left = area("Variable 2 value 1: 42").left();
        let first = area(&format!("Variable 2 value 3: {prefix}…"));
        let second = area(&format!("Variable 2 value 4: {prefix}…"));
        assert!((first.left() - left).abs() < 0.5);
        assert!((second.left() - left).abs() < 0.5);
        assert!(first.bottom() < second.top());
        assert!((second.height() - density.spacing().control).abs() < 0.5);
    }
    app.harness
        .get_by_label(&format!("Variable 2 value 4: {prefix}…"))
        .hover();
    app.harness.run_steps(8);
    app.harness.get_by_label(&second);
}

#[test]
fn keeping_the_latest_capture_while_another_chip_is_active_is_an_explicit_edit() {
    let mut app = with_saved_user("http://h".into());
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            for value in ["old-capture", "recent-capture"] {
                scopes::set_current(db, Scope::Global, &[("user".into(), value.into())])?;
            }
            Ok(())
        })
        .unwrap();
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("recent-capture")
    );
    assert!(app.harness.query_by_label_contains("old-capture").is_none());
    let capture = "Variable 2 latest capture: recent-capture";
    click(&mut app.harness, "Variable 2 value 2: 77");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "77");
    assert_eq!(stored(&app).1.values("user"), ["42", "77"]);
    let globals = app.dir.path().join("collections/globals.yaml");
    assert!(
        !std::fs::read_to_string(&globals)
            .unwrap()
            .contains("recent-capture")
    );

    open_menu(&mut app, capture);
    click(&mut app.harness, "Keep in list");
    assert_eq!(
        app.harness.state().state.variables.rows[1].current(),
        Some("77")
    );
    assert!(app.harness.query_by_label(capture).is_none());
    app.harness
        .get_by_label("Variable 2 value 3: recent-capture");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "77");
    assert_eq!(
        stored(&app).1.values("user"),
        ["42", "77", "recent-capture"]
    );
    assert!(
        std::fs::read_to_string(globals)
            .unwrap()
            .contains("recent-capture")
    );
}
