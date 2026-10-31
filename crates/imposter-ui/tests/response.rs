//! The response viewer: folding formatted JSON, copying paths and values, and JSONPath
//! filters, against real responses from a loopback server.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_testkit::{Server, response};
use imposter_ui::app::HistoryTab;
use support::{click, collection, request, settle, start_with, text_input, type_into};

const BODY: &str =
    r#"{"data":{"items":[{"id":7,"name":"seven"},{"id":8,"name":"eight"}]},"total":2}"#;

struct Fixture {
    app: support::App,
    server: Server,
    _runtime: tokio::runtime::Runtime,
}

/// A request to a server answering `body` as `content_type`, sent once.
fn sent(content_type: &'static str, body: &'static str) -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(move |_, _| {
        response("200 OK", &[("Content-Type", content_type)], body.as_bytes())
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

/// Click `label` and return what that frame copied.
fn copy(app: &mut support::App, label: &str) -> Vec<String> {
    app.harness.get_by_label(label).click();
    app.harness.step();
    app.harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn shows(app: &support::App, text: &str) -> bool {
    app.harness.query_by_label_contains(text).is_some()
}

#[test]
fn json_folds_by_container_and_all_at_once() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    assert!(shows(app, "\"name\": \"seven\""));

    click(&mut app.harness, "Fold $.data.items[0]");
    assert!(
        !shows(app, "\"name\": \"seven\""),
        "folded contents are hidden"
    );
    assert!(shows(app, "\"name\": \"eight\""));
    app.harness.get_by_label_contains("{ … },");
    click(&mut app.harness, "Unfold $.data.items[0]");
    assert!(shows(app, "\"name\": \"seven\""));

    click(&mut app.harness, "Collapse all");
    assert!(!shows(app, "\"id\": 7"));
    assert!(shows(app, "\"total\": 2"), "the top level stays open");
    app.harness.get_by_label("Unfold $.data");
    click(&mut app.harness, "Expand all");
    assert!(shows(app, "\"id\": 7"));
}

#[test]
fn rows_with_fold_markers_keep_the_plain_row_height() {
    let fixture = sent("application/json", BODY);
    let harness = &fixture.app.harness;
    let top = |label: &str| harness.get_by_label_contains(label).rect().top();
    let step = top("\"name\": \"seven\"") - top("\"id\": 7");
    assert!(
        (10.0..20.0).contains(&step),
        "rows are placed by a fixed height; a taller row would drift: {step}"
    );
}

#[test]
fn the_tools_share_the_tab_row_and_the_rest_wait_under_more_tools() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    // The response's Body tab: the request's is further left.
    let body = app
        .harness
        .query_all_by_role_and_label(egui::accesskit::Role::Button, "Body")
        .map(|node| node.rect())
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    let copy = app.harness.get_by_label("Copy body").rect();
    let more = app.harness.get_by_label("More tools").rect();
    for tool in [copy, more] {
        assert!(
            (tool.center().y - body.center().y).abs() < 1.0,
            "{tool:?} is on the tab row"
        );
        assert!(tool.right() <= app.harness.ctx.content_rect().right());
    }
    assert!(
        app.harness.query_by_label("Collapse all").is_none(),
        "fold buttons give way first"
    );
    app.harness.get_by_label("Fold $.data");
    click(&mut app.harness, "More tools");
    click(&mut app.harness, "Filter with JSONPath");
    app.harness.run_steps(2);
    text_input(&app.harness, "Filter the response with JSONPath");
}

#[test]
fn find_opens_the_folds_that_hide_a_match() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    click(&mut app.harness, "Collapse all");
    assert!(!shows(app, "\"name\": \"eight\""));
    click(&mut app.harness, "Find");
    text_input(&app.harness, "Find in response body").type_text("eight");
    app.harness.run_steps(3);
    assert!(shows(app, "\"name\": \"eight\""));
    assert!(
        !shows(app, "\"name\": \"seven\""),
        "other folds stay closed"
    );
}

#[test]
fn history_responses_can_be_folded_and_filtered() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.state_mut().state.history.tab = HistoryTab::Response;
    app.harness.run_steps(2);

    click(&mut app.harness, "Fold $.data");
    assert!(!shows(app, "\"id\": 7"));
    click(&mut app.harness, "Filter with JSONPath");
    text_input(&app.harness, "Filter the response with JSONPath").type_text("$.total");
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.get_by_label("1 match");
    app.harness.get_by_label("  2");
}

#[test]
fn rows_copy_their_path_and_value() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;

    app.harness
        .get_by_label_contains("\"name\": \"eight\"")
        .click_secondary();
    app.harness.run_steps(2);
    assert_eq!(copy(app, "Copy path"), ["$.data.items[1].name"]);

    app.harness
        .get_by_label_contains("\"name\": \"eight\"")
        .click_secondary();
    app.harness.run_steps(2);
    assert_eq!(
        copy(app, "Copy value"),
        ["eight"],
        "strings are copied without quotes"
    );
}

#[test]
fn a_jsonpath_filter_shows_matches_with_their_real_paths() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;

    click(&mut app.harness, "Filter with JSONPath");
    text_input(&app.harness, "Filter the response with JSONPath").type_text("$..items[*].id");
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.get_by_label("2 matches");
    assert!(!shows(app, "\"name\""), "the results replace the body");

    app.harness.get_by_label("  8").click_secondary();
    app.harness.run_steps(2);
    assert_eq!(
        copy(app, "Copy path"),
        ["$.data.items[1].id"],
        "a result's path is its path in the body"
    );
    app.harness.run_steps(2);
    assert_eq!(copy(app, "Copy filtered result"), ["[\n  7,\n  8\n]"]);

    // Sending again filters the new response with the same query.
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    assert_eq!(fixture.server.requests().len(), 2);
    let app = &mut fixture.app;
    app.harness.get_by_label("2 matches");

    type_into(
        &mut app.harness,
        "Filter the response with JSONPath",
        " bad words",
    );
    settle(&mut app.harness);
    app.harness.get_by_label_contains("Invalid JSONPath");
    assert!(
        shows(app, "\"name\": \"seven\""),
        "a bad path shows the body"
    );

    click(&mut app.harness, "Close");
    assert!(
        app.harness
            .query_by_label("Filter the response with JSONPath")
            .is_none()
    );
    assert!(shows(app, "\"name\": \"seven\""));
}

#[test]
fn only_json_responses_offer_a_filter() {
    let mut fixture = sent("text/plain", "plain words");
    let app = &mut fixture.app;
    assert!(shows(app, "plain words"));
    assert!(app.harness.query_by_label("Filter with JSONPath").is_none());
    assert!(app.harness.query_by_label("Collapse all").is_none());
}

#[test]
fn a_body_larger_than_the_preview_can_be_saved_whole() {
    let big: &'static str =
        Box::leak(format!("[{}]", "1,".repeat(150 * 1024) + "1").into_boxed_str());
    let mut fixture = sent("application/json", big);
    let app = &mut fixture.app;
    app.harness.get_by_label("Preview shows 256 KB of 300 KB");
    let out = app.dir.path().join("saved.json");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(out.clone())]);
    click(&mut app.harness, "Save body…");
    settle(&mut app.harness);
    assert_eq!(
        std::fs::read(&out).unwrap(),
        big.as_bytes(),
        "the whole body"
    );
    app.harness.get_by_label("Saved the body to saved.json");
}
