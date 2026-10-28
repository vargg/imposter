//! Tree operations, editor details, drafts, cancellation, the viewer and the layout.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui_kittest::kittest::Queryable;
use poster_app::core::model::{BodyType, Request, RequestTemplate};
use poster_app::store::{collections, executions, kv};
use poster_ui::app::{CloseState, EditorTab};
use poster_ui::layout::{MIN_WINDOW, Orientation};
use support::{
    answer_name, click, click_button, collection, request, settle, start_with, type_into,
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
    app.harness.get_by_label("Name:").type_text(" (renamed)");
    app.harness.run_steps(1);
    click(&mut app.harness, "OK");
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
    click(
        &mut app.harness,
        "Use application default for Timeout (seconds)",
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
    app.harness.get_by_label("Draft");

    click(&mut app.harness, "Save to collection…");
    app.harness.get_by_label("Request name in API:");
    click(&mut app.harness, "OK");
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
    app.harness.get_by_label("Request in progress");
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
        poster_app::core::execution::Outcome::Cancelled
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
        .show(poster_app::core::execution::Response {
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
    click(&mut app.harness, "Next");
    app.harness.get_by_label("Match 2 of 3");
    click(&mut app.harness, "Previous");
    click(&mut app.harness, "Previous");
    app.harness.get_by_label("Match 3 of 3");
    click(&mut app.harness, "Close");
    assert!(app.harness.query_by_label("Match 3 of 3").is_none());

    app.harness.state_mut().state.response.tab = poster_ui::app::ResponseTab::Headers;
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
    click(&mut app.harness, "Stack request and response");
    assert_eq!(
        app.harness.state().state.layout.orientation,
        Orientation::Stacked
    );
    click(&mut app.harness, "Put request and response side by side");
    assert_eq!(
        app.harness.state().state.layout.orientation,
        Orientation::SideBySide
    );

    click(&mut app.harness, "Hide Collections");
    assert!(!app.harness.state().state.layout.sidebar_visible);
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
        .show(poster_app::core::execution::Response {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"response body".to_vec(),
            ..Default::default()
        });
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
            assert!(
                (copy.right() - (size.x - 30.0)).abs() < 2.0,
                "Copy is anchored to the panel's right inset"
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
        assert!(visible.width() >= 28.0 && visible.height() >= 28.0);
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
    app.harness.get_by_label("Select a request");

    click(&mut app.harness, "Collection actions");
    click(&mut app.harness, "Delete Collection");
    click_button(&mut app.harness, "Delete");
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| collections::collections(db))
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
    click(
        &mut app.harness,
        "Use application default for Timeout (seconds)",
    );
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
            "Body", "Auth", "Query", "Headers", "Cookies", "Settings", "Docs"
        ]
    );
    let positions: Vec<_> = labels
        .iter()
        .filter(|label| **label != "Settings")
        .map(|label| app.harness.get_by_label(label).rect().left())
        .collect();
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
    let editor = poster_ui::editor::Editor::load(&saved);
    assert_eq!(
        editor.to_request().unwrap().template.description,
        saved.template.description
    );
}
