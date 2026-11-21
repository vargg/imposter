//! The dialog kit: one button row with the main action last, Enter to confirm, Escape to
//! cancel, captions without colons, and dialogs that fit the window.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui::{Color32, Rect, Shape};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::store::collections;
use imposter_ui::dialogs::Dialog;
use imposter_ui::icons::Icon;
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme;
use support::{click, collection, request, settle, start_with};

fn two_requests() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Keep", "http://x");
        request(store, api.id, "Remove", "http://x");
    });
    settle(&mut app.harness);
    app
}

fn names(app: &support::App) -> Vec<String> {
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            let api = collections::collections(db)?[0].id;
            collections::tree(db, api)
        })
        .unwrap()
        .unwrap()
        .requests
        .into_iter()
        .map(|r| r.name)
        .collect()
}

fn ask_to_delete(app: &mut support::App) {
    app.harness.get_by_label("GET Remove").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Delete…");
    app.harness.run_steps(2);
}

fn button_rect(app: &support::App, label: &str) -> Rect {
    app.harness
        .get_by_role(Role::Dialog)
        .get_by_role_and_label(Role::Button, label)
        .rect()
}

fn assert_frame(app: &support::App, title: &str, labels: &[&str]) -> Rect {
    let palette = theme::palette();
    let window = app.harness.ctx.content_rect();
    let frame = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Rect(shape) if shape.fill == palette.surface && shape.corner_radius.nw == 8 => {
                Some(shape)
            }
            _ => None,
        })
        .expect("rounded modal frame");
    assert_eq!(frame.stroke, egui::Stroke::new(1.0, palette.field_border));
    let modal = frame.rect;
    assert!(
        window.contains_rect(modal),
        "{modal:?} fits inside {window:?}"
    );
    assert!(
        app.harness
            .output()
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,
                Shape::Rect(shape) if shape.rect == window && shape.fill == palette.modal_scrim
                    && shape.fill.a() > 100 && shape.fill.r() == 0
            )),
        "the modal paints the palette's stronger scrim over the window"
    );

    let inside = modal.shrink(15.99);
    let dialog = app.harness.get_by_role(Role::Dialog);
    let heading = dialog.get_by_role_and_label(Role::Label, title).rect();
    let close = button_rect(app, "Close");
    assert!(
        inside.contains_rect(heading),
        "{title} has 16 pt padding: {heading:?} in {modal:?}"
    );
    assert!(
        inside.contains_rect(close),
        "Close has 16 pt padding: {close:?} in {modal:?}"
    );
    assert!(heading.right() < close.left() && heading.y_range().intersects(close.y_range()));
    assert!((modal.right() - close.right() - 17.0).abs() <= 1.0);
    support::assert_icon_button(&app.harness, "Close");

    let buttons: Vec<_> = dialog
        .query_all_by_role(Role::Button)
        .filter(|node| {
            node.accesskit_node()
                .label()
                .is_some_and(|label| labels.contains(&label.as_str()))
        })
        .collect();
    assert_eq!(
        buttons
            .iter()
            .map(|node| node.accesskit_node().label().unwrap())
            .collect::<Vec<_>>(),
        labels,
        "accessible order matches reading order"
    );
    for pair in buttons.windows(2) {
        assert!(
            pair[0].rect().right() < pair[1].rect().left(),
            "buttons read left to right"
        );
    }
    for button in &buttons {
        assert!(
            inside.contains_rect(button.rect()),
            "footer button has 16 pt padding"
        );
    }
    if let Some(main) = buttons.last() {
        let rect = main.rect();
        assert!(
            (modal.right() - rect.right() - 17.0).abs() <= 1.0,
            "main action is flush right: {rect:?} in {modal:?}"
        );
        assert!(
            (modal.bottom() - rect.bottom() - 17.0).abs() <= 1.0,
            "footer has 16 pt bottom padding"
        );
        assert!(
            app.harness
                .output()
                .shapes
                .iter()
                .any(|s| matches!(&s.shape,
                    Shape::LineSegment { points, stroke } if *stroke == egui::Stroke::new(1.0, palette.border)
                        && points[0].y > heading.bottom() && points[0].y < rect.top()
                        && (points[0].x - heading.left()).abs() <= 1.0
                        && (points[1].x - rect.right()).abs() <= 1.0
                )),
            "a full-width divider separates the footer"
        );
    }
    modal
}

fn fill(app: &support::App, label: &str) -> Color32 {
    let rect = button_rect(app, label);
    app.harness
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Rect(shape) if shape.rect == rect => Some(shape.fill),
            _ => None,
        })
        .expect("painted button")
}

#[test]
fn the_destructive_action_is_last_enter_confirms_and_escape_cancels() {
    let mut app = two_requests();
    ask_to_delete(&mut app);
    let modal = assert_frame(&app, "Delete request", &["Cancel", "Delete request"]);
    assert_eq!(fill(&app, "Cancel"), theme::palette().input);
    assert_eq!(fill(&app, "Delete request"), theme::palette().error);
    app.harness
        .get_by_role_and_label(Role::Button, "Cancel")
        .focus();
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::Tab);
    app.harness.run_steps(2);
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Delete request")
            .is_focused()
    );

    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(names(&app).len(), 2, "Escape deletes nothing");

    ask_to_delete(&mut app);
    let outside = modal.left_top() - egui::Vec2::splat(4.0);
    app.harness.hover_at(outside);
    app.harness.drag_at(outside);
    app.harness.run_steps(1);
    app.harness.drop_at(outside);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(names(&app).len(), 2, "a backdrop click deletes nothing");

    ask_to_delete(&mut app);
    app.harness
        .get_by_role_and_label(Role::Button, "Close")
        .hover();
    app.harness.run_steps(4);
    app.harness.get_by_label("Close (Esc)");
    support::click_button(&mut app.harness, "Close");
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(names(&app).len(), 2, "Close deletes nothing");

    ask_to_delete(&mut app);
    app.harness.key_press(egui::Key::Enter);
    settle(&mut app.harness);
    assert_eq!(names(&app), ["Keep"], "Enter confirms");
}

#[test]
fn name_dialogs_use_a_verb_and_captions_without_colons() {
    let mut app = two_requests();
    click(&mut app.harness, "New");
    click(&mut app.harness, "New Folder");
    app.harness.run_steps(2);
    assert_frame(&app, "New Folder", &["Cancel", "Create"]);
    support::text_input(&app.harness, "Name");
    assert!(app.harness.query_by_label("Name:").is_none());
    let create = app.harness.get_by_role_and_label(Role::Button, "Create");
    assert!(
        create.accesskit_node().is_disabled(),
        "no name, nothing to create"
    );
    let fade = app
        .harness
        .ctx
        .style_of(app.harness.ctx.theme())
        .visuals
        .disabled_alpha();
    assert_eq!(
        fill(&app, "Create"),
        theme::palette().input.gamma_multiply(fade)
    );
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert!(
        app.harness.state().state.dialog.is_some(),
        "Enter leaves an empty form open"
    );
    support::click_button(&mut app.harness, "Close");
    assert!(app.harness.state().state.dialog.is_none());
    click(&mut app.harness, "New");
    click(&mut app.harness, "New Folder");
    support::text_input(&app.harness, "Name").type_text("Users");
    app.harness.run_steps(2);
    assert_eq!(fill(&app, "Create"), theme::palette().accent);
    app.harness.key_press(egui::Key::Enter);
    settle(&mut app.harness);
    app.harness.get_by_label("Folder Users");
}

#[test]
fn a_tall_dialog_keeps_its_button_inside_the_smallest_window() {
    let mut app = two_requests();
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    app.harness.run_steps(2);
    app.harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Slash,
    );
    app.harness.run_steps(4);
    let modal = assert_frame(&app, "Keyboard shortcuts", &[]);
    assert!(app.harness.query_by_label("Esc to close").is_none());
    assert_eq!(
        app.harness
            .get_by_role(Role::Dialog)
            .query_all_by_role(Role::Button)
            .count(),
        1
    );
    let content_right = modal.right() - 17.0;
    // The scroll bar (tall and thin) runs along that edge, not inside the list.
    let bars: Vec<Rect> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if rect.rect.width() < 12.0 && rect.rect.height() > 40.0 => {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect();
    assert!(
        bars.iter()
            .any(|bar| (bar.right() - content_right).abs() <= 6.0),
        "a scroll bar at {content_right}: {bars:?}"
    );
    for shortcut in imposter_ui::shortcuts::ALL {
        if let Some(node) = app.harness.query_by_label(shortcut.description) {
            let rect = node.rect();
            assert!(
                rect.right() <= content_right - 12.0,
                "{:?} ends at {} under the scroll bar (content ends at {content_right})",
                shortcut.description,
                rect.right()
            );
        }
    }
    support::click_button(&mut app.harness, "Close");
    assert!(app.harness.state().state.dialog.is_none());
}

#[test]
fn a_guard_keeps_discard_neutral_and_close_preserves_the_edit() {
    use imposter_app::core::settings::AppSettings;
    use imposter_app::store::kv;
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &AppSettings {
                        autosave: false,
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Keep", "http://keep");
        request(store, api.id, "Other", "http://other");
    });
    click(&mut app.harness, "GET Keep");
    settle(&mut app.harness);
    support::type_into(&mut app.harness, "Request URL", "/edited");
    click(&mut app.harness, "GET Other");
    app.harness.run_steps(4);

    assert_frame(
        &app,
        "Unsaved request changes",
        &["Cancel", "Discard", "Save"],
    );
    assert_eq!(fill(&app, "Cancel"), theme::palette().input);
    assert_eq!(fill(&app, "Discard"), theme::palette().input);
    assert_eq!(fill(&app, "Save"), theme::palette().accent);
    assert!(app.harness.output().shapes.iter().any(|s| matches!(&s.shape,
        Shape::Text(text) if text.galley.job.text == "Discard" && text.fallback_color == theme::palette().error
    )));
    support::click_button(&mut app.harness, "Close");
    assert!(app.harness.state().state.dialog.is_none());
    app.harness.get_by_label("API / Keep");
    assert_eq!(support::save_state(&app.harness, "Save request"), "Unsaved");
}

#[test]
fn import_notes_default_to_open_only_for_short_lists_and_can_be_toggled() {
    let mut app = two_requests();
    for count in [3, 4, 3] {
        app.harness.state_mut().state.dialog = Some(support::import_review(count));
        app.harness.run_steps(4);
        assert_frame(&app, "Review collection import", &["Cancel", "Import"]);
        let summary = app
            .harness
            .get_by_label("Creates Pet Store: 0 requests in 0 folders");
        let environments = app.harness.get_by_label("New environments: None");
        assert!(summary.rect().bottom() < environments.rect().top());
        let label = format!("{count} notes");
        let starts_open = count <= 3;
        if count == 4 {
            click(
                &mut app.harness,
                "Remove the listed unsupported execution features",
            );
            assert!(
                !app.harness
                    .get_by_role_and_label(Role::Button, "Import")
                    .accesskit_node()
                    .is_disabled()
            );
        }
        for open in [starts_open, !starts_open] {
            let toggle = app
                .harness
                .get_by_role_and_label(Role::DisclosureTriangle, &label);
            assert_eq!(toggle.accesskit_node().data().is_expanded(), Some(open));
            for n in 1..=count {
                assert_eq!(
                    app.harness
                        .query_by_label(&format!("Import warning {n}"))
                        .is_some(),
                    open
                );
            }
            let icons = app.harness.output().shapes.iter().filter(|s| matches!(&s.shape,
                Shape::Text(text) if text.galley.job.text == Icon::Warning.glyph().to_string()
                    && text.fallback_color == theme::palette().warning
            )).count();
            assert_eq!(icons, if open { count } else { 0 });
            toggle.focus();
            app.harness.run_steps(2);
            app.harness.key_press(egui::Key::Enter);
            app.harness.run_steps(4);
            assert!(matches!(
                app.harness.state().state.dialog,
                Some(Dialog::Import { .. })
            ));
        }
        support::click_button(&mut app.harness, "Close");
        assert!(app.harness.state().state.dialog.is_none());
    }
}

#[test]
fn a_multiline_dialog_keeps_enter_for_editing() {
    let mut app = two_requests();
    app.harness.state_mut().state.dialog = Some(Dialog::Curl {
        text: "curl https://example.com".into(),
    });
    app.harness.run_steps(4);
    assert_frame(&app, "Import from curl", &["Cancel", "Open as draft"]);
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert!(
        matches!(&app.harness.state().state.dialog, Some(Dialog::Curl { text }) if text.contains('\n'))
    );
    support::click_button(&mut app.harness, "Close");
    assert!(app.harness.state().state.dialog.is_none());
}
