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
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app
}

fn enter_value(app: &mut support::App, text: &str) {
    let label = "Variable 2 value";
    support::text_input(&app.harness, label).click();
    app.harness.run_steps(2);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.run_steps(1);
    if text.is_empty() {
        app.harness.key_press(egui::Key::Backspace);
    } else {
        support::text_input(&app.harness, label).type_text(text);
    }
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
fn chips_swap_the_current_value_under_the_guard() {
    let mut app = variables_app();
    click(&mut app.harness, "Variable 2 saved value 1: 42");
    assert!(!app.harness.state().state.variables.dirty());
    click(&mut app.harness, "Variable 2 saved value 2: 77");
    assert_eq!(app.harness.state().state.variables.rows[1].value, "77");
    assert!(app.harness.state().state.variables.dirty());
    assert_eq!(stored(&app).0["user"], "42");

    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "77");
    assert_eq!(stored(&app).1, saved("user", &["42", "77"]));
    assert!(!app.harness.state().state.variables.dirty());

    click(&mut app.harness, "Variable 2 saved value 1: 42");
    enter_value(&mut app, "91");
    click(&mut app.harness, "Pin variable 2 value");
    assert_eq!(
        app.harness.state().state.variables.rows[1].saved,
        ["42", "77", "91"]
    );
    click(&mut app.harness, "Revert unsaved changes");
    assert!(app.harness.state().state.variables.dirty());
    support::click_button(&mut app.harness, "Discard changes");
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.value, "77");
    assert_eq!(row.saved, ["42", "77"]);
    assert!(!app.harness.state().state.variables.dirty());
}

#[test]
fn enter_pins_the_typed_value_once_and_never_empty() {
    let mut app = variables_app();
    assert!(disabled(&app, "Pin variable 2 value"));
    assert!(
        !disabled(&app, "Pin variable 1 value"),
        "plain rows can pin"
    );

    enter_value(&mut app, "1001");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    let chip = app.harness.get_by_label("Variable 2 saved value 3: 1001");
    assert_eq!(
        chip.accesskit_node().description().as_deref(),
        Some("The current value")
    );
    assert!(!support::text_input(&app.harness, "Variable 2 value").is_focused());

    enter_value(&mut app, "1001");
    assert!(disabled(&app, "Pin variable 2 value"));
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    enter_value(&mut app, "");
    assert!(disabled(&app, "Pin variable 2 value"));
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.value, "");
    assert_eq!(row.saved, ["42", "77", "1001"]);
    app.harness
        .get_by_label("Variable 2 value in use, not pinned: Empty value");

    enter_value(&mut app, "1001");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "1001");
    assert_eq!(stored(&app).1, saved("user", &["42", "77", "1001"]));
}

#[test]
fn unsaved_edits_to_the_scope_block_choosing_and_a_clean_screen_follows_the_choice() {
    let mut app = with_saved_user("http://h".into());
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app.harness.state_mut().state.variables.rows[0].value = "http://edited".into();
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
        app.harness.state().state.variables.rows[1].value,
        "77",
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
fn the_current_chip_is_filled_and_an_unpinned_value_gets_a_ghost_chip() {
    let mut app = variables_app();
    let current = app
        .harness
        .get_by_label("Variable 2 saved value 1: 42")
        .rect();
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
            .query_by_label_contains("value in use, not pinned")
            .is_none()
    );

    enter_value(&mut app, "91");
    let ghost = app
        .harness
        .get_by_label("Variable 2 value in use, not pinned: 91");
    let area = ghost.rect();
    let shapes = &app.harness.output().shapes;
    assert!(shapes.iter().any(|clipped| matches!(&clipped.shape,
        egui::Shape::Text(text)
            if area.contains(text.pos)
                && text.galley.job.text == "91"
                && text.galley.job.sections.iter().any(|section| section.format.color == palette.text_muted)
    )));
    assert!(shapes.iter().filter(|clipped| matches!(&clipped.shape,
        egui::Shape::LineSegment { points, stroke }
            if points.iter().all(|point| area.contains(*point)) && stroke.color == palette.field_border
    )).count() > 4, "a dashed border");
    ghost.click();
    app.harness.run_steps(2);
    assert!(support::text_input(&app.harness, "Variable 2 value").is_focused());
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
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    let chip = format!("Variable 1 saved value 2: {prefix}…");
    app.harness.get_by_label(&chip).click_secondary();
    app.harness.run_steps(2);
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

    app.harness.get_by_label(&chip).click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Remove from list");
    assert!(app.harness.query_by_label(&chip).is_none());
    assert_eq!(app.harness.state().state.variables.rows[0].saved, ["42"]);
}

#[test]
fn a_hovered_remove_button_stays_in_place_and_asks_for_a_second_click() {
    let mut app = variables_app();
    let chip_name = "Variable 2 saved value 1: 42";
    let remove_name = "Remove variable 2 saved value 1";
    let chip = app.harness.get_by_label(chip_name);
    let before = chip.rect();
    assert!(app.harness.query_by_label(remove_name).is_none());
    chip.hover();
    app.harness.run_steps(2);
    let remove = app.harness.get_by_label(remove_name);
    assert!(remove.rect().left() > before.right());
    remove.hover();
    app.harness.run_steps(2);
    assert_eq!(app.harness.get_by_label(chip_name).rect(), before);
    click(&mut app.harness, remove_name);
    assert_eq!(
        app.harness.state().state.variables.rows[1].saved,
        ["42", "77"]
    );
    click(
        &mut app.harness,
        "Click again to remove variable 2 saved value 1",
    );
    let row = &app.harness.state().state.variables.rows[1];
    assert_eq!(row.value, "42");
    assert_eq!(row.saved, ["77"]);
    app.harness
        .get_by_label("Variable 2 value in use, not pinned: 42");
}

#[test]
fn tab_reaches_remove_and_two_keyboard_activations_remove_the_value() {
    let mut app = variables_app();
    let chip = "Variable 2 saved value 2: 77";
    app.harness.get_by_label(chip).focus();
    app.harness.run_steps(2);
    assert!(app.harness.get_by_label(chip).is_focused());

    app.harness.key_press(egui::Key::Tab);
    app.harness.run_steps(2);
    assert!(
        app.harness
            .get_by_label("Remove variable 2 saved value 2")
            .is_focused()
    );
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert!(
        app.harness
            .get_by_label("Click again to remove variable 2 saved value 2")
            .is_focused()
    );
    assert_eq!(
        app.harness.state().state.variables.rows[1].saved,
        ["42", "77"]
    );
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().state.variables.rows[1].saved, ["42"]);
}

#[test]
fn long_chips_wrap_under_the_field_and_row_remove_buttons_stay_aligned() {
    let mut app = variables_app();
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    let prefix = "x".repeat(60);
    let first = format!("{prefix}-one");
    let second = format!("{prefix}-two");
    app.harness.state_mut().state.variables.rows[1]
        .saved
        .extend([first, second.clone()]);
    for density in [Density::Compact, Density::Comfortable] {
        app.harness
            .state_mut()
            .state
            .actions
            .push(Action::SetAppearance(Appearance {
                theme: ThemeChoice::Dark,
                density,
            }));
        app.harness.run_steps(3);

        let left = |label| app.harness.get_by_label(label).rect().left();
        assert!((left("Remove variable 1") - left("Remove variable 2")).abs() < 0.5);
        let field = support::text_input(&app.harness, "Variable 2 value").rect();
        let long_names = [
            format!("Variable 2 saved value 3: {prefix}…"),
            format!("Variable 2 saved value 4: {prefix}…"),
        ];
        let [first, second] = long_names.map(|label| app.harness.get_by_label(&label).rect());
        assert!((first.left() - field.left()).abs() < 0.5);
        assert!((second.left() - field.left()).abs() < 0.5);
        assert!(first.bottom() < second.top());
        assert!(
            (second.height() - density.spacing().control).abs() < 0.5,
            "{density:?} chip height was {}",
            second.height()
        );
    }

    app.harness
        .get_by_label(&format!("Variable 2 saved value 4: {prefix}…"))
        .hover();
    app.harness.run_steps(8);
    app.harness.get_by_label(&second);
}
