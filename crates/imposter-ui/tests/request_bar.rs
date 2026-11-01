//! The request bar: method, URL and Send as one control, with one edge and one focus ring.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::Rect;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use imposter_ui::theme::palette;
use support::{click, collection, request, settle, start_with};

fn opened() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Users", "http://example.com/users");
    });
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    app
}

fn parts(app: &support::App) -> [Rect; 3] {
    [
        app.harness.get_by_label("HTTP method").rect(),
        support::text_input(&app.harness, "Request URL").rect(),
        app.harness
            .get_by_role_and_label(Role::Button, "Send")
            .rect(),
    ]
}

/// Outlined rectangles painted with `color`.
fn outlined(app: &support::App, color: egui::Color32) -> Vec<(Rect, f32)> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.stroke.color == color && rect.stroke.width > 0.0 => {
                Some((rect.rect, rect.stroke.width))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn method_url_and_send_are_one_control_on_one_line() {
    let app = opened();
    let parts = parts(&app);
    let bar = outlined(&app, palette().field_border)
        .into_iter()
        .map(|(rect, _)| rect)
        .filter(|rect| {
            parts
                .iter()
                .all(|part| rect.expand(0.5).contains_rect(*part))
        })
        .collect::<Vec<_>>();
    assert_eq!(bar.len(), 1, "one edge around all three: {bar:?}");
    let bar = bar[0];
    for part in parts {
        assert!(
            (part.center().y - bar.center().y).abs() < 1.0,
            "{part:?} centred in {bar:?}"
        );
    }
    // Send fills the right end; no frame of its own around the bar.
    assert!(
        (parts[2].right() - bar.right()).abs() <= 2.5,
        "{:?} ends {bar:?}",
        parts[2]
    );
    let around = outlined(&app, palette().border)
        .into_iter()
        .filter(|(rect, _)| rect.contains_rect(bar) && rect.height() < 80.0)
        .count();
    assert_eq!(around, 0, "no card around the bar");

    // The URL's text sits in the middle of the bar, level with the method.
    let text = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == "http://example.com/users" => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap();
    assert!(
        (text.center().y - bar.center().y).abs() < 1.5,
        "{text:?} centred in {bar:?}"
    );
}

#[test]
fn the_focus_ring_goes_around_the_whole_bar() {
    let mut app = opened();
    support::text_input(&app.harness, "Request URL").click();
    app.harness.run_steps(2);
    let parts = parts(&app);
    let rings = outlined(&app, palette().accent);
    assert!(
        rings.iter().any(|(rect, width)| *width >= 1.5
            && parts
                .iter()
                .all(|part| rect.expand(0.5).contains_rect(*part))),
        "{rings:?}"
    );
}
