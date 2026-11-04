//! The real app over a real database in a temporary directory.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use std::path::Path;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::paths::open_store;
use imposter_app::store::Store;
use imposter_ui::app::ImposterApp;
use tempfile::TempDir;

pub struct App {
    pub dir: TempDir,
    pub harness: Harness<'static, ImposterApp>,
}

pub fn harness_at(dir: &Path, size: egui::Vec2) -> Harness<'static, ImposterApp> {
    let store = open_store(dir).unwrap();
    let mut harness = Harness::builder()
        .with_size(size)
        .with_max_steps(64)
        .build_eframe(move |cc| ImposterApp::new(&cc.egui_ctx, store).unwrap());
    settle(&mut harness);
    harness
}

pub fn start() -> App {
    start_with(|_| {})
}

/// Prepare the database before the app opens it.
pub fn start_with(prepare: impl FnOnce(&Store)) -> App {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open_store(dir.path()).unwrap();
        prepare(&store);
        store.close().unwrap();
    }
    let harness = harness_at(dir.path(), egui::vec2(1400.0, 900.0));
    App { dir, harness }
}

/// Run frames until background work is done and its results are applied.
pub fn settle(harness: &mut Harness<'_, ImposterApp>) {
    for _ in 0..100 {
        assert!(
            harness.state().bridge().wait_idle(Duration::from_secs(10)),
            "background work hung"
        );
        harness.run_steps(2);
        if harness.state().bridge().running() == 0 {
            // Two more frames: a result applied in the last one may open a dialog, which
            // spends its first frame on an invisible sizing pass. A result may also start the
            // next task, which can finish within those frames: settled means none started.
            let spawned = harness.state().bridge().spawned();
            harness.run_steps(2);
            if harness.state().bridge().running() == 0
                && harness.state().bridge().spawned() == spawned
            {
                return;
            }
        }
    }
    panic!("the app never settled");
}

pub fn click(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

/// Open the request tab named `label`, from the row or, when it did not fit, from "More".
pub fn open_tab(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    if harness
        .query_by_role_and_label(egui::accesskit::Role::Button, label)
        .is_none()
    {
        click_button(harness, "More request tabs");
    }
    click_button(harness, label);
}

/// Open the response tab named `label`, from the row or, when it did not fit, from "More".
pub fn open_response_tab(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    if harness
        .query_by_role_and_label(egui::accesskit::Role::Button, label)
        .is_none()
    {
        click_button(harness, "More response tabs");
    }
    click_button(harness, label);
}

/// Click the menu item `label` of the menu just opened; the empty request pane may offer
/// a button with the same text.
pub fn click_menu_item(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    harness.get_all_by_label(label).next_back().unwrap().click();
    harness.run_steps(2);
}

/// Click the button named `label` when the same text also appears elsewhere (a title).
pub fn click_button(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, label)
        .click();
    harness.run_steps(2);
}

/// Focus a field by its accessible name and type into it.
pub fn type_into(harness: &mut Harness<'_, ImposterApp>, label: &str, text: &str) {
    text_input(harness, label).click();
    harness.run_steps(1);
    text_input(harness, label).type_text(text);
    harness.run_steps(2);
}

/// The single- or multi-line text input named `label` (its caption may share the name).
pub fn text_input<'h>(
    harness: &'h Harness<'_, ImposterApp>,
    label: &'h str,
) -> egui_kittest::Node<'h> {
    use egui::accesskit::Role;
    harness
        .query_by_role_and_label(Role::TextInput, label)
        .or_else(|| harness.query_by_role_and_label(Role::MultilineTextInput, label))
        .or_else(|| harness.query_by_role_and_label(Role::PasswordInput, label))
        .unwrap_or_else(|| panic!("no text input named {label:?}"))
}

/// Answer the open name dialog with Enter, which presses its main button.
pub fn answer_name(harness: &mut Harness<'_, ImposterApp>, text: &str) {
    harness.run_steps(2);
    text_input(harness, "Name").type_text(text);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    settle(harness);
}

pub fn collection(store: &Store, name: &str) -> imposter_app::store::collections::Collection {
    let name = name.to_owned();
    store
        .call_blocking(move |db| {
            imposter_app::store::collections::create_collection(
                db,
                &name,
                "",
                &imposter_app::core::model::Variables::new(),
            )
        })
        .unwrap()
}

pub fn request(
    store: &Store,
    collection: imposter_app::core::ids::CollectionId,
    name: &str,
    url: &str,
) -> imposter_app::core::model::Request {
    let template = imposter_app::core::model::RequestTemplate {
        name: name.to_owned(),
        url: url.to_owned(),
        body_type: imposter_app::core::model::BodyType::None,
        ..Default::default()
    };
    store
        .call_blocking(move |db| {
            imposter_app::store::collections::create_request(db, collection, None, &template)
        })
        .unwrap()
}

/// The save state of the screen whose Save icon is named `save`, as screen readers hear it
/// ("Saved", "Unsaved", "Draft", "Save failed"…): the icon's description.
pub fn save_state(harness: &Harness<'_, ImposterApp>, save: &str) -> String {
    harness
        .get_all_by_label(save)
        .find_map(|node| node.accesskit_node().description())
        .unwrap_or_else(|| panic!("{save} has no save state"))
}

/// The button named `label` is an icon: square, a control high, its name for screen
/// readers and its words in the hover text.
pub fn assert_icon_button(harness: &Harness<'_, ImposterApp>, label: &str) {
    let rect = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, label)
        .rect();
    assert!(
        (rect.width() - rect.height()).abs() < 0.5 && rect.width() <= 32.0,
        "{label} is not an icon button: {rect:?}"
    );
}
