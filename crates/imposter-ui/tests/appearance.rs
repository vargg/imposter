//! Appearance: the density setting packs lists and controls, applies at once and is
//! remembered.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use imposter_app::store::kv;
use imposter_ui::app::{Action, Appearance};
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::{Density, ThemeChoice};
use support::{click, collection, request, settle, start_with};

fn many_requests() -> support::App {
    start_with(|store| {
        let api = collection(store, "API");
        for i in 0..20 {
            request(store, api.id, &format!("Request {i:02}"), "http://x");
        }
    })
}

fn tops(app: &support::App) -> Vec<f32> {
    let mut tops: Vec<f32> = app
        .harness
        .query_all_by_label_contains("GET Request ")
        .map(|node| node.rect().top())
        .collect();
    tops.sort_by(f32::total_cmp);
    tops
}

fn set_density(app: &mut support::App, density: Density) {
    app.harness
        .state_mut()
        .state
        .actions
        .push(Action::SetAppearance(Appearance {
            theme: ThemeChoice::Dark,
            density,
        }));
    app.harness.run_steps(3);
}

#[test]
fn rows_follow_the_density() {
    let mut app = many_requests();
    for density in [Density::Compact, Density::Comfortable] {
        set_density(&mut app, density);
        let tops = tops(&app);
        let pitch = tops[1] - tops[0];
        let row = density.spacing().row;
        assert!(
            (pitch - (row + 2.0)).abs() <= 1.0,
            "{density:?}: rows {pitch} apart, expected {row} + 2"
        );
    }
}

#[test]
fn compact_shows_more_of_the_tree_at_the_smallest_window() {
    let mut app = many_requests();
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    app.harness.run_steps(3);
    let visible = |app: &support::App| {
        let bottom = app.harness.get_by_label("Hide Collections").rect().top();
        tops(app).iter().filter(|top| **top + 20.0 < bottom).count()
    };
    let compact = visible(&app);
    assert!(
        compact >= 10,
        "{compact} requests visible (7 before densities)"
    );
    set_density(&mut app, Density::Comfortable);
    assert!(visible(&app) < compact, "comfortable shows fewer");
}

#[test]
fn the_density_is_chosen_in_settings_and_remembered() {
    let mut app = many_requests();
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Density")
        .click();
    app.harness.run_steps(2);
    click(&mut app.harness, "Comfortable");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .ctx
            .style_of(egui::Theme::Dark)
            .spacing
            .interact_size
            .y,
        Density::Comfortable.spacing().control
    );
    let stored: serde_json::Value = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "appearance"))
        .unwrap()
        .unwrap();
    assert_eq!(stored["density"], "comfortable");
    assert_eq!(stored["theme"], "dark", "the theme is kept");
}

/// The shapes painted last frame that lie within `area`.
fn shapes_in(app: &support::App, area: egui::Rect) -> Vec<egui::Shape> {
    app.harness
        .output()
        .shapes
        .iter()
        .map(|clipped| clipped.shape.clone())
        .filter(|shape| area.expand(1.0).contains_rect(shape.visual_bounding_rect()))
        .collect()
}

#[test]
fn hovering_a_tab_previews_its_underline_without_a_button_frame() {
    let mut app = many_requests();
    let tab = app.harness.get_by_role_and_label(Role::Button, "History");
    let area = tab.rect();
    tab.hover();
    app.harness.run_steps(2);
    let shapes = shapes_in(&app, area);
    assert!(
        !shapes
            .iter()
            .any(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.rect.height() > area.height() - 2.0)),
        "no frame: {shapes:?}"
    );
    assert!(
        shapes.iter().any(|shape| matches!(
            shape,
            egui::Shape::LineSegment { points, .. } if points[0].y > area.center().y
        )),
        "an underline: {shapes:?}"
    );
}

/// The font of `needle` in the text `whole` painted last frame.
fn font_of(app: &support::App, whole: &str, needle: &str) -> egui::FontId {
    for clipped in &app.harness.output().shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            let job = &text.galley.job;
            if job.text != whole {
                continue;
            }
            if let Some(at) = job.text.find(needle) {
                let section = job
                    .sections
                    .iter()
                    .find(|s| s.byte_range.contains(&egui::text::ByteIndex(at)))
                    .unwrap();
                return section.format.font_id.clone();
            }
        }
    }
    panic!("{needle:?} was not painted");
}

#[test]
fn titles_use_the_title_style() {
    let mut app = many_requests();
    click(&mut app.harness, "GET Request 03");
    settle(&mut app.harness);
    app.harness.run_steps(1);
    let title = imposter_ui::theme::semibold(15.0);
    assert_eq!(
        font_of(&app, "API / Request 03", "Request 03"),
        title,
        "the request's name"
    );
    assert_eq!(
        font_of(&app, "API / Request 03", "API").family,
        egui::FontFamily::Proportional,
        "its place stays regular"
    );
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    app.harness.run_steps(1);
    assert_eq!(
        font_of(&app, "Appearance", "Appearance"),
        title,
        "Settings group headings"
    );
    assert_eq!(
        font_of(&app, "Host client certificates", "Host client certificates"),
        title,
        "and the heading of the host certificates"
    );
}

#[test]
fn a_selected_row_is_filled_and_marked_but_not_outlined() {
    let mut app = many_requests();
    click(&mut app.harness, "GET Request 03");
    settle(&mut app.harness);
    // Away from the row, so it is not hovered.
    app.harness.hover_at(egui::pos2(900.0, 600.0));
    app.harness.run_steps(2);
    let row = app.harness.get_by_label("GET Request 03").rect();
    let frames: Vec<egui::epaint::RectShape> = shapes_in(&app, row)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.rect.height() > row.height() - 2.0 => Some(rect),
            _ => None,
        })
        .collect();
    assert!(
        frames
            .iter()
            .any(|frame| frame.fill != egui::Color32::TRANSPARENT),
        "filled: {frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame.stroke.width == 0.0
                || frame.stroke.color == egui::Color32::TRANSPARENT),
        "no outline: {frames:?}"
    );
}

/// Where the text `exactly` was painted last frame, every time.
fn painted(app: &support::App, exactly: &str) -> Vec<egui::Rect> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == exactly => {
                Some(text.visual_bounding_rect())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn tab_labels_keep_their_distance() {
    let mut app = many_requests();
    click(&mut app.harness, "GET Request 03");
    settle(&mut app.harness);
    app.harness.run_steps(1);
    let gap = |app: &support::App, left: &str, right: &str| {
        let a = painted(app, left);
        let b = painted(app, right);
        a.iter()
            .flat_map(|a| b.iter().map(move |b| b.left() - a.right()))
            .filter(|gap| *gap > -1.0)
            .fold(f32::INFINITY, f32::min)
    };
    let request = gap(&app, "Body", "Auth");
    assert!(request >= 10.0, "request tabs {request} apart");
    let screens = gap(&app, "Requests", "Variables");
    assert!(screens >= 16.0, "screen tabs {screens} apart");
}

#[test]
fn hovering_an_icon_button_draws_no_orange() {
    let mut app = many_requests();
    // Not the current layout, so not selected.
    let button = app.harness.get_by_label("Stack request and response");
    let area = button.rect();
    button.hover();
    app.harness.run_steps(2);
    let accent = imposter_ui::theme::palette().accent;
    let strokes: Vec<egui::Color32> = shapes_in(&app, area)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::LineSegment { stroke, .. } => Some(stroke.color),
            egui::Shape::Rect(rect) => Some(rect.stroke.color),
            egui::Shape::Path(path) => match path.stroke.color {
                egui::epaint::ColorMode::Solid(color) => Some(color),
                egui::epaint::ColorMode::UV(_) => None,
            },
            _ => None,
        })
        .collect();
    assert!(!strokes.is_empty(), "the icon is drawn");
    assert!(!strokes.contains(&accent), "{strokes:?}");
}

/// Grips painted last frame: bars of the grip's length in the control-edge color.
fn grips(app: &support::App) -> usize {
    let edge = imposter_ui::theme::palette().field_border;
    app.harness
        .output()
        .shapes
        .iter()
        .filter(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                let long = |side: f32| (side - imposter_ui::widgets::GRIP_LENGTH).abs() < 0.5;
                rect.fill == edge && (long(rect.rect.width()) != long(rect.rect.height()))
            }
            _ => false,
        })
        .count()
}

#[test]
fn splits_show_a_grip_that_can_be_found() {
    let mut app = many_requests();
    // The split shows once a request is open; before, one pane says what to do.
    click(&mut app.harness, "GET Request 00");
    settle(&mut app.harness);
    app.harness.hover_at(egui::pos2(5.0, 5.0));
    app.harness.run_steps(2);
    assert_eq!(grips(&app), 1, "request and response");
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness.hover_at(egui::pos2(5.0, 5.0));
    app.harness.run_steps(2);
    assert_eq!(grips(&app), 1, "History's list and details");
}
