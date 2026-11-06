//! Workspace navigation and the supporting tools use the real header and menus.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its frame loop

mod support;

use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{EnvColor, Variables};
use imposter_app::store::environments;
use imposter_ui::app::{Action, Appearance, Screen};
use imposter_ui::theme::{Density, ThemeChoice};
use support::{click, open_screen, settle, start, start_with, type_into};

#[test]
fn the_workspace_stays_visible_and_tools_are_named_in_the_menu() {
    let mut app = start();
    app.harness.set_size(egui::vec2(720.0, 480.0));
    app.harness.run_steps(2);
    assert!(app.harness.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.job.text == "Global (no environment)" && !text.galley.elided)
    }));
    for label in ["Requests", "Variables", "History", "Tools", "Settings"] {
        app.harness.get_by_role_and_label(Role::Button, label);
    }
    for label in ["Mocks", "Load"] {
        assert!(
            app.harness
                .query_by_role_and_label(Role::Button, label)
                .is_none()
        );
    }
    support::assert_icon_button(&app.harness, "Settings");
    for (label, screen) in [("Mocks", Screen::Mocks), ("Load", Screen::Load)] {
        open_screen(&mut app.harness, label);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
        app.harness.get_by_role_and_label(Role::Label, label);
        assert_eq!(
            app.harness
                .get_by_label("Tools")
                .accesskit_node()
                .is_selected(),
            Some(true)
        );
        assert!(!egui::Popup::is_any_open(&app.harness.ctx));
    }
    for (label, screen) in [
        ("Variables", Screen::Variables),
        ("History", Screen::History),
        ("Requests", Screen::Requests),
    ] {
        click(&mut app.harness, label);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
        assert_eq!(
            app.harness
                .get_by_label("Tools")
                .accesskit_node()
                .is_selected(),
            Some(false)
        );
    }
}

#[test]
fn tools_and_settings_work_from_the_keyboard_and_palette() {
    let mut app = start();
    app.harness.get_by_label("Requests").focus();
    app.harness.run_steps(2);
    for label in ["Variables", "History", "Environment", "Tools", "Settings"] {
        app.harness.key_press(Key::Tab);
        app.harness.run_steps(2);
        assert!(
            app.harness.get_by_label(label).is_focused(),
            "Tab should reach {label}"
        );
    }
    app.harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    app.harness.run_steps(2);
    assert!(app.harness.get_by_label("Tools").is_focused());
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    app.harness.get_by_role_and_label(Role::Button, "Mocks");
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(2);
    assert!(!egui::Popup::is_any_open(&app.harness.ctx));
    assert_eq!(app.harness.state().state.screen, Screen::Requests);

    app.harness.get_by_label("Tools").focus();
    app.harness.run_steps(2);
    app.harness.key_press(Key::Space);
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(Role::Button, "Load")
        .focus();
    app.harness.run_steps(2);
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Load);

    app.harness.get_by_label("Settings").focus();
    app.harness.run_steps(2);
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Settings);
    for (query, screen) in [("Go to Mocks", Screen::Mocks), ("Go to Load", Screen::Load)] {
        app.harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
        settle(&mut app.harness);
        type_into(&mut app.harness, "Command palette search", query);
        app.harness.key_press(Key::Enter);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
    }
}

#[test]
fn settings_keeps_its_unsaved_marker_when_another_screen_is_open() {
    let mut app = start();
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Proxy server", "http://localhost:8080");
    click(&mut app.harness, "Requests");
    assert_eq!(
        app.harness
            .get_by_label("Settings")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Unsaved changes")
    );
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    click(&mut app.harness, "Save settings");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .get_by_label("Settings")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Settings")
    );
}

#[test]
fn long_environments_leave_every_header_control_visible_in_both_densities() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                let env = environments::create_environment(
                    db,
                    "Production — Europe West with a long environment name",
                    &Variables::new(),
                )?;
                environments::set_environment_color(db, env.id, Some(EnvColor::Red))?;
                environments::set_active_environment(db, Some(env.id))
            })
            .unwrap();
    });
    for size in [egui::vec2(720.0, 480.0), egui::vec2(1500.0, 850.0)] {
        app.harness.set_size(size);
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            for density in [Density::Compact, Density::Comfortable] {
                app.harness
                    .state_mut()
                    .state
                    .actions
                    .push(Action::SetAppearance(Appearance { theme, density }));
                app.harness.run_steps(3);
                let controls: Vec<_> = [
                    "Requests",
                    "Variables",
                    "History",
                    "Environment",
                    "Tools",
                    "Settings",
                ]
                .into_iter()
                .map(|label| (label, app.harness.get_by_label(label).rect()))
                .collect();
                let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                for (i, (label, rect)) in controls.iter().enumerate() {
                    assert!(
                        window.contains_rect(*rect),
                        "{size:?} {theme:?} {density:?}: {label} {rect:?}"
                    );
                    assert!(
                        rect.width() >= 24.0 && rect.height() >= 24.0,
                        "{label}: {rect:?}"
                    );
                    for (other, other_rect) in &controls[i + 1..] {
                        assert!(!rect.intersects(*other_rect), "{label} overlaps {other}");
                    }
                }
                assert!(
                    controls
                        .windows(2)
                        .all(|pair| pair[0].1.right() <= pair[1].1.left())
                );
                click(&mut app.harness, "Tools");
                app.harness.run_steps(2);
                for label in ["Mocks", "Load"] {
                    assert!(
                        window.contains_rect(
                            app.harness
                                .get_by_role_and_label(Role::Button, label)
                                .rect()
                        )
                    );
                }
                app.harness.key_press(Key::Escape);
                app.harness.run_steps(2);
            }
        }
    }
}
