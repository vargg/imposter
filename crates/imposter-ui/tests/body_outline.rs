//! The request Body tab's read-only JSON outline.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{BodyType, RequestTemplate};
use imposter_app::store::collections::create_request;
use support::{click, collection, settle, start_with, text_input};

const BODY: &str = r#"{  "nested":{"name":"{{name}}"}, "total":2 }"#;
const INNER: &str = r#""name": "{{name}}""#;

fn request_with_body(body: &'static str) -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        for name in ["First", "Second"] {
            store
                .call_blocking(move |db| {
                    let template = RequestTemplate {
                        name: name.to_owned(),
                        url: "http://example.test".into(),
                        body_type: BodyType::Json,
                        body: body.as_bytes().to_vec(),
                        ..Default::default()
                    };
                    create_request(db, api.id, None, &template)
                })
                .unwrap();
        }
    });
    click(&mut app.harness, "GET First");
    settle(&mut app.harness);
    app
}

fn selected(app: &support::App) -> Option<Toggled> {
    app.harness
        .get_by_role_and_label(Role::Button, "Outline")
        .accesskit_node()
        .toggled()
}

fn assert_editor(app: &support::App, value: &str) {
    let field = text_input(&app.harness, "Request body");
    assert_eq!(field.accesskit_node().value().as_deref(), Some(value));
    assert!(field.is_focused());
    assert_eq!(selected(app), Some(Toggled::False));
}

fn outline_tool(app: &mut support::App, label: &str) {
    click(&mut app.harness, "More outline tools");
    click(&mut app.harness, label);
}

#[test]
fn outline_folds_formatted_json_without_changing_the_editor_text() {
    let mut app = request_with_body(BODY);
    assert_eq!(selected(&app), Some(Toggled::False));
    click(&mut app.harness, "Outline");
    assert_eq!(selected(&app), Some(Toggled::True));
    assert!(app.harness.query_by_label("Request body").is_none());
    app.harness.get_by_label_contains(INNER);

    click(&mut app.harness, "Fold $.nested");
    assert!(app.harness.query_by_label_contains(INNER).is_none());
    app.harness.get_by_label_contains(r#""total": 2"#);
    click(&mut app.harness, "Unfold $.nested");
    app.harness.get_by_label_contains(INNER);
    outline_tool(&mut app, "Collapse all");
    assert!(app.harness.query_by_label_contains(INNER).is_none());
    outline_tool(&mut app, "Expand all");
    app.harness.get_by_label_contains(INNER);

    click(&mut app.harness, "Outline");
    assert_editor(&app, BODY);
}

#[test]
fn outline_exits_to_the_focused_editor_on_typing_or_enter() {
    let mut app = request_with_body(BODY);
    click(&mut app.harness, "Outline");
    app.harness.event(egui::Event::Text("x".into()));
    app.harness.run_steps(2);
    assert_editor(&app, BODY);

    click(&mut app.harness, "Outline");
    app.harness.get_by_label_contains(INNER).click();
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert_editor(&app, BODY);
}

#[test]
fn outline_requires_complete_json_and_is_hidden_for_other_body_types() {
    for body in [r#"{"name": }"#, r#"{"name": {{name}}}"#, ""] {
        let mut app = request_with_body(body);
        let outline = app.harness.get_by_role_and_label(Role::Button, "Outline");
        assert!(outline.accesskit_node().is_disabled(), "{body:?}");
        outline.hover();
        app.harness.run_steps(45);
        app.harness.get_by_label("Outline needs valid JSON");
    }

    let mut app = request_with_body(BODY);
    click(&mut app.harness, "Outline");
    click(&mut app.harness, "Text");
    assert!(app.harness.query_by_label("Outline").is_none());
    text_input(&app.harness, "Request body");
    click(&mut app.harness, "JSON");
    assert_eq!(selected(&app), Some(Toggled::False));
}

#[test]
fn outline_is_kept_per_request_and_editing_resets_its_folds() {
    let mut app = request_with_body(BODY);
    click(&mut app.harness, "Outline");
    click(&mut app.harness, "Fold $.nested");
    click(&mut app.harness, "GET Second");
    settle(&mut app.harness);
    assert_eq!(selected(&app), Some(Toggled::False));
    click(&mut app.harness, "GET First");
    settle(&mut app.harness);
    assert_eq!(selected(&app), Some(Toggled::True));
    app.harness.get_by_label("Unfold $.nested");

    click(&mut app.harness, "Outline");
    assert_editor(&app, BODY);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.event(egui::Event::Text(BODY.replace('2', "3")));
    app.harness.run_steps(2);
    click(&mut app.harness, "Outline");
    app.harness.get_by_label("Fold $.nested");
    app.harness.get_by_label_contains(r#""total": 3"#);
    app.harness.get_by_label_contains(INNER);
}

#[test]
fn outline_find_reveals_matches_in_folded_rows() {
    let mut app = request_with_body(BODY);
    click(&mut app.harness, "Outline");
    outline_tool(&mut app, "Collapse all");
    click(&mut app.harness, "Find in request body");
    let field = text_input(&app.harness, "Find in request body outline");
    assert!(field.is_focused());
    field.type_text("{{name}}");
    app.harness.run_steps(2);
    app.harness.get_by_label_contains(INNER);
    app.harness.get_by_label("Match 1 of 1");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert_eq!(selected(&app), Some(Toggled::True));
    app.harness.get_by_label_contains(INNER);
    click(&mut app.harness, "Outline");
    assert_editor(&app, BODY);
}
