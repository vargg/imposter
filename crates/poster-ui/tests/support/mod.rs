//! The real app over a real database in a temporary directory.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use std::path::Path;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use poster_app::paths::open_store;
use poster_app::store::Store;
use poster_ui::app::PosterApp;
use tempfile::TempDir;

pub struct App {
    pub dir: TempDir,
    pub harness: Harness<'static, PosterApp>,
}

pub fn harness_at(dir: &Path, size: egui::Vec2) -> Harness<'static, PosterApp> {
    let store = open_store(dir).unwrap();
    let mut harness = Harness::builder()
        .with_size(size)
        .with_max_steps(64)
        .build_eframe(move |cc| PosterApp::new(&cc.egui_ctx, store).unwrap());
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
pub fn settle(harness: &mut Harness<'_, PosterApp>) {
    for _ in 0..100 {
        assert!(
            harness.state().bridge().wait_idle(Duration::from_secs(10)),
            "background work hung"
        );
        harness.run_steps(2);
        if harness.state().bridge().running() == 0 {
            // Two more frames: a result applied in the last one may open a dialog, which
            // spends its first frame on an invisible sizing pass.
            harness.run_steps(2);
            if harness.state().bridge().running() == 0 {
                return;
            }
        }
    }
    panic!("the app never settled");
}

pub fn click(harness: &mut Harness<'_, PosterApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

/// Click the button named `label` when the same text also appears elsewhere (a title).
pub fn click_button(harness: &mut Harness<'_, PosterApp>, label: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, label)
        .click();
    harness.run_steps(2);
}

/// Focus a field by its accessible name and type into it.
pub fn type_into(harness: &mut Harness<'_, PosterApp>, label: &str, text: &str) {
    text_input(harness, label).click();
    harness.run_steps(1);
    text_input(harness, label).type_text(text);
    harness.run_steps(2);
}

/// The single- or multi-line text input named `label` (its caption may share the name).
pub fn text_input<'h>(
    harness: &'h Harness<'_, PosterApp>,
    label: &'h str,
) -> egui_kittest::Node<'h> {
    use egui::accesskit::Role;
    harness
        .query_by_role_and_label(Role::TextInput, label)
        .or_else(|| harness.query_by_role_and_label(Role::MultilineTextInput, label))
        .or_else(|| harness.query_by_role_and_label(Role::PasswordInput, label))
        .unwrap_or_else(|| panic!("no text input named {label:?}"))
}

/// Answer the open name dialog.
pub fn answer_name(harness: &mut Harness<'_, PosterApp>, text: &str) {
    harness.run_steps(2);
    harness.get_by_label("Name:").type_text(text);
    harness.run_steps(1);
    click(harness, "OK");
    settle(harness);
}

pub fn collection(store: &Store, name: &str) -> poster_app::store::collections::Collection {
    let name = name.to_owned();
    store
        .call_blocking(move |db| {
            poster_app::store::collections::create_collection(
                db,
                &name,
                "",
                &poster_app::core::model::Variables::new(),
            )
        })
        .unwrap()
}

pub fn request(
    store: &Store,
    collection: poster_app::core::ids::CollectionId,
    name: &str,
    url: &str,
) -> poster_app::core::model::Request {
    let template = poster_app::core::model::RequestTemplate {
        name: name.to_owned(),
        url: url.to_owned(),
        body_type: poster_app::core::model::BodyType::None,
        ..Default::default()
    };
    store
        .call_blocking(move |db| {
            poster_app::store::collections::create_request(db, collection, None, &template)
        })
        .unwrap()
}
