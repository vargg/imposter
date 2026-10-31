//! The dialog kit: one button row with the main action last, Enter to confirm, Escape to
//! cancel, captions without colons, and dialogs that fit the window.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::store::collections;
use imposter_ui::layout::MIN_WINDOW;
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
    click(&mut app.harness, "Delete");
    app.harness.run_steps(2);
}

#[test]
fn the_destructive_action_is_last_enter_confirms_and_escape_cancels() {
    let mut app = two_requests();
    ask_to_delete(&mut app);
    let cancel = app
        .harness
        .get_by_role_and_label(Role::Button, "Cancel")
        .rect();
    let delete = app
        .harness
        .get_by_role_and_label(Role::Button, "Delete")
        .rect();
    assert!(delete.left() > cancel.right(), "the action comes last");

    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(names(&app).len(), 2, "Escape deletes nothing");

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
    support::text_input(&app.harness, "Name");
    assert!(app.harness.query_by_label("Name:").is_none());
    let create = app.harness.get_by_role_and_label(Role::Button, "Create");
    assert!(
        create.accesskit_node().is_disabled(),
        "no name, nothing to create"
    );
    support::text_input(&app.harness, "Name").type_text("Users");
    app.harness.run_steps(1);
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
    let close = app
        .harness
        .get_by_role_and_label(Role::Button, "Close")
        .rect();
    let window = app.harness.ctx.content_rect();
    assert!(window.contains_rect(close), "{close:?} in {window:?}");
    let title = app.harness.get_by_label("Keyboard shortcuts").rect();
    assert!(window.contains_rect(title));
    // The sheet is centred: its content ends as far from the right as the title starts
    // from the left. Text stays clear of the scroll bar inside that edge.
    let content_right = window.width() - title.left();
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
}
