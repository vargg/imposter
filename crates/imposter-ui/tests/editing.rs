//! Tree operations, editor details, drafts, cancellation, the viewer and the layout.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{BodyType, Request, RequestTemplate};
use imposter_app::store::{collections, executions, kv};
use imposter_ui::app::{CloseState, EditorTab};
use imposter_ui::layout::{MIN_WINDOW, Orientation};
use support::{
    answer_name, click, click_button, collection, request, save_state, settle, start_with,
    type_into,
};

fn open(app: &mut support::App, label: &str) {
    click(&mut app.harness, label);
    settle(&mut app.harness);
}

#[test]
fn tree_context_menus_rename_move_and_create_inside_folders() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Old name", "http://x");
        let id = api.id;
        store
            .call_blocking(move |db| collections::create_folder(db, id, None, "Users", "", 5))
            .unwrap();
    });
    open(&mut app, "GET Old name");

    app.harness.get_by_label("GET Old name").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Rename");
    support::text_input(&app.harness, "Name").type_text(" (renamed)");
    app.harness.run_steps(1);
    click_button(&mut app.harness, "Rename");
    settle(&mut app.harness);
    app.harness.get_by_label("API / Old name (renamed)");

    app.harness
        .get_by_label("GET Old name (renamed)")
        .click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Move…");
    click_button(&mut app.harness, "Move");
    settle(&mut app.harness);
    app.harness.get_by_label("API / Users / Old name (renamed)");

    app.harness.get_by_label("Folder Users").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "New Folder here");
    answer_name(&mut app.harness, "Admins");
    app.harness.get_by_label("Folder Admins");

    let stored = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        collections::tree(db, api)
    });
    let tree = stored.unwrap().unwrap();
    assert_eq!(tree.requests[0].name, "Old name (renamed)");
    assert!(tree.requests[0].folder_id.is_some());
    assert!(
        tree.folders
            .iter()
            .any(|f| f.name == "Admins" && f.parent_id.is_some())
    );
}

#[test]
fn dragging_a_request_onto_a_folder_moves_it() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Mover", "http://x");
        let id = api.id;
        store
            .call_blocking(move |db| collections::create_folder(db, id, None, "Target", "", 5))
            .unwrap();
    });
    let from = app.harness.get_by_label("GET Mover").rect().center();
    let to = app.harness.get_by_label("Folder Target").rect().center();

    app.harness.drag_at(from);
    app.harness.run_steps(1);
    for step in 1..=6 {
        #[allow(clippy::cast_precision_loss)] // six small steps
        app.harness
            .hover_at(from + (to - from) * (step as f32 / 6.0));
        app.harness.run_steps(1);
    }
    app.harness.drop_at(to);
    app.harness.run_steps(2);
    settle(&mut app.harness);

    let tree = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        collections::tree(db, api)
    });
    let tree = tree.unwrap().unwrap();
    let target = tree.folders.iter().find(|f| f.name == "Target").unwrap().id;
    assert_eq!(tree.requests[0].folder_id, Some(target));
}

/// Drag the row named `from` to `fraction` of the height of the row named `to` (0 is its top).
fn drag_row(app: &mut support::App, from: &str, to: &str, fraction: f32) {
    let from = app.harness.get_by_label(from).rect().center();
    let rect = app.harness.get_by_label(to).rect();
    let to = egui::pos2(rect.center().x, rect.top() + rect.height() * fraction);
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    for step in 1..=6 {
        #[allow(clippy::cast_precision_loss)] // six small steps
        app.harness
            .hover_at(from + (to - from) * (step as f32 / 6.0));
        app.harness.run_steps(1);
    }
    app.harness.drop_at(to);
    app.harness.run_steps(2);
    settle(&mut app.harness);
}

/// The root requests' names in the order the store gives the tree.
fn root_order(app: &support::App) -> Vec<String> {
    let tree = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        collections::tree(db, api)
    });
    let mut requests = tree.unwrap().unwrap().requests;
    requests.retain(|r| r.folder_id.is_none());
    requests.sort_by_key(|r| (r.position, r.id.0));
    requests.into_iter().map(|r| r.name).collect()
}

fn three_requests() -> support::App {
    start_with(|store| {
        let api = collection(store, "API");
        for name in ["Alpha", "Bravo", "Charlie"] {
            request(store, api.id, name, "http://x");
        }
    })
}

#[test]
fn dragging_a_request_onto_a_rows_edges_reorders_it() {
    let mut app = three_requests();
    drag_row(&mut app, "GET Charlie", "GET Alpha", 0.2);
    assert_eq!(root_order(&app), ["Charlie", "Alpha", "Bravo"], "before");
    drag_row(&mut app, "GET Charlie", "GET Bravo", 0.8);
    assert_eq!(root_order(&app), ["Alpha", "Bravo", "Charlie"], "after");
}

#[test]
fn dragging_near_the_trees_bottom_edge_scrolls_it() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        for n in 0..40 {
            request(store, api.id, &format!("r{n:02}"), "http://x");
        }
    });
    let first = || "GET r00";
    let before = app.harness.get_by_label(first()).rect().top();
    // The tree ends a little above the status bar's controls.
    let bottom = app.harness.get_by_label("Hide Collections").rect().top() - 12.0;
    let from = app.harness.get_by_label(first()).rect().center();
    let to = egui::pos2(from.x, bottom);
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    for step in 1..=20 {
        #[allow(clippy::cast_precision_loss)] // a few small steps, then holding still
        app.harness
            .hover_at(from + (to - from) * (f32::min(step as f32, 6.0) / 6.0));
        app.harness.run_steps(1);
    }
    let during = app.harness.get_by_label(first()).rect().top();
    app.harness.drop_at(to);
    app.harness.run_steps(2);
    assert!(
        during < before - 20.0,
        "the tree scrolled down while held at its edge: {before} -> {during}"
    );
}

#[test]
fn reordering_is_off_while_the_tree_is_filtered() {
    let mut app = three_requests();
    type_into(&mut app.harness, "Filter requests", "url:x");
    settle(&mut app.harness);
    drag_row(&mut app, "GET Charlie", "GET Alpha", 0.2);
    assert_eq!(root_order(&app), ["Alpha", "Bravo", "Charlie"]);
}

#[test]
fn alt_arrows_and_the_menu_move_the_selected_request() {
    let mut app = three_requests();
    open(&mut app, "GET Alpha");
    // The tree row, not a text field, has the keyboard.
    app.harness.get_by_label("GET Alpha").focus();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowDown);
    app.harness.run_steps(2);
    settle(&mut app.harness);
    assert_eq!(root_order(&app), ["Bravo", "Alpha", "Charlie"]);

    // In a text field Alt+arrows move the cursor, never the request.
    support::text_input(&app.harness, "Request URL").click();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowDown);
    app.harness.run_steps(2);
    settle(&mut app.harness);
    assert_eq!(root_order(&app), ["Bravo", "Alpha", "Charlie"]);

    app.harness.get_by_label("GET Charlie").click_secondary();
    app.harness.run_steps(2);
    let down = app
        .harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Move down");
    assert!(
        down.accesskit_node().is_disabled(),
        "the last request cannot go down"
    );
    click(&mut app.harness, "Move up");
    settle(&mut app.harness);
    assert_eq!(root_order(&app), ["Bravo", "Charlie", "Alpha"]);
}

#[test]
fn a_reordered_open_request_keeps_its_place_after_an_edit_is_saved() {
    let mut app = three_requests();
    open(&mut app, "GET Alpha");
    drag_row(&mut app, "GET Alpha", "GET Charlie", 0.8);
    assert_eq!(root_order(&app), ["Bravo", "Charlie", "Alpha"]);

    type_into(&mut app.harness, "Request URL", "/edited");
    click(&mut app.harness, "Save request");
    settle(&mut app.harness);
    let stored = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        collections::tree(db, api)
    });
    let alpha = stored
        .unwrap()
        .unwrap()
        .requests
        .into_iter()
        .find(|r| r.name == "Alpha")
        .unwrap();
    assert_eq!(alpha.url, "http://x/edited", "the edit was saved");
    assert_eq!(root_order(&app), ["Bravo", "Charlie", "Alpha"]);
}

fn json_body(app: &mut support::App) -> String {
    app.harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .body_text
        .clone()
}

/// Open "Create" on its Body tab as an empty JSON body with the editor focused.
fn empty_json_body() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Create", "http://x");
    });
    open(&mut app, "GET Create");
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = EditorTab::Body;
        pane.editor.body_type = BodyType::Json;
    }
    app.harness.run_steps(2);
    app.harness.get_by_label("Request body").focus();
    app.harness.run_steps(1);
    app
}

fn type_key(app: &mut support::App, text: &str) {
    for c in text.chars() {
        app.harness.event(egui::Event::Text(c.to_string()));
        app.harness.run_steps(1);
    }
}

#[test]
fn typing_json_closes_pairs_and_indents_new_lines() {
    let mut app = empty_json_body();
    type_key(&mut app, "{");
    assert_eq!(json_body(&mut app), "{}");
    app.harness.key_press(egui::Key::Backspace);
    app.harness.run_steps(1);
    assert_eq!(json_body(&mut app), "", "Backspace removes the empty pair");
    type_key(&mut app, "{");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(1);
    assert_eq!(json_body(&mut app), "{\n  \n}");
    type_key(&mut app, "\"id\": [");
    assert_eq!(
        json_body(&mut app),
        "{\n  \"id\": []\n}",
        "the quote stepped over"
    );
    type_key(&mut app, "1]");
    assert_eq!(json_body(&mut app), "{\n  \"id\": [1]\n}", "] stepped over");
}

#[test]
fn undo_steps_back_over_a_closed_pair_as_typed() {
    let mut app = empty_json_body();
    // egui keeps an undo point once the text has stood still for a second.
    let pause = |app: &mut support::App| app.harness.run_steps(90);
    let undo = |app: &mut support::App| {
        app.harness
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        app.harness.run_steps(2);
    };
    pause(&mut app);
    type_key(&mut app, "[");
    pause(&mut app);
    type_key(&mut app, "1");
    pause(&mut app);
    assert_eq!(json_body(&mut app), "[1]");
    undo(&mut app);
    assert_eq!(json_body(&mut app), "[]", "the pair, never a lone [");
    undo(&mut app);
    assert_eq!(json_body(&mut app), "");
}

#[test]
fn a_paste_into_the_body_is_never_paired() {
    let mut app = empty_json_body();
    type_key(&mut app, "[");
    app.harness.event(egui::Event::Paste("{".into()));
    app.harness.run_steps(2);
    assert_eq!(json_body(&mut app), "[{]");
}

#[test]
fn a_json_body_with_placeholders_is_valid_and_formats_on_request_or_paste() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Create", "http://x");
    });
    open(&mut app, "GET Create");
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = EditorTab::Body;
        pane.editor.body_type = BodyType::Json;
        pane.editor.body_text = r#"{"id":{{id}},"price":19.50}"#.into();
    }
    app.harness.run_steps(2);
    // A placeholder stands for a value, so this is not an error.
    app.harness.get_by_label("Valid JSON");

    click(&mut app.harness, "Format");
    assert_eq!(
        json_body(&mut app),
        "{\n  \"id\": {{id}},\n  \"price\": 19.50\n}"
    );
    assert_eq!(save_state(&app.harness, "Save request"), "Unsaved");
    assert!(
        app.harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Format")
            .accesskit_node()
            .is_disabled(),
        "already formatted"
    );

    // The shortcut, from anywhere on the Requests screen.
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_text = r#"[1,{"a":[]}]"#.into();
    app.harness.run_steps(1);
    app.harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::F,
    );
    app.harness.run_steps(2);
    assert_eq!(json_body(&mut app), "[\n  1,\n  {\n    \"a\": []\n  }\n]");

    // Pasted into an empty body, a document is laid out at once.
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_text = String::new();
    app.harness.run_steps(1);
    app.harness.get_by_label("Request body").focus();
    app.harness.run_steps(1);
    app.harness
        .event(egui::Event::Paste(r#"{"token":"{{t}}"}"#.into()));
    app.harness.run_steps(2);
    assert_eq!(json_body(&mut app), "{\n  \"token\": \"{{t}}\"\n}");
}

#[test]
fn request_settings_take_one_row_each_and_show_what_they_inherit() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Get", "http://x");
    });
    open(&mut app, "GET Get");
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Settings;
    app.harness.run_steps(2);
    // Each label shares its row with its control.
    for label in [
        "Timeout (seconds)",
        "Follow redirects",
        "Proxy URL",
        "Key password",
    ] {
        let rows: Vec<egui::Rect> = app
            .harness
            .query_all_by_label(label)
            .map(|node| node.rect())
            .collect();
        assert_eq!(rows.len(), 2, "{label}: its caption and its control");
        assert!(
            (rows[0].center().y - rows[1].center().y).abs() < 4.0,
            "{label}: {rows:?}"
        );
    }
    let options = |app: &support::App| {
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .options
            .clone()
    };

    // An on/off setting: the default says what it is; On or Off overrides it.
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Follow redirects")
        .click();
    app.harness.run_steps(3);
    click(&mut app.harness, "Off");
    assert!(!options(&app).follow_redirects.inherit);
    assert!(!options(&app).follow_redirects.value);
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Follow redirects")
        .click();
    app.harness.run_steps(3);
    click(&mut app.harness, "Default (On)");
    assert!(options(&app).follow_redirects.inherit);

    // A text setting: typing overrides, the reset icon goes back.
    type_into(&mut app.harness, "Timeout (seconds)", "5");
    assert!(!options(&app).timeout.inherit);
    assert_eq!(options(&app).timeout.value, "5");
    click(
        &mut app.harness,
        "Use application default for Timeout (seconds)",
    );
    assert!(options(&app).timeout.inherit);
}

#[test]
fn body_types_and_invalid_settings_reach_the_saved_request() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Upload", "http://x");
    });
    open(&mut app, "GET Upload");

    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Body;
    app.harness.run_steps(2);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Json;
    app.harness.run_steps(1);
    type_into(&mut app.harness, "Request body", r#"{"a": 1}"#);
    app.harness.get_by_label("Valid JSON");

    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Settings;
    app.harness.run_steps(2);
    // Typing over the default makes it the request's own; no box to untick first.
    assert!(
        app.harness
            .query_by_label("Use application default for Timeout (seconds)")
            .is_none(),
        "nothing to reset yet"
    );
    type_into(&mut app.harness, "Timeout (seconds)", "abc");
    app.harness.get_by_label("Invalid fields");
    app.harness
        .get_by_label("Enter a positive number of seconds.");

    click(
        &mut app.harness,
        "Use application default for Timeout (seconds)",
    );
    for _ in 0..4 {
        app.harness.step();
    }
    settle(&mut app.harness);
    let saved = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        let tree = collections::tree(db, api)?.unwrap();
        collections::request(db, tree.requests[0].id)
    });
    let saved = saved.unwrap().unwrap();
    assert_eq!(saved.template.body_type, BodyType::Json);
    assert_eq!(saved.template.body, br#"{"a": 1}"#);
    assert_eq!(saved.template.options.timeout, None);
}

#[test]
fn a_restored_draft_is_saved_into_the_selected_collection() {
    let mut app = start_with(|store| {
        collection(store, "API");
        let draft = Request {
            template: RequestTemplate {
                name: "Draft".into(),
                url: "http://draft".into(),
                ..RequestTemplate::default()
            },
            ..Request::default()
        };
        store
            .call_blocking(move |db| kv::set(db, kv::Table::UiState, "draft", &draft))
            .unwrap();
    });
    app.harness.get_by_label("Unsaved draft / Draft");
    assert_eq!(save_state(&app.harness, "Save to collection…"), "Draft");

    click(&mut app.harness, "Save to collection…");
    app.harness.get_by_label("Request name in API");
    click_button(&mut app.harness, "Save");
    settle(&mut app.harness);

    app.harness.get_by_label("API / Draft");
    let tree = app.harness.state().store().call_blocking(|db| {
        let api = collections::collections(db)?[0].id;
        collections::tree(db, api)
    });
    assert_eq!(tree.unwrap().unwrap().requests[0].name, "Draft");
}

#[test]
fn cancelling_a_running_request_records_a_cancelled_run() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!(
        "http://127.0.0.1:{}/slow",
        listener.local_addr().unwrap().port()
    );
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    runtime.spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        accepted_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Slow", &url);
    });
    open(&mut app, "GET Slow");

    click(&mut app.harness, "Send");
    accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    app.harness.run_steps(1);
    app.harness.get_by_label("Cancel stops it.");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);

    app.harness.get_by_label("Cancelled");
    let summary = app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::page(db, None, 0, 1))
        .unwrap();
    assert_eq!(
        summary[0].outcome,
        imposter_app::core::execution::Outcome::Cancelled
    );
}

#[test]
fn the_viewer_finds_text_in_the_response() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Viewer", "http://x");
    });
    open(&mut app, "GET Viewer");
    app.harness
        .state_mut()
        .state
        .response
        .show(imposter_app::core::execution::Response {
            status: 200,
            reason: "OK".into(),
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"alpha beta\nALPHA gamma\nalpha".to_vec(),
            ..Default::default()
        });
    app.harness.run_steps(2);

    app.harness.get_by_label("Copy body").click();
    app.harness.step();
    assert!(app.harness.output().platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "alpha beta\nALPHA gamma\nalpha")
    }));
    let wrapped = app.harness.state().state.response.body_view.wrap;
    click(&mut app.harness, "Wrap");
    assert_eq!(app.harness.state().state.response.body_view.wrap, !wrapped);
    click(&mut app.harness, "Wrap");
    assert_eq!(app.harness.state().state.response.body_view.wrap, wrapped);

    click(&mut app.harness, "Find");
    app.harness
        .get_by_label("Find in response body")
        .type_text("alpha");
    app.harness.run_steps(2);
    app.harness.get_by_label("Match 1 of 3");
    for label in ["Next", "Previous", "Close"] {
        support::assert_icon_button(&app.harness, label);
    }
    click(&mut app.harness, "Next");
    app.harness.get_by_label("Match 2 of 3");
    click(&mut app.harness, "Previous");
    click(&mut app.harness, "Previous");
    app.harness.get_by_label("Match 3 of 3");
    click(&mut app.harness, "Close");
    assert!(app.harness.query_by_label("Match 3 of 3").is_none());

    app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Headers;
    app.harness.run_steps(2);
    app.harness.get_by_label("Copy headers");
}

#[test]
fn the_layout_switch_and_the_sidebar_toggle() {
    let mut app = start_with(|store| {
        collection(store, "API");
    });
    assert_eq!(
        app.harness.state().state.layout.orientation,
        Orientation::SideBySide
    );
    // Both layouts have a button; the current one is selected.
    let selected = |app: &support::App, label: &str| {
        app.harness
            .get_by_label(label)
            .accesskit_node()
            .toggled()
            .map(|state| state == egui::accesskit::Toggled::True)
    };
    assert_eq!(
        selected(&app, "Put request and response side by side"),
        Some(true)
    );
    assert_eq!(selected(&app, "Stack request and response"), Some(false));
    click(&mut app.harness, "Stack request and response");
    assert_eq!(
        app.harness.state().state.layout.orientation,
        Orientation::Stacked
    );
    assert_eq!(selected(&app, "Stack request and response"), Some(true));
    assert_eq!(
        selected(&app, "Put request and response side by side"),
        Some(false)
    );
    click(&mut app.harness, "Put request and response side by side");
    assert_eq!(
        app.harness.state().state.layout.orientation,
        Orientation::SideBySide
    );

    // The sidebar toggle is an action, never selected, and a line sets it apart from the
    // layouts: drawn selected beside them, it read as a third layout.
    assert_eq!(selected(&app, "Hide Collections"), Some(false));
    let toggle = app.harness.get_by_label("Hide Collections").rect();
    let layouts = app
        .harness
        .get_by_label("Put request and response side by side")
        .rect();
    let divided = app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::LineSegment { points: [a, b], .. }
            if (a.x - b.x).abs() < 0.5
                && a.x > toggle.right()
                && a.x < layouts.left()
                && (a.y - b.y).abs() > toggle.height() / 2.0)
    });
    assert!(divided, "a separator between the toggle and the layouts");
    click(&mut app.harness, "Hide Collections");
    assert!(!app.harness.state().state.layout.sidebar_visible);
    assert_eq!(selected(&app, "Show Collections"), Some(false));
    click(&mut app.harness, "Show Collections");
    assert!(app.harness.state().state.layout.sidebar_visible);
}

#[test]
fn collection_controls_share_a_row_and_plus_creates_a_folder() {
    let mut app = start_with(|store| {
        collection(
            store,
            "A collection with a name much wider than the sidebar",
        );
    });
    let picker = app.harness.get_by_label("Collection").rect();
    let plus = app.harness.get_by_label("New").rect();
    let more = app.harness.get_by_label("Collection actions").rect();
    assert!(picker.right() < plus.left() && plus.right() < more.left());
    for icon in [plus, more] {
        assert!((icon.center().y - picker.center().y).abs() < 1.0);
        assert!(
            icon.right() < app.harness.state().state.layout.sidebar_width,
            "picker: {picker:?}, plus: {plus:?}, more: {more:?}, sidebar: {}",
            app.harness.state().state.layout.sidebar_width
        );
    }
    click(&mut app.harness, "New");
    click(&mut app.harness, "New Folder");
    answer_name(&mut app.harness, "Users");
    app.harness.get_by_label("Folder Users");
    let collection = app.harness.state().state.collection.unwrap();
    let tree = app
        .harness
        .state()
        .store()
        .call_blocking(move |db| collections::tree(db, collection))
        .unwrap()
        .unwrap();
    assert_eq!(tree.folders.len(), 1);
    assert_eq!(tree.folders[0].name, "Users");
}

#[test]
fn response_tools_share_the_tab_row_with_copy_at_the_right() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Viewer", "http://x");
    });
    open(&mut app, "GET Viewer");
    app.harness
        .state_mut()
        .state
        .response
        .show(imposter_app::core::execution::Response {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"response body".to_vec(),
            ..Default::default()
        });
    // The viewer's tools belong to the raw text of the headers, not their table.
    app.harness.state_mut().state.response.headers_raw = true;
    // The smallest window stacks the panes, as a user would there.
    for (size, orientation) in [
        (egui::Vec2::from(MIN_WINDOW), Orientation::Stacked),
        (egui::vec2(1500.0, 850.0), Orientation::SideBySide),
    ] {
        app.harness.state_mut().state.layout.orientation = orientation;
        app.harness.set_size(size);
        app.harness.run_steps(2);
        for (tab, copy_label, copied) in [
            ("Body", "Copy body", "response body"),
            ("Headers", "Copy headers", "content-type: text/plain"),
        ] {
            app.harness
                .get_all_by_label(tab)
                .next_back()
                .unwrap()
                .click();
            app.harness.run_steps(2);
            let tab_rect = app
                .harness
                .get_all_by_label(tab)
                .next_back()
                .unwrap()
                .rect();
            let wrap = app.harness.get_by_label("Wrap").rect();
            let search = app.harness.get_by_label("Find").rect();
            let copy = app.harness.get_by_label(copy_label).rect();
            assert!(tab_rect.right() < wrap.left());
            assert!(wrap.right() < search.left() && search.right() < copy.left());
            for tool in [wrap, search, copy] {
                assert!((tool.center().y - tab_rect.center().y).abs() < 1.0);
            }
            // The headers' Raw toggle ends the row, right after Copy; the body's Save does.
            let end = if tab == "Headers" {
                "Raw"
            } else {
                "Save body…"
            };
            let last = app.harness.get_by_label(end).rect();
            assert!(copy.right() < last.left(), "{end} after Copy");
            assert!((last.center().y - tab_rect.center().y).abs() < 1.0);
            assert!(
                (last.right() - (size.x - 30.0)).abs() < 2.0,
                "the last tool is anchored to the panel's right inset"
            );
            app.harness.get_by_label(copy_label).click();
            app.harness.step();
            assert!(
                app.harness
                    .output()
                    .platform_output
                    .commands
                    .iter()
                    .any(|command| {
                        matches!(command, egui::OutputCommand::CopyText(text) if text == copied)
                    })
            );
        }
    }
}

/// The smallest pointer target WCAG 2.5.8 (AA) allows.
const MIN_TARGET: f32 = 24.0;

#[test]
fn sidebar_icon_stays_in_the_bottom_left_and_supports_the_keyboard() {
    let mut app = start_with(|store| {
        collection(store, "API");
    });
    for size in [egui::Vec2::from(MIN_WINDOW), egui::vec2(1500.0, 850.0)] {
        app.harness.set_size(size);
        app.harness.run_steps(2);
        let visible = app.harness.get_by_label("Hide Collections").rect();
        assert!(visible.left() < 32.0 && visible.bottom() > size.y - 20.0);
        // WCAG 2.5.8: a pointer target is at least 24 by 24 points, in every density.
        assert!(visible.width() >= MIN_TARGET && visible.height() >= MIN_TARGET);
        app.harness.get_by_label("Hide Collections").focus();
        app.harness.run_steps(2);
        assert!(app.harness.get_by_label("Hide Collections").is_focused());
        app.harness.key_press(egui::Key::Space);
        app.harness.run_steps(2);
        assert!(!app.harness.state().state.layout.sidebar_visible);
        assert!(app.harness.query_by_label("Collection").is_none());
        assert_eq!(app.harness.get_by_label("Show Collections").rect(), visible);
        click(&mut app.harness, "Show Collections");
        app.harness.get_by_label("Collection");
    }
}

#[test]
fn switching_collections_clears_a_request_from_another_collection_and_deleting_one_selects_the_next()
 {
    let mut app = start_with(|store| {
        let a = collection(store, "A");
        request(store, a.id, "In A", "http://a");
        collection(store, "B");
    });
    open(&mut app, "GET In A");

    click(&mut app.harness, "Collection");
    click(&mut app.harness, "B");
    settle(&mut app.harness);
    app.harness.get_by_label("This collection is empty");

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Delete Collection…");
    app.harness
        .get_by_label_contains("Delete \"B\" with nothing inside, including unsaved edits?");
    click_button(&mut app.harness, "Delete collection");
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(collections::collections)
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["A"]);
    assert_eq!(app.harness.state().state.collections.len(), 1);
}

#[test]
fn closing_with_invalid_edits_asks_and_cancel_keeps_the_window_open() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Edited", "http://x");
    });
    open(&mut app, "GET Edited");
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Settings;
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Timeout (seconds)", "x");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness
        .get_by_label("Save changes to \"Edited\" before continuing?");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);

    assert_eq!(app.harness.state().close_state(), CloseState::Open);
}

#[test]
fn a_long_url_stays_on_one_line_in_a_narrow_pane() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(
            store,
            api.id,
            "Long",
            "http://localhost:8080/a/rather/long/path/that/does/not/fit?page=1&size=50",
        );
    });
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    click(&mut app.harness, "GET Long");
    settle(&mut app.harness);

    let field = support::text_input(&app.harness, "Request URL").rect();
    let send = app.harness.get_by_label("Send").rect();
    assert!(
        field.height() <= send.height() + 1.0,
        "the URL field grew to {} px",
        field.height()
    );
}

#[test]
fn body_is_default_and_docs_save_with_the_request() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Documented", "http://x");
    });
    open(&mut app, "GET Documented");
    assert_eq!(
        app.harness.state().state.editor.as_ref().unwrap().tab,
        EditorTab::Body
    );
    let labels: Vec<_> = EditorTab::ALL.iter().map(|(_, label)| *label).collect();
    assert_eq!(
        labels,
        [
            "Body", "Auth", "Query", "Headers", "Cookies", "Settings", "Docs", "Tests", "Captures",
            "Scripts"
        ]
    );
    // Tabs that fit are in this order; the rest wait under "More", in the same order.
    let positions: Vec<_> = labels
        .iter()
        .filter(|label| **label != "Settings")
        .filter_map(|label| app.harness.query_by_label(label).map(|n| n.rect().left()))
        .collect();
    assert!(positions.len() >= 7, "most tabs fit: {positions:?}");
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    click(&mut app.harness, "Docs");
    type_into(
        &mut app.harness,
        "Request description",
        "First line\nSecond line",
    );
    click(&mut app.harness, "Save request");
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
    let saved = app
        .harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, id))
        .unwrap()
        .unwrap();
    assert_eq!(saved.template.description, "First line\nSecond line");
    let editor = imposter_ui::editor::Editor::load(&saved);
    assert_eq!(
        editor.to_request().unwrap().template.description,
        saved.template.description
    );
}

#[test]
fn long_request_names_are_single_line_and_available_on_hover() {
    let title = "A long request name that must stay on one line even when the sidebar is narrow";
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, title, "http://x");
    });
    open(&mut app, &format!("GET {title}"));
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    for orientation in [Orientation::SideBySide, Orientation::Stacked] {
        app.harness.state_mut().state.layout.orientation = orientation;
        app.harness.run_steps(2);
        let rect = app.harness.get_by_label(&format!("GET {title}")).rect();
        assert!(rect.height() <= 32.0, "{rect:?}");
        assert!(
            rect.right() < 350.0,
            "sidebar row must not expand: {rect:?}"
        );
        app.harness.hover_at(rect.center());
        app.harness.run_steps(45);
        app.harness.get_by_label(title);
        app.harness.hover_at(egui::pos2(700.0, 450.0));
        app.harness.run_steps(2);
    }
}

#[test]
fn duplication_captures_edits_and_cancel_creates_nothing() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Original", "http://x");
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &imposter_app::core::settings::AppSettings {
                        autosave: false,
                        ..Default::default()
                    },
                )
            })
            .unwrap();
    });
    open(&mut app, "GET Original");
    click(&mut app.harness, "Docs");
    type_into(&mut app.harness, "Request description", "unsaved notes");
    let duplicate = |app: &mut support::App| {
        app.harness.get_by_label("GET Original").click_secondary();
        app.harness.run_steps(2);
        click(&mut app.harness, "Duplicate");
    };
    duplicate(&mut app);
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);
    assert!(app.harness.query_by_label("GET Original (copy)").is_none());
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .description,
        "unsaved notes"
    );
    duplicate(&mut app);
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    app.harness.get_by_label("API / Original (copy)");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .description,
        "unsaved notes"
    );
    open(&mut app, "GET Original");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .description,
        ""
    );
    duplicate(&mut app);
    settle(&mut app.harness);
    app.harness.get_by_label("API / Original (copy 2)");
}

#[test]
fn duplication_of_closed_requests_and_failures_preserve_the_editor() {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Open", "http://open");
        request(store, api.id, "Closed", "http://closed");
    });
    open(&mut app, "GET Open");
    app.harness.get_by_label("GET Closed").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Duplicate");
    settle(&mut app.harness);
    app.harness.get_by_label("API / Closed (copy)");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .url,
        "http://closed"
    );
    app.harness.state().store().call_blocking(|db| {
        db.execute_batch("CREATE TRIGGER reject_copy BEFORE INSERT ON ids BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;")?;
        Ok(())
    }).unwrap();
    app.harness.get_by_label("GET Closed").click_secondary();
    app.harness.run_steps(2);
    click(&mut app.harness, "Duplicate");
    settle(&mut app.harness);
    app.harness.get_by_label("Cannot duplicate request");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .name(),
        "Closed (copy)"
    );
}
