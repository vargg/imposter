//! Workspace branding and navigation through the real left rail.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its frame loop

mod support;

use egui::{Key, Modifiers, Shape, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::mock::{MockConfig, MockRoute};
use imposter_app::core::model::{EnvColor, Variables};
use imposter_app::store::{environments, kv};
use imposter_ui::app::{Action, Appearance, Screen};
use imposter_ui::icons::Icon;
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::{self, Density, ThemeChoice, Tone};
use support::{App, click, open_screen, settle, start, start_with, type_into};

const RAIL: [(Screen, &str, Icon); 7] = [
    (Screen::Requests, "Requests", Icon::Requests),
    (Screen::Variables, "Variables", Icon::Variables),
    (Screen::History, "History", Icon::History),
    (Screen::Mocks, "Mocks", Icon::Mocks),
    (Screen::Load, "Load", Icon::Load),
    (Screen::Run, "Run", Icon::Run),
    (Screen::Settings, "Settings", Icon::Settings),
];

fn item<'a>(app: &'a App, label: &'a str) -> egui_kittest::Node<'a> {
    app.harness.get_by_role_and_label(Role::Button, label)
}

fn painted_text(app: &App, label: &str) -> bool {
    app.harness.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, Shape::Text(text)
            if text.galley.job.text == label && !text.galley.elided)
    })
}

fn assert_markers(app: &App, label: &str, description: Option<&str>, tones: &[Tone]) {
    let button = item(app, label);
    assert_eq!(
        button.accesskit_node().description().as_deref(),
        description
    );
    let dots: Vec<_> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Circle(dot) if button.rect().contains(dot.center) => Some(dot),
            _ => None,
        })
        .collect();
    let colors: Vec<_> = dots.iter().map(|dot| dot.fill).collect();
    assert_eq!(
        colors,
        tones
            .iter()
            .map(|tone| theme::palette().tone(*tone))
            .collect::<Vec<_>>(),
        "{label}"
    );
    assert!(
        dots.windows(2)
            .all(|pair| pair[0].center.distance(pair[1].center) > pair[0].radius + pair[1].radius),
        "both markers on {label} must be visible"
    );
}

#[test]
fn the_rail_selects_each_screen_and_paints_its_glyphs_and_selection_bar() {
    let mut app = start();
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    app.harness.run_steps(2);
    assert!(painted_text(&app, "Global (no environment)"));
    assert!(app.harness.query_by_label("Tools").is_none());
    for (screen, label, _) in RAIL {
        open_screen(&mut app.harness, label);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
        if matches!(screen, Screen::Mocks | Screen::Load | Screen::Run) {
            app.harness.get_by_role_and_label(Role::Label, label);
        }
        assert!(!egui::Popup::is_any_open(&app.harness.ctx));
        app.harness.hover_at(egui::pos2(300.0, 20.0));
        app.harness.run_steps(2);
        let p = theme::palette();
        let shapes = &app.harness.output().shapes;
        for (destination, name, icon) in RAIL {
            let selected = destination == screen;
            let button = item(&app, name);
            let rect = button.rect();
            assert_eq!(
                button.accesskit_node().is_selected(),
                Some(selected),
                "{name}"
            );
            assert!(shapes.iter().any(|clipped| {
                matches!(&clipped.shape, Shape::Text(text)
                    if rect.contains(text.pos)
                        && text.galley.job.text == icon.glyph().to_string()
                        && text.galley.job.sections[0].format.font_id.family == egui::FontFamily::Name(theme::ICONS.into())
                        && text.fallback_color == if selected { p.accent } else { p.text_muted })
            }), "glyph on {name}");
            assert!(
                !shapes.iter().any(|clipped| {
                    matches!(&clipped.shape, Shape::Rect(shape) if shape.rect == rect)
                }),
                "{name} is frameless"
            );
        }
        let selected = item(&app, label).rect();
        let bars: Vec<_> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Rect(bar) if bar.fill == p.selected_bar && bar.rect.left() == 0.0 => {
                    Some(bar.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 1, "one bar on the rail's left edge");
        assert!((bars[0].width() - 2.0).abs() < f32::EPSILON);
        assert!(selected.y_range().contains(bars[0].center().y));
    }
}

#[test]
fn the_rail_works_from_the_keyboard_and_preserves_palette_actions() {
    let mut app = start();
    item(&app, "Requests").focus();
    app.harness.run_steps(2);
    for (_, label, _) in &RAIL[1..] {
        app.harness.key_press(Key::Tab);
        app.harness.run_steps(2);
        assert!(item(&app, label).is_focused(), "Tab should reach {label}");
    }
    app.harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    app.harness.run_steps(2);
    assert!(item(&app, "Run").is_focused());
    app.harness.key_press(Key::Space);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Run);

    item(&app, "Load").focus();
    app.harness.run_steps(2);
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Load);

    item(&app, "Settings").focus();
    app.harness.run_steps(2);
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Settings);
    for (query, screen) in [
        ("Go to Mocks", Screen::Mocks),
        ("Go to Load", Screen::Load),
        ("Go to Run", Screen::Run),
    ] {
        app.harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
        settle(&mut app.harness);
        type_into(&mut app.harness, "Command palette search", query);
        app.harness.key_press(Key::Enter);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
    }
}

#[test]
fn the_rail_tooltips_name_the_destination_and_its_shortcut() {
    use imposter_ui::shortcuts;

    let mut app = start();
    app.harness
        .ctx
        .global_style_mut(|style| style.interaction.tooltip_delay = 0.0);
    let settings_hint = shortcuts::hint(&app.harness.ctx, "Settings", &shortcuts::SETTINGS);
    for (label, hint, icon) in [
        ("Variables", "Variables", Icon::Variables),
        ("Settings", settings_hint.as_str(), Icon::Settings),
    ] {
        let rect = item(&app, label).rect();
        app.harness.hover_at(rect.center());
        app.harness.run_steps(5);
        assert!(painted_text(&app, hint), "tooltip on {label}");
        assert!(
            app.harness.output().shapes.iter().any(|clipped| {
                matches!(&clipped.shape, Shape::Text(text)
                if rect.contains(text.pos) && text.galley.job.text == icon.glyph().to_string()
                    && text.fallback_color == theme::palette().text)
            }),
            "hovered {label} is painted in the text color"
        );
    }
}

#[test]
fn variables_and_settings_keep_unsaved_markers_on_other_screens() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                environments::set_globals(
                    db,
                    &[("host".to_owned(), "dev".to_owned())]
                        .into_iter()
                        .collect(),
                )
            })
            .unwrap();
    });
    open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Add value to variable 1", "test");
    open_screen(&mut app.harness, "Settings");
    settle(&mut app.harness);
    type_into(&mut app.harness, "Proxy server", "http://localhost:8080");
    open_screen(&mut app.harness, "Requests");
    for label in ["Variables", "Settings"] {
        assert_markers(&app, label, Some("Unsaved changes"), &[Tone::Warning]);
    }

    for (label, save) in [
        ("Settings", "Save settings"),
        ("Variables", "Save variables"),
    ] {
        open_screen(&mut app.harness, label);
        settle(&mut app.harness);
        click(&mut app.harness, save);
        settle(&mut app.harness);
        let hint = if label == "Settings" {
            imposter_ui::shortcuts::hint(&app.harness.ctx, label, &imposter_ui::shortcuts::SETTINGS)
        } else {
            label.to_owned()
        };
        assert_markers(&app, label, Some(&hint), &[]);
    }
}

#[test]
fn mocks_can_show_activity_and_unsaved_changes_at_the_same_time() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set(
                    db,
                    kv::Table::Settings,
                    "mock_server",
                    &MockConfig {
                        port: 0,
                        routes: vec![MockRoute::default()],
                    },
                )
            })
            .unwrap();
    });
    open_screen(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    click(&mut app.harness, "Start mock server");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .harness
        .state()
        .state
        .mocks
        .run
        .as_ref()
        .is_none_or(|run| run.address.is_none())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "mock server did not start"
        );
        app.harness.run_steps(2);
        std::thread::yield_now();
    }
    click(&mut app.harness, "Mock route 1");
    type_into(&mut app.harness, "Mock response body", "unsaved");
    open_screen(&mut app.harness, "Requests");
    assert_markers(
        &app,
        "Mocks",
        Some("Running · Unsaved changes"),
        &[Tone::Warning, Tone::Ok],
    );

    // Hold the presentation state while the listener is still alive, so its stop
    // notification cannot race with the frame that paints the stopping marker.
    app.harness
        .state_mut()
        .state
        .mocks
        .run
        .as_mut()
        .unwrap()
        .stopping = true;
    app.harness.run_steps(2);
    assert_markers(
        &app,
        "Mocks",
        Some("Stopping… · Unsaved changes"),
        &[Tone::Warning, Tone::Muted],
    );
    app.harness
        .state_mut()
        .state
        .mocks
        .run
        .as_mut()
        .unwrap()
        .stopping = false;
    open_screen(&mut app.harness, "Mocks");
    click(&mut app.harness, "Stop mock server");
    settle(&mut app.harness);
    assert_markers(&app, "Mocks", Some("Unsaved changes"), &[Tone::Warning]);
}

#[test]
fn branding_and_the_rail_fit_with_a_long_environment_in_both_densities() {
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
    for (size, wide) in [
        (egui::Vec2::from(MIN_WINDOW), false),
        (egui::vec2(1500.0, 850.0), true),
    ] {
        app.harness.set_size(size);
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            for density in [Density::Compact, Density::Comfortable] {
                app.harness
                    .state_mut()
                    .state
                    .actions
                    .push(Action::SetAppearance(Appearance { theme, density }));
                app.harness.run_steps(3);

                let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let logo = app
                    .harness
                    .get_by_role_and_label(Role::Image, "I’mposter")
                    .rect();
                let logo_texture = app.harness.state().state.logo.as_ref().unwrap().id();
                assert!(app.harness.output().shapes.iter().any(|clipped| {
                        matches!(&clipped.shape, Shape::Rect(mark) if mark.fill_texture_id() == logo_texture)
                }), "the mark is always painted");
                assert_eq!(painted_text(&app, "I’mposter"), wide);
                assert!(!painted_text(&app, "I’mp"));
                let environment = app
                    .harness
                    .get_by_role_and_label(Role::ComboBox, "Environment")
                    .rect();
                assert!(window.contains_rect(logo) && window.contains_rect(environment));
                assert!(logo.right() < environment.left() && environment.right() > size.x - 32.0);
                assert!(environment.width() >= 24.0 && environment.height() >= 24.0);

                let rects = RAIL.map(|(_, label, _)| {
                    support::assert_icon_button_in_range(&app.harness, label, 32.0..=36.0);
                    let rect = item(&app, label).rect();
                    assert!(window.contains_rect(rect), "{label}: {rect:?}");
                    assert!((rect.center().x - 24.0).abs() <= 1.0, "{label}: {rect:?}");
                    rect
                });
                assert!(logo.bottom() < rects[0].top());
                assert!(
                    rects
                        .windows(2)
                        .all(|pair| pair[0].bottom() < pair[1].top())
                );
                assert!(rects[3].top() - rects[2].bottom() > rects[2].top() - rects[1].bottom());
                let status_top = support::status_bar(&app).top();
                assert!((8.0..=16.0).contains(&(status_top - rects[6].bottom())));
            }
        }
    }
}

#[test]
fn every_screen_has_one_toolbar_above_its_content_and_the_status_bar_is_status_only() {
    let mut app = start();
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    for (screen, title, content_role, content, action) in [
        (
            Screen::Requests,
            "Requests",
            Role::Label,
            "Start with a collection",
            "Stack request and response",
        ),
        (
            Screen::Variables,
            "Variables",
            Role::Label,
            "Variable name",
            "Save variables",
        ),
        (Screen::History, "History", Role::Label, "When", "Refresh"),
        (
            Screen::Mocks,
            "Mocks",
            Role::Button,
            "Add mock route",
            "Save mocks",
        ),
        (
            Screen::Load,
            "Load",
            Role::TextInput,
            "Total requests",
            "Start the load test",
        ),
        (
            Screen::Run,
            "Run",
            Role::CheckBox,
            "Keep going after a failure",
            "Start the run",
        ),
        (
            Screen::Settings,
            "Settings",
            Role::Label,
            "Appearance",
            "Save settings",
        ),
    ] {
        open_screen(&mut app.harness, title);
        settle(&mut app.harness);
        assert_eq!(app.harness.state().state.screen, screen);
        let toolbar = app.harness.get_by_role(Role::Toolbar);
        let row = toolbar.rect();
        let (heading_role, heading_name, height) = if screen == Screen::Variables {
            (
                Role::Button,
                "Variables tab",
                imposter_ui::widgets::tab_height(),
            )
        } else {
            (Role::Label, title, theme::spacing().control)
        };
        let heading = toolbar.get_by_role_and_label(heading_role, heading_name);
        let action = toolbar.get_by_role_and_label(Role::Button, action);
        let content = app
            .harness
            .get_by_role_and_label(content_role, content)
            .rect();
        assert!(row.contains_rect(heading.rect()), "{title}");
        assert!(row.contains_rect(action.rect()), "{title}");
        assert!((row.height() - height).abs() <= 1.0, "{title}: {row:?}");
        assert!(heading.rect().right() < action.rect().left(), "{title}");
        assert!(
            row.bottom() < content.top(),
            "{title}: {row:?}, {content:?}"
        );
        assert!(
            app.harness.output().shapes.iter().any(|clipped| {
                matches!(clipped.shape, Shape::LineSegment { points: [a, b], stroke }
                if (a.x - row.left()).abs() < 0.5
                    && (b.x - row.right()).abs() < 0.5
                    && row.bottom() < a.y
                    && a.y < content.top()
                    && (a.y - b.y).abs() < f32::EPSILON
                    && (stroke.width - 1.0).abs() < f32::EPSILON
                    && stroke.color == theme::palette().field_border)
            }),
            "{title} toolbar divider"
        );

        let status = app
            .harness
            .get_by_role_and_label(Role::Status, "Status bar");
        for label in [
            "Hide Collections",
            "Show Collections",
            "Put request and response side by side",
            "Stack request and response",
        ] {
            assert!(status.query_by_label(label).is_none(), "{title}: {label}");
            assert_eq!(
                toolbar.query_by_label(label).is_some(),
                screen == Screen::Requests && label != "Show Collections",
                "{title}: {label}"
            );
        }
    }
}
