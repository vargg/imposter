//! The real app over a real database in a temporary directory.
#![cfg(test)]
#![allow(dead_code)] // each test binary uses a different subset

use std::path::Path;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::settings::AppSettings;
use imposter_app::paths::open_store;
use imposter_app::store::{Store, kv};
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

pub fn manual_save(store: &Store) {
    store
        .call_blocking(|db| {
            kv::set_app_settings(
                db,
                &AppSettings {
                    autosave: false,
                    ..AppSettings::default()
                },
            )
        })
        .unwrap();
}

/// Freeze the clock until the next advance so click helpers cannot consume the debounce.
pub fn advance_time(harness: &mut Harness<'_, ImposterApp>, seconds: f64) {
    let now = harness.ctx.input(|input| input.time);
    harness.input_mut().time = Some(now + seconds);
    harness.step();
}

pub fn disable_autosave(harness: &mut Harness<'_, ImposterApp>) {
    open_screen(harness, "Settings");
    settle(harness);
    open_settings_category(harness, "General");
    harness
        .get_by_role_and_label(egui::Role::CheckBox, "Automatically save changes")
        .click();
    harness.run_steps(2);
    click(harness, "Save settings");
    settle(harness);
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

pub fn status_bar(app: &App) -> egui::Rect {
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::Status, "Status bar")
        .rect()
}

pub fn click(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

/// Navigate through the left rail.
pub fn open_screen(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    click_button(harness, label);
}

pub fn drag_horizontally(harness: &mut Harness<'_, ImposterApp>, start: egui::Pos2, by: f32) {
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    for step in 1..=10_u8 {
        harness.hover_at(start + egui::vec2(by * f32::from(step) / 10.0, 0.0));
        harness.run_steps(1);
    }
    harness.drop_at(start + egui::vec2(by, 0.0));
    harness.run_steps(2);
}

pub fn assert_split_grip(harness: &Harness<'_, ImposterApp>, edge: egui::Pos2) {
    let splitter = harness
        .get_by_role_and_label(egui::accesskit::Role::Splitter, "Resize panel")
        .rect();
    assert!((splitter.center().x - edge.x).abs() <= 1.0);
    let palette = imposter_ui::theme::palette();
    assert!(
        harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Rect(shape)
            if shape.rect.center().distance(splitter.center()) <= 1.0
                && (shape.rect.width() - 3.0).abs() < f32::EPSILON
                && (shape.rect.height() - imposter_ui::widgets::GRIP_LENGTH).abs() < f32::EPSILON
                && (shape.fill == palette.field_border || shape.fill == palette.text_muted))
        }),
        "missing visible grip at {splitter:?}"
    );
}

pub fn open_settings_category(harness: &mut Harness<'_, ImposterApp>, category: &str) {
    click_button(harness, &format!("{category} settings"));
}

/// Open the scope row's context menu without choosing it as the edited scope.
pub fn open_environment_menu(harness: &mut Harness<'_, ImposterApp>, environment: &str) {
    harness
        .get_by_label(&format!("Environment · {environment}"))
        .click_secondary();
    harness.run_steps(2);
}

/// Open the request tab named `label`, from the row or from "More".
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

/// Choose a command from the response viewer's More tools menu.
pub fn viewer_tool(harness: &mut Harness<'_, ImposterApp>, label: &str) {
    click(harness, "More tools");
    click(harness, label);
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
    // Rows below the fold of a tall Settings category must be scrolled into view first;
    // otherwise the click misses and the following keystrokes land nowhere.
    text_input(harness, label).scroll_to_me();
    harness.run_steps(12);
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

/// Help stays off the page, but the info icon describes it to assistive technology.
pub fn assert_help(harness: &Harness<'_, ImposterApp>, text: &str) {
    assert!(
        harness.query_by_label(text).is_none(),
        "visible help: {text}"
    );
    assert!(
        harness
            .get_all_by_label("Help")
            .any(|node| { node.accesskit_node().description().as_deref() == Some(text) }),
        "no help icon describes {text:?}"
    );
}

/// The button named `label` is a square toolbar icon with its accessible name.
pub fn assert_icon_button(harness: &Harness<'_, ImposterApp>, label: &str) {
    assert_icon_button_in_range(harness, label, 0.0..=32.0);
}

pub fn import_review(notes: usize) -> imposter_ui::dialogs::Dialog {
    use imposter_app::core::model::{SavedValues, Scripts, Variables};
    use imposter_app::formats::native::{Name, NativeDocument, TransferCollection};
    use imposter_app::transfer::ImportPreview;

    let document = NativeDocument::new(TransferCollection {
        name: Name::new("Pet Store").unwrap(),
        description: String::new(),
        variables: Variables::new(),
        folders: Vec::new(),
        requests: Vec::new(),
        saved_values: SavedValues::default(),
        scripts: Scripts::default(),
    });
    imposter_ui::dialogs::Dialog::review_import(Box::new(ImportPreview {
        document,
        collection_name: "Pet Store".into(),
        environment_names: Vec::new(),
        warnings: (1..=notes).map(|n| format!("Import warning {n}")).collect(),
        requires_removal: true,
    }))
}

pub fn assert_icon_button_in_range(
    harness: &Harness<'_, ImposterApp>,
    label: &str,
    size: std::ops::RangeInclusive<f32>,
) {
    let rect = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, label)
        .rect();
    assert!(
        (rect.width() - rect.height()).abs() < 0.5 && size.contains(&rect.width()),
        "{label} is not an icon button: {rect:?}"
    );
}
