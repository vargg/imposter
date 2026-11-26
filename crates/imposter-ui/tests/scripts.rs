//! The Scripts tab and script results: editing, compile errors, examples, the reference,
//! and what a send shows and keeps.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{RequestTemplate, Scripts};
use imposter_app::store::{collections, executions};
use imposter_testkit::{Server, response};
use imposter_ui::app::ResponseTab;
use imposter_ui::layout::{MIN_WINDOW, Orientation};
use imposter_ui::scripts_tab::{AFTER_EDITOR, BEFORE_EDITOR};
use imposter_ui::theme::{self, Density};
use support::{click, click_button, collection, request, settle, start_with};

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

fn fixture() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"id": 7}"#,
        )
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn open_scripts(app: &mut support::App) {
    support::open_tab(&mut app.harness, "Scripts");
}

fn set_scripts(app: &mut support::App, before: &str, after: &str) {
    let scripts = &mut app
        .harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .scripts;
    before.clone_into(&mut scripts.before);
    after.clone_into(&mut scripts.after);
    app.harness.run_steps(2);
}

fn send(app: &mut support::App) {
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
}

#[test]
fn scripts_are_typed_saved_and_marked_on_their_tab() {
    let mut f = fixture();
    open_scripts(&mut f.app);
    let tab = f.app.harness.get_by_role_and_label(Role::Button, "Scripts");
    assert_eq!(tab.accesskit_node().description(), None, "nothing set");

    support::type_into(
        &mut f.app.harness,
        BEFORE_EDITOR,
        "request.set_header(\"X\", \"1\");",
    );
    f.app.harness.get_by_label("Compiles");
    let tab = f.app.harness.get_by_role_and_label(Role::Button, "Scripts");
    assert_eq!(tab.accesskit_node().description().as_deref(), Some("Set"));

    click(&mut f.app.harness, "Save request");
    settle(&mut f.app.harness);
    let stored = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            let id = collections::collections(db)?[0].id;
            collections::requests(db, id)
        })
        .unwrap();
    assert_eq!(
        stored[0].template.scripts.before,
        "request.set_header(\"X\", \"1\");"
    );
}

#[test]
fn compile_state_shares_each_header_and_the_editors_share_the_pane() {
    let mut f = fixture();
    f.app.harness.set_size(egui::vec2(1500.0, 850.0));
    open_scripts(&mut f.app);
    set_scripts(&mut f.app, "let one = 1;", "let two = ;");
    let harness = &f.app.harness;
    let before = harness.get_by_label(BEFORE_EDITOR).rect();
    let after = harness.get_by_label(AFTER_EDITOR).rect();
    let pane = harness.get_by_label("Request pane").rect();
    assert!((before.height() - after.height()).abs() < 1.0);
    assert!(
        after.bottom() >= pane.bottom() - 16.0,
        "the editors fill the pane"
    );
    assert!(before.bottom() < after.top());
    for (label, state, editor) in [
        (BEFORE_EDITOR, harness.get_by_label("Compiles"), before),
        (
            AFTER_EDITOR,
            harness.get_by_label_contains("Line 1, column"),
            after,
        ),
    ] {
        let example = harness
            .get_by_label(&format!("Insert an example into the {label}"))
            .rect();
        assert!((state.rect().center().y - example.center().y).abs() < 1.0);
        assert!(state.rect().bottom() <= editor.top());
    }
    assert_eq!(harness.query_all_by_label("Compiles").count(), 1);
    let help = harness
        .query_all_by_label("Help")
        .filter_map(|node| node.accesskit_node().description())
        .collect::<Vec<_>>();
    assert_eq!(
        help.iter()
            .filter(|description| description.contains("Rhai scripts."))
            .count(),
        2
    );
    assert!(harness.query_by_label_contains("Rhai scripts.").is_none());
}

#[test]
fn script_editors_fit_and_reveal_the_last_line_in_every_layout() {
    let mut failures = Vec::new();
    for inherited in [false, true] {
        let mut app = start_with(|store| {
            let api = collection(store, "API");
            store
                .call_blocking(move |db| {
                    let folder = collections::create_folder(db, api.id, None, "Users", "", 0)?;
                    let scripts = Scripts {
                        before: "let outer = 1;".into(),
                        ..Default::default()
                    };
                    if inherited {
                        collections::set_collection_scripts(db, api.id, &scripts)?;
                        collections::set_folder_scripts(db, folder.id, &scripts)?;
                    }
                    collections::create_request(
                        db,
                        api.id,
                        Some(folder.id),
                        &RequestTemplate {
                            name: "Items".into(),
                            url: "http://example.test/items".into(),
                            scripts: Scripts {
                                before: "let first = 1;".into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    )
                    .map(drop)
                })
                .unwrap();
        });
        click(&mut app.harness, "GET Items");
        settle(&mut app.harness);
        open_scripts(&mut app);
        for size in [MIN_WINDOW, [1500.0, 850.0]] {
            for density in [Density::Compact, Density::Comfortable] {
                for orientation in [Orientation::SideBySide, Orientation::Stacked] {
                    failures.extend(check_script_layout(
                        &mut app,
                        size,
                        density,
                        orientation,
                        inherited,
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn check_script_layout(
    app: &mut support::App,
    size: [f32; 2],
    density: Density,
    orientation: Orientation,
    inherited: bool,
) -> Option<String> {
    app.harness.set_size(size.into());
    let state = &mut app.harness.state_mut().state;
    state.appearance.density = density;
    state.layout.orientation = orientation;
    set_scripts(app, "let first = 1;", "");
    let code = format!("{}let last_line = ;", "// line\n".repeat(64));
    app.harness.get_by_label(AFTER_EDITOR).focus();
    app.harness.step();
    app.harness.event(egui::Event::Paste(code.clone()));
    app.harness.step();
    app.harness.key_press(egui::Key::Home);
    app.harness.key_press(egui::Key::End);
    app.harness.run_steps(24);
    let harness = &mut app.harness;
    assert_eq!(
        harness.get_by_label(AFTER_EDITOR).value().as_deref(),
        Some(code.as_str()),
        "{size:?} {density:?} {orientation:?} inherited={inherited}"
    );
    harness.get_by_label_contains("Line 65, column");
    assert_eq!(
        harness
            .query_by_label("Also runs, before this request's:")
            .is_some(),
        inherited
    );

    let before = script_view(harness, BEFORE_EDITOR);
    let after = script_view(harness, AFTER_EDITOR);
    let pane = harness.get_by_label("Request pane").rect();
    let before_header = script_header(harness, BEFORE_EDITOR);
    let after_header = script_header(harness, AFTER_EDITOR);
    let line_bottom = harness.output().shapes.iter().find_map(|clipped| {
        let egui::Shape::Text(shape) = &clipped.shape else {
            return None;
        };
        shape
            .galley
            .job
            .text
            .ends_with("let last_line = ;")
            .then(|| shape.pos.y + shape.galley.rows.last().unwrap().rect().bottom())
    });

    harness.hover_at(before_header.center());
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(24);
    let after_scroll = script_header(harness, BEFORE_EDITOR);
    let padded_bottom = pane.bottom() - theme::spacing().sm;
    if after.bottom() > padded_bottom + 0.5
        || (before.height() - after.height()).abs() > 0.5
        || before_header.bottom() > before.top() + 0.5
        || before.shrink(0.5).intersects(after_header)
        || before.shrink(0.5).intersects(after)
        || after_header.bottom() > after.top() + 0.5
        || (after_scroll.top() - before_header.top()).abs() > 0.5
        || line_bottom.is_none_or(|bottom| bottom > after.bottom() + 0.5 || bottom <= after.top())
    {
        Some(format!(
            "{size:?} {density:?} {orientation:?} inherited={inherited}: padded pane bottom {padded_bottom}, before {before:?}, after {after:?}, headers {before_header:?} {after_header:?}, scrolled header {after_scroll:?}, last line bottom {line_bottom:?}"
        ))
    } else {
        None
    }
}

fn script_view(
    harness: &egui_kittest::Harness<'_, imposter_ui::app::ImposterApp>,
    label: &str,
) -> egui::Rect {
    let mut parent = harness.get_by_label(label).parent();
    while let Some(node) = parent {
        if node.accesskit_node().role() == Role::ScrollView {
            return node.rect();
        }
        parent = node.parent();
    }
    panic!("{label} has no scroll view");
}

fn script_header(
    harness: &egui_kittest::Harness<'_, imposter_ui::app::ImposterApp>,
    label: &str,
) -> egui::Rect {
    harness
        .get_by_label(&format!("Insert an example into the {label}"))
        .rect()
}

#[test]
fn a_compile_error_is_shown_with_its_line() {
    let mut f = fixture();
    f.app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    f.app.harness.run_steps(2);
    open_scripts(&mut f.app);
    set_scripts(&mut f.app, "", "let a = 1;\nlet b = ;");
    let error = f.app.harness.get_by_label_contains("Line 2, column 9:");
    let full = error.value().unwrap();
    error.hover();
    f.app.harness.run_steps(4);
    assert_eq!(f.app.harness.query_all_by_label(&full).count(), 2);
}

#[test]
fn examples_go_in_at_the_cursor_and_the_reference_lists_the_api() {
    let mut f = fixture();
    open_scripts(&mut f.app);
    click(
        &mut f.app.harness,
        &format!("Insert an example into the {AFTER_EDITOR}"),
    );
    click(&mut f.app.harness, "Test the status");
    let after = f
        .app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .scripts
        .after
        .clone();
    assert_eq!(after, "test(\"status is 200\", response.status == 200);\n");

    click(&mut f.app.harness, "Script reference");
    f.app.harness.run_steps(3);
    assert!(matches!(
        f.app.harness.state().state.dialog,
        Some(imposter_ui::dialogs::Dialog::ScriptReference)
    ));
    for entry in imposter_app::script::API {
        // A name can be listed under two headings (request and mock scripts).
        assert!(
            f.app
                .harness
                .query_all_by_label(entry.name)
                .next()
                .is_some(),
            "the reference lists {}",
            entry.name
        );
    }
}

#[test]
fn a_send_shows_script_tests_and_console_and_history_keeps_only_the_tests() {
    let mut f = fixture();
    set_scripts(
        &mut f.app,
        r#"request.set_header("X-Signed", "yes");"#,
        r#"print("seen " + response.status);
test("status is 200", response.status == 200);
test("id", || { expect(response.path("$.id"), 8) });"#,
    );
    send(&mut f.app);
    assert_eq!(
        f.server.requests()[0].header("x-signed"),
        Some("yes"),
        "the before script's header was sent"
    );
    f.app.harness.state_mut().state.response.tab = ResponseTab::Tests;
    f.app.harness.run_steps(3);
    f.app.harness.get_by_label("status is 200");
    f.app.harness.get_by_label("expected 8, got 7");
    f.app
        .harness
        .get_by_label("Console output is not kept in History");
    f.app.harness.get_by_label("seen 200");
    let tests = f
        .app
        .harness
        .get_by_role_and_label(Role::Button, "Check results");
    assert_eq!(
        tests.accesskit_node().description().as_deref(),
        Some("1 failed")
    );

    let record = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            let summary = executions::page(db, None, 0, 1)?.remove(0);
            executions::execution(db, summary.id)
        })
        .unwrap()
        .unwrap();
    assert_eq!(record.response.script.unwrap().tests.len(), 2);
    assert_eq!(record.response.console, Vec::<String>::new());
}

#[test]
fn a_failing_before_script_is_not_sent_and_says_where() {
    let mut f = fixture();
    set_scripts(&mut f.app, "let a = 1;\nthrow \"no key\";", "");
    send(&mut f.app);
    assert_eq!(f.server.requests().len(), 0);
    f.app
        .harness
        .get_by_label_contains("Before-sending script, line 2: no key");
    let count = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap();
    assert_eq!(count, 0);
}

/// API > Users > "Items", opened.
fn in_folder() -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"{}")));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        store
            .call_blocking(move |db| {
                let users = collections::create_folder(db, api.id, None, "Users", "", 0)?;
                let template = RequestTemplate {
                    name: "Items".into(),
                    url,
                    ..Default::default()
                };
                collections::create_request(db, api.id, Some(users.id), &template).map(drop)
            })
            .unwrap();
    });
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn stored_scripts(app: &support::App) -> (Scripts, Scripts) {
    app.harness
        .state()
        .store()
        .call_blocking(|db| {
            let collection = collections::collections(db)?.remove(0);
            let folder = collections::folders(db, collection.id)?.remove(0);
            Ok((collection.scripts, folder.scripts))
        })
        .unwrap()
}

#[test]
fn a_folders_scripts_are_edited_in_a_dialog_and_run_around_its_requests() {
    let mut f = in_folder();
    f.app.harness.get_by_label("Folder Users").click_secondary();
    f.app.harness.run_steps(2);
    click(&mut f.app.harness, "Scripts…");
    f.app.harness.run_steps(3);
    let before = script_view(&f.app.harness, "Folder \"Users\" before-sending script");
    let after = script_view(&f.app.harness, "Folder \"Users\" after-response script");
    let save = f
        .app
        .harness
        .get_by_role_and_label(Role::Button, "Save")
        .rect();
    assert!((before.height() - after.height()).abs() <= 0.5);
    assert!(before.bottom() < after.top());
    assert!(
        after.bottom() < save.top(),
        "the editors fit above the footer"
    );
    support::type_into(
        &mut f.app.harness,
        "Folder \"Users\" before-sending script",
        "request.set_header(\"X-Folder\", \"users\");",
    );
    click_button(&mut f.app.harness, "Save");
    settle(&mut f.app.harness);
    let (_, folder) = stored_scripts(&f.app);
    assert_eq!(
        folder.before,
        "request.set_header(\"X-Folder\", \"users\");"
    );
    let row = f.app.harness.get_by_label("Folder Users");
    assert_eq!(
        row.accesskit_node().description().as_deref(),
        Some("Runs a script before sending")
    );
    let row = row.rect();
    let glyph = imposter_ui::icons::Icon::Script.glyph().to_string();
    assert!(
        f.app.harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if row.contains(text.pos) && text.galley.job.text == glyph)
        }),
        "the folder's row marks its scripts"
    );
    let more = f
        .app
        .harness
        .get_by_role_and_label(Role::Button, "More request tabs");
    assert_eq!(
        more.accesskit_node().description().as_deref(),
        Some("More tabs: Scripts: Folder \"Users\" scripts also run"),
        "the request's own Scripts tab is empty, but says what runs around it"
    );

    open_scripts(&mut f.app);
    f.app
        .harness
        .get_by_label("Open the scripts of Folder \"Users\"");
    send(&mut f.app);
    assert_eq!(f.server.requests()[0].header("x-folder"), Some("users"));
}

#[test]
fn the_collection_menu_edits_its_scripts_and_cancel_keeps_nothing() {
    let mut f = in_folder();
    click(&mut f.app.harness, "Collection actions");
    click(&mut f.app.harness, "Collection Scripts…");
    f.app.harness.run_steps(3);
    support::type_into(
        &mut f.app.harness,
        "Collection \"API\" after-response script",
        "test(\"ok\", true);",
    );
    click_button(&mut f.app.harness, "Cancel");
    settle(&mut f.app.harness);
    assert!(stored_scripts(&f.app).0.is_empty(), "Cancel keeps nothing");

    click(&mut f.app.harness, "Collection actions");
    click(&mut f.app.harness, "Collection Scripts…");
    f.app.harness.run_steps(3);
    support::type_into(
        &mut f.app.harness,
        "Collection \"API\" after-response script",
        "test(\"from the collection\", true);",
    );
    click_button(&mut f.app.harness, "Save");
    settle(&mut f.app.harness);
    assert_eq!(
        stored_scripts(&f.app).0.after,
        "test(\"from the collection\", true);"
    );
    let picker = f.app.harness.get_by_label("Collection");
    assert_eq!(
        picker.accesskit_node().description().as_deref(),
        Some("Runs a script after each response"),
        "the collection picker says its scripts run"
    );
    let picker = picker.rect();
    let glyph = imposter_ui::icons::Icon::Script.glyph().to_string();
    assert!(
        f.app.harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if picker.contains(text.pos) && text.galley.job.text.starts_with(&glyph))
        }),
        "the picker marks them before the name"
    );
    open_scripts(&mut f.app);
    click(&mut f.app.harness, "Open the scripts of Collection \"API\"");
    f.app.harness.run_steps(3);
    assert!(matches!(
        f.app.harness.state().state.dialog,
        Some(imposter_ui::dialogs::Dialog::Scripts { .. })
    ));
}
