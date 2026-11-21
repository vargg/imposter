//! Environment colors: chosen in the environment dialog, shown wherever the active
//! environment is named (the picker, Send's hint, the palette).
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{EnvColor, Variables};
use imposter_app::store::environments;
use imposter_ui::app::Screen;
use imposter_ui::icons::Icon;
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::env_color;
use imposter_ui::vars::Scope;
use support::{click, click_button, collection, request, settle, start_with};

fn stored(app: &support::App) -> Vec<(String, Option<EnvColor>)> {
    app.harness
        .state()
        .store()
        .call_blocking(environments::environments)
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
    ["No color", "Red", "Yellow", "Green", "Blue", "Purple"]
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
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "New environment");
    assert_eq!(chosen(&app), ["No color"]);

    support::text_input(&app.harness, "Name").type_text("Prod");
    app.harness.run_steps(2);
    assert_eq!(chosen(&app), ["Red"], "a production name suggests red");
    let has_suggestion = |app: &support::App| {
        app.harness.get_all_by_label("Help").any(|node| {
            node.accesskit_node().description().as_deref()
                == Some("Suggested for production environments")
        })
    };
    assert!(
        has_suggestion(&app),
        "the suggested color has contextual help"
    );
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
    assert!(!has_suggestion(&app), "a chosen color has no suggestion");
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
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
    support::open_environment_menu(&mut app.harness, "Prod");
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
    click(&mut app.harness, "Edit name and color…");
    assert_eq!(chosen(&app), ["Red"]);
    support::text_input(&app.harness, "Name").type_text("uction");
    app.harness.run_steps(2);
    assert_eq!(
        chosen(&app),
        ["Red"],
        "renaming never changes the color by itself"
    );
    click(&mut app.harness, "No color");
    click_button(&mut app.harness, "Save");
    settle(&mut app.harness);

    assert_eq!(
        stored(&app),
        [("Dev".to_owned(), None), ("Production".to_owned(), None)]
    );
}

#[test]
fn a_focused_environment_can_open_its_menu_with_the_keyboard() {
    let mut app = with_environments(false);
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app.harness.get_by_label("Environment · Prod").focus();
    app.harness.run_steps(2);
    app.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
    app.harness.run_steps(2);
    assert!(
        app.harness
            .get_by_label("Edit name and color…")
            .is_focused()
    );
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}

#[test]
fn hovering_an_environment_reveals_its_menu_without_selecting_the_scope() {
    let mut app = with_environments(false);
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    let label = "Actions for environment Prod";
    let menu = app
        .harness
        .get_by_role_and_label(Role::Button, label)
        .rect();
    let more = Icon::More.glyph().to_string();
    let painted = |app: &support::App| {
        app.harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if menu.contains(text.pos) && text.galley.job.text == more)
        })
    };
    assert!(!painted(&app));
    app.harness.get_by_label("Environment · Prod").hover();
    app.harness.run_steps(2);
    assert!(painted(&app));
    click_button(&mut app.harness, label);
    app.harness.get_by_label("Edit name and color…");
    app.harness.get_by_label("Delete environment…");
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}

fn send_description(app: &support::App) -> Option<String> {
    app.harness
        .get_by_role_and_label(Role::Button, "Send")
        .accesskit_node()
        .description()
}

fn send_geometry(app: &support::App) -> (egui::Rect, egui::Rect) {
    (
        app.harness
            .get_by_role_and_label(Role::Button, "Send")
            .rect(),
        app.harness.get_by_label("Request URL").rect(),
    )
}

fn assert_send_hint(app: &mut support::App, name: &str) {
    let hint = imposter_ui::shortcuts::hint(
        &app.harness.ctx,
        &format!("Send with {name}"),
        &imposter_ui::shortcuts::SEND,
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Send")
        .hover();
    app.harness.run_steps(45);
    app.harness.get_by_role_and_label(Role::Label, &hint);
}

/// Painted rectangles filled with `fill`.
fn filled(app: &support::App, fill: egui::Color32) -> Vec<egui::Rect> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == fill => Some(rect.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn a_colored_environment_fills_the_picker_and_send_hints_name_the_environment() {
    let mut app = with_environments(true);
    let red = env_color(EnvColor::Red);
    let tint = red.gamma_multiply(imposter_ui::widgets::PILL_TINT);
    // The picker: its name in the color on its tint, after a stripe in the full color.
    assert_eq!(painted_in(&app, red), ["● Prod"]);
    let picker = app.harness.get_by_label("Environment").rect();
    assert!(
        filled(&app, tint)
            .iter()
            .any(|fill| fill.contains_rect(picker.shrink(1.0))),
        "the picker is on the tint"
    );
    assert!(
        filled(&app, red).iter().any(|stripe| stripe.width() <= 3.0
            && (stripe.left() - picker.left()).abs() < 1.0
            && (stripe.height() - picker.height()).abs() < 1.0),
        "a stripe on the picker's left edge"
    );
    let prod_geometry = send_geometry(&app);
    assert!((96.0..=96.5).contains(&prod_geometry.0.width()));
    assert_eq!(
        send_description(&app).as_deref(),
        Some("Sends with the Prod environment")
    );
    assert!(app.harness.query_by_label("Prod").is_none(), "named once");
    assert_eq!(red_dots(&app), 0);
    // No colored rim: a red edge read as an error, a green one as success.
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
    assert_send_hint(&mut app, "Prod");

    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Dev");
    settle(&mut app.harness);
    assert_eq!(send_geometry(&app), prod_geometry);
    assert_eq!(painted_in(&app, red), Vec::<String>::new());
    assert_eq!(filled(&app, tint), Vec::<egui::Rect>::new(), "no tint");
    assert_eq!(
        send_description(&app).as_deref(),
        Some("Sends with the Dev environment")
    );
    assert!(app.harness.query_by_label("Dev").is_none(), "named once");
    assert_send_hint(&mut app, "Dev");

    // Without an environment Send names none.
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Global (no environment)");
    settle(&mut app.harness);
    assert_eq!(send_geometry(&app), prod_geometry);
    let description = send_description(&app).unwrap_or_default();
    assert!(!description.contains("environment"), "{description}");
}

#[test]
fn a_long_colored_environment_name_keeps_send_and_the_url_in_place() {
    let name = "Production Europe West Primary for Customer Accounts and Billing Services";
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                let prod = environments::create_environment(db, "Prod", &Variables::new())?;
                environments::set_environment_color(db, prod.id, Some(EnvColor::Red))?;
                let long = environments::create_environment(db, name, &Variables::new())?;
                environments::set_environment_color(db, long.id, Some(EnvColor::Red))?;
                environments::set_active_environment(db, Some(prod.id))
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://h");
    });
    app.harness.set_size(MIN_WINDOW.into());
    click(&mut app.harness, "GET Get");
    settle(&mut app.harness);
    let short_geometry = send_geometry(&app);
    click(&mut app.harness, "Environment");
    click(&mut app.harness, name);
    settle(&mut app.harness);

    let (send, url) = send_geometry(&app);
    assert_eq!((send, url), short_geometry);
    assert!((96.0..=96.5).contains(&send.width()), "{send:?}");
    assert!(url.width() >= 200.0, "{url:?}");
    let text = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text)
                if send.contains(text.pos) && text.galley.job.text == "Send" =>
            {
                Some(text)
            }
            _ => None,
        })
        .unwrap();
    assert!(!text.galley.elided);
    assert!((text.pos.x + text.galley.size().x / 2.0 - send.center().x).abs() <= 1.0);
    assert_eq!(
        send_description(&app).unwrap(),
        format!("Sends with the {name} environment")
    );
    assert_send_hint(&mut app, name);

    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    let label = format!("Environment · {name}");
    let row = app.harness.get_by_label(&label);
    assert!(app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Text(text)
            if row.rect().contains(text.pos) && text.galley.job.text == name && text.galley.elided)
    }));
    row.hover();
    app.harness.run_steps(12);
    app.harness.get_by_role_and_label(
        Role::Label,
        &format!("{name}\nActive: requests use this environment"),
    );
}

#[test]
fn the_picker_and_the_palette_mark_colored_environments() {
    let mut app = with_environments(false);
    let red = env_color(EnvColor::Red);
    click(&mut app.harness, "Environment");
    assert_eq!(painted_in(&app, red), ["● "], "only Prod's option");
    // Options without a color keep the dot's room, so every name starts in line.
    let options: Vec<String> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with("● ") => {
                Some(text.galley.job.text.clone())
            }
            _ => None,
        })
        .collect();
    for option in ["● Global (no environment)", "● Dev", "● Prod"] {
        assert!(options.iter().any(|o| o == option), "{option}: {options:?}");
    }
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
    support::open_screen(&mut app.harness, "Requests");
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Global (no environment)");
    settle(&mut app.harness);
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Manage environments…");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}
