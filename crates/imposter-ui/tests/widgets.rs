//! Editor widgets through the UI: auth, rows, multipart, binary bodies and custom methods.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{AuthKind, BodyType, FormKind, RequestAuth};
use imposter_app::store::collections;
use imposter_ui::app::EditorTab;
use support::{click, collection, request, settle, start_with, type_into};

fn opened() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Widgets", "http://x");
    });
    click(&mut app.harness, "GET Widgets");
    settle(&mut app.harness);
    app
}

fn tab(app: &mut support::App, tab: EditorTab) {
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = tab;
    app.harness.run_steps(2);
}

fn saved(app: &mut support::App) -> imposter_app::core::model::Request {
    for _ in 0..4 {
        app.harness.step();
    }
    settle(&mut app.harness);
    let id = app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .id()
        .unwrap();
    app.harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, id))
        .unwrap()
        .unwrap()
}

#[test]
fn basic_auth_fields_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .get_by_label("No authentication is configured for this request.");

    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .auth
        .kind = Some(AuthKind::Basic);
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Username", "ann");
    type_into(&mut app.harness, "Password", "secret");

    assert_eq!(
        saved(&mut app).template.auth,
        RequestAuth::basic("ann", "secret")
    );
}

#[test]
fn bearer_tokens_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .auth
        .kind = Some(AuthKind::BearerToken);
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Token", "t0k");

    assert_eq!(saved(&mut app).template.auth, RequestAuth::bearer("t0k"));
}

#[test]
fn header_rows_can_be_added_edited_disabled_and_cleared() {
    let mut app = opened();
    tab(&mut app, EditorTab::Headers);

    click(&mut app.harness, "Add header");
    type_into(&mut app.harness, "Headers row 1 name", "X-Trace");
    type_into(&mut app.harness, "Headers row 1 value", "on");
    click(&mut app.harness, "Add header");
    type_into(&mut app.harness, "Headers row 2 name", "X-Off");
    click(&mut app.harness, "Headers row 2 enabled");
    let headers = saved(&mut app).template.headers;
    assert_eq!(headers.len(), 2);
    assert_eq!(
        (
            headers[0].name.as_str(),
            headers[0].value.as_str(),
            headers[0].is_enabled
        ),
        ("X-Trace", "on", true)
    );
    assert!(!headers[1].is_enabled);

    // No saving between the clicks: that takes frames, and the button disarms with time.
    let rows = |app: &support::App| {
        let editor = &app.harness.state().state.editor.as_ref().unwrap().editor;
        editor.headers.len()
    };
    click(&mut app.harness, "Remove Headers row 2");
    assert_eq!(rows(&app), 2, "one click only arms");
    click(&mut app.harness, "Click again to remove Headers row 2");
    assert_eq!(rows(&app), 1);
    assert_eq!(saved(&mut app).template.headers.len(), 1);

    click(&mut app.harness, "Clear all");
    app.harness.get_by_label("Remove all rows?");
    click(&mut app.harness, "Remove all");
    assert_eq!(saved(&mut app).template.headers, []);
}

#[test]
fn query_rows_rewrite_the_url() {
    let mut app = opened();
    tab(&mut app, EditorTab::Params);
    click(&mut app.harness, "Add query");
    type_into(&mut app.harness, "Query row 1 name", "q");
    type_into(&mut app.harness, "Query row 1 value", "a b");

    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .url,
        "http://x?q=a+b"
    );
    assert_eq!(saved(&mut app).template.url, "http://x?q=a+b");
}

#[test]
fn multipart_fields_and_binary_paths_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Body);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Multipart;
    app.harness.run_steps(2);
    click(&mut app.harness, "Add field");
    type_into(&mut app.harness, "Multipart row 1 name", "upload");
    type_into(&mut app.harness, "Multipart row 1 value", "/tmp/file.bin");
    type_into(
        &mut app.harness,
        "Multipart row 1 content type",
        "application/octet-stream",
    );
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .multipart[0]
        .kind = FormKind::File;
    app.harness
        .state_mut()
        .state
        .actions
        .push(imposter_ui::app::Action::Edited {
            url: false,
            params: false,
        });
    let fields = saved(&mut app).template.multipart_fields().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].name.as_str(), fields[0].kind),
        ("upload", FormKind::File)
    );
    assert_eq!(fields[0].content_type, "application/octet-stream");

    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Binary;
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Binary file path", "/data/blob.bin");
    let request = saved(&mut app);
    assert_eq!(
        (request.template.body_type, request.template.body.as_slice()),
        (BodyType::Binary, b"/data/blob.bin".as_slice())
    );
}

#[test]
fn a_custom_method_keeps_its_case() {
    let mut app = opened();
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "Custom method…");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .custom_method
            .as_deref(),
        Some("GET"),
        "choosing a custom method opens its editor"
    );
    type_into(&mut app.harness, "Custom HTTP method", "-search");
    app.harness.get_by_label("Use");
    type_into(&mut app.harness, "Custom HTTP method", " bad");
    app.harness.get_by_label("ASCII token, no spaces");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .custom_method = Some("PropFind".into());
    app.harness.run_steps(2);
    click(&mut app.harness, "Use");

    assert_eq!(saved(&mut app).template.method, "PropFind");
}

#[test]
fn standard_methods_are_picked_from_the_list() {
    let mut app = opened();
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "DELETE");

    assert_eq!(saved(&mut app).template.method, "DELETE");
    app.harness.get_by_label("DELETE Widgets");
}
