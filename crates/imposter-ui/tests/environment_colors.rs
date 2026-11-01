//! Environment colors: chosen in the environment dialog, shown wherever the active
//! environment is (picker, request bar, status bar, palette).
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{EnvColor, Variables};
use imposter_app::store::environments;
use imposter_ui::app::Screen;
use imposter_ui::theme::env_color;
use imposter_ui::vars::Scope;
use support::{click, click_button, collection, request, settle, start_with};

fn stored(app: &support::App) -> Vec<(String, Option<EnvColor>)> {
    app.harness
        .state()
        .store()
        .call_blocking(|db| environments::environments(db))
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.color))
        .collect()
}

fn swatch(app: &support::App, label: &str) -> bool {
    app.harness
        .get_by_role_and_label(Role::RadioButton, label)
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

fn chosen(app: &support::App) -> Vec<&'static str> {
    [
        "No color", "Red", "Orange", "Yellow", "Green", "Blue", "Purple",
    ]
    .into_iter()
    .filter(|label| swatch(app, label))
    .collect()
}

/// The text of every painted run whose color is `color`.
fn painted_in(app: &support::App, color: egui::Color32) -> Vec<String> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.job.clone()),
            _ => None,
        })
        .flat_map(|job| {
            job.sections
                .iter()
                .filter(|section| section.format.color == color)
                .map(|section| {
                    job.text[section.byte_range.start.0..section.byte_range.end.0].to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn red_dots(app: &support::App) -> usize {
    let red = env_color(EnvColor::Red);
    app.harness
        .output()
        .shapes
        .iter()
        .filter(|clipped| matches!(&clipped.shape, egui::Shape::Circle(c) if c.fill == red))
        .count()
}

/// An API collection with one request, and environments Prod (red) and Dev (no color).
fn with_environments(active: bool) -> support::App {
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                let prod = environments::create_environment(db, "Prod", &Variables::new())?;
                environments::set_environment_color(db, prod.id, Some(EnvColor::Red))?;
                environments::create_environment(db, "Dev", &Variables::new())?;
                if active {
                    environments::set_active_environment(db, Some(prod.id))?;
                }
                Ok(())
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h");
    });
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    app
}

#[test]
fn a_new_environment_named_for_production_is_offered_red_until_a_color_is_chosen() {
    let mut app = start_with(|_| {});
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "New environment");
    assert_eq!(chosen(&app), ["No color"]);

    support::text_input(&app.harness, "Name").type_text("Prod");
    app.harness.run_steps(2);
    assert_eq!(chosen(&app), ["Red"], "a production name suggests red");
    support::text_input(&app.harness, "Name").type_text("ucts");
    app.harness.run_steps(2);
    assert_eq!(
        chosen(&app),
        ["No color"],
        "the suggestion follows the name"
    );
    support::text_input(&app.harness, "Name").type_text(" live");
    app.harness.run_steps(2);
    assert_eq!(chosen(&app), ["Red"]);

    click(&mut app.harness, "Blue");
    support::text_input(&app.harness, "Name").type_text(" eu");
    app.harness.run_steps(2);
    assert_eq!(chosen(&app), ["Blue"], "a chosen color stays while typing");
    click_button(&mut app.harness, "Create");
    settle(&mut app.harness);

    assert_eq!(
        stored(&app),
        [("Products live eu".to_owned(), Some(EnvColor::Blue))]
    );
}

#[test]
fn renaming_shows_the_color_and_can_clear_it() {
    let mut app = with_environments(false);
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "Variable scope");
    click(&mut app.harness, "Environment · Prod");
    settle(&mut app.harness);

    click(&mut app.harness, "Rename environment");
    assert_eq!(chosen(&app), ["Red"]);
    support::text_input(&app.harness, "Name").type_text("uction");
    app.harness.run_steps(2);
    assert_eq!(
        chosen(&app),
        ["Red"],
        "renaming never changes the color by itself"
    );
    click(&mut app.harness, "No color");
    click_button(&mut app.harness, "Rename");
    settle(&mut app.harness);

    assert_eq!(
        stored(&app),
        [("Dev".to_owned(), None), ("Production".to_owned(), None)]
    );
}

#[test]
fn the_active_environment_color_shows_in_the_picker_request_bar_and_status_bar() {
    let mut app = with_environments(true);
    let red = env_color(EnvColor::Red);
    // The picker writes a red dot before the name; the status bar paints one.
    assert_eq!(painted_in(&app, red), ["● "]);
    assert_eq!(red_dots(&app), 1);
    let name = app.harness.get_by_label("Prod").rect();
    let dot = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(c) if c.fill == red => Some(c.center),
            _ => None,
        })
        .unwrap();
    assert!(
        dot.x < name.left() && (dot.y - name.center().y).abs() < 3.0,
        "the dot comes before the name: {dot:?} {name:?}"
    );
    // The request bar is rimmed in red.
    let rims = app
        .harness
        .output()
        .shapes
        .iter()
        .filter(|clipped| {
            matches!(&clipped.shape, egui::Shape::Rect(rect)
                if rect.stroke.color == red && rect.stroke.width >= 2.0)
        })
        .count();
    assert_eq!(rims, 1);

    // Dev has no color: nothing is red, and the bar has its usual edge.
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Dev");
    settle(&mut app.harness);
    assert_eq!(painted_in(&app, red), Vec::<String>::new());
    assert_eq!(red_dots(&app), 0);
    let rims = app
        .harness
        .output()
        .shapes
        .iter()
        .filter(
            |clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.stroke.color == red),
        )
        .count();
    assert_eq!(rims, 0);
}

#[test]
fn the_picker_and_the_palette_mark_colored_environments() {
    let mut app = with_environments(false);
    let red = env_color(EnvColor::Red);
    click(&mut app.harness, "Environment");
    assert_eq!(painted_in(&app, red), ["● "], "only Prod's option");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);

    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    app.harness.run_steps(2);
    support::text_input(&app.harness, "Command palette search").type_text("prod");
    app.harness.run_steps(2);
    assert_eq!(painted_in(&app, red), ["● "]);
    app.harness.get_by_label("Prod");
}

#[test]
fn manage_environments_opens_variables_on_the_active_environment() {
    let mut app = with_environments(true);
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Manage environments…");
    settle(&mut app.harness);

    let state = &app.harness.state().state;
    assert_eq!(state.screen, Screen::Variables);
    let prod = state
        .environments
        .iter()
        .find(|e| e.name == "Prod")
        .unwrap();
    assert_eq!(state.variables.scope, Scope::Environment(prod.id));

    // Without an active environment, Global.
    click(&mut app.harness, "Requests");
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Global (no environment)");
    settle(&mut app.harness);
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Manage environments…");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}
