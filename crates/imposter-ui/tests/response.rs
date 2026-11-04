//! The response viewer: folding formatted JSON, copying paths and values, and JSONPath
//! filters, against real responses from a loopback server.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
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
    // Toggles in the menu are check boxes showing their state.
    click(&mut app.harness, "More tools");
    let wrap = app
        .harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Wrap");
    assert_eq!(wrap.accesskit_node().toggled(), Some(Toggled::True));
    click(&mut app.harness, "Wrap");
    assert!(!app.harness.state().state.response.body_view.wrap);
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
fn the_line_under_the_pointer_offers_its_actions_without_a_right_click() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    assert!(
        app.harness
            .query_by_label("Row actions $.data.items[1].name")
            .is_none()
    );
    let line = app
        .harness
        .get_by_label_contains("\"name\": \"eight\"")
        .rect();
    app.harness.hover_at(line.center());
    app.harness.run_steps(2);
    let button = app
        .harness
        .get_by_label("Row actions $.data.items[1].name")
        .rect();
    assert!(
        (button.center().y - line.center().y).abs() < 4.0,
        "on the line: {button:?} {line:?}"
    );
    click(&mut app.harness, "Row actions $.data.items[1].name");
    // The menu says what it acts on, opens under the line so it stays in view, and stays
    // while the pointer moves into it.
    let header = app.harness.get_by_label("$.data.items[1].name").rect();
    assert!(header.top() >= line.bottom(), "{header:?} under {line:?}");
    let item = app.harness.get_by_label("Copy path").rect();
    app.harness.hover_at(item.center());
    app.harness.run_steps(2);
    assert_eq!(copy(app, "Copy path"), ["$.data.items[1].name"]);
}

#[test]
fn wrapped_lines_stop_short_of_the_row_actions() {
    let long: &'static str =
        Box::leak(format!(r#"{{"note":"{}"}}"#, "word ".repeat(80)).into_boxed_str());
    let mut fixture = sent("application/json", long);
    let app = &mut fixture.app;
    let line = app.harness.get_by_label_contains("\"note\"").rect();
    app.harness.hover_at(line.center());
    app.harness.run_steps(2);
    let button = app.harness.get_by_label("Row actions $.note").rect();
    let line = app.harness.get_by_label_contains("\"note\"").rect();
    assert!(
        line.right() <= button.left(),
        "{line:?} runs under {button:?}"
    );

    // Too narrow for the wrap's 20 columns, the line reaches the edge: the "⋯" is drawn
    // on a fill over it.
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    let line = app.harness.get_by_label_contains("\"note\"").rect();
    app.harness.hover_at(line.center());
    app.harness.run_steps(2);
    let button = app.harness.get_by_label("Row actions $.note").rect();
    let shapes = &app.harness.output().shapes;
    let text = shapes
        .iter()
        .rposition(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.job.text.contains("\"note\"") && text.pos.y < button.bottom())
        })
        .unwrap();
    let fill = shapes
        .iter()
        .rposition(|clipped| {
            matches!(&clipped.shape, egui::Shape::Rect(rect)
                if rect.rect == button && rect.fill == imposter_ui::theme::palette().hover)
        })
        .unwrap();
    assert!(fill > text, "the fill is drawn over the line");
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
    // One line at the smallest size: the notice, then the actions as icons at its end.
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    let notice = app
        .harness
        .get_by_label("Preview shows 256 KB of 300 KB")
        .rect();
    let save = app.harness.get_by_label("Save body…").rect();
    let copy = app.harness.get_by_label("Copy full body").rect();
    for action in [save, copy] {
        assert!(
            (action.center().y - notice.center().y).abs() < 2.0,
            "{action:?} {notice:?}"
        );
    }
    assert!(notice.right() <= save.left() && save.right() <= copy.left());
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

#[test]
fn headers_show_as_a_table_with_a_copy_per_row_and_raw_on_request() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    app.harness
        .query_all_by_role_and_label(egui::accesskit::Role::Button, "Headers")
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .click();
    app.harness.run_steps(2);
    let name = app.harness.get_by_label("content-type").rect();
    // The status line shows the content type too; the table's is below it.
    let value = app
        .harness
        .query_all_by_label("application/json")
        .map(|node| node.rect())
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    assert!(
        (name.top() - value.top()).abs() < 1.0 && name.right() < value.left(),
        "one row, two columns"
    );
    // Too narrow for two columns, the value goes under its name, whole.
    app.harness
        .set_size(egui::Vec2::from(imposter_ui::layout::MIN_WINDOW));
    app.harness.run_steps(3);
    let table_value = |app: &support::App| {
        app.harness
            .query_all_by_label("application/json")
            .map(|node| node.rect())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .unwrap()
    };
    let narrow_name = app.harness.get_by_label("content-type").rect();
    let narrow_value = table_value(app);
    assert!(
        narrow_value.top() >= narrow_name.bottom() - 0.5
            && (narrow_value.left() - narrow_name.left()).abs() < 1.0,
        "{narrow_name:?} above {narrow_value:?}"
    );
    let line = app.harness.ctx.global_style().text_styles[&egui::TextStyle::Monospace].size;
    assert!(
        narrow_value.height() < 2.0 * line,
        "on one line: {narrow_value:?}"
    );
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.run_steps(3);
    let value = table_value(app);
    app.harness.hover_at(value.center());
    app.harness.run_steps(2);
    app.harness.get_by_label("Copy content-type").click();
    let mut copied = None;
    for _ in 0..3 {
        app.harness.step();
        copied = copied.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
        });
    }
    assert_eq!(copied.as_deref(), Some("content-type: application/json"));

    // Raw is a toggle at the row's right end, after Copy, not a tab after Tests.
    let raw = app.harness.get_by_label("Raw");
    assert_eq!(raw.accesskit_node().toggled(), Some(Toggled::False));
    assert!(raw.rect().left() > app.harness.get_by_label("Copy headers").rect().right());
    click(&mut app.harness, "Raw");
    app.harness
        .get_by_label_contains("content-type: application/json");
    app.harness.get_by_label("Find");
    let raw = app.harness.get_by_label("Raw");
    assert_eq!(raw.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn the_request_as_sent_shows_the_values_generated_for_it() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"ok")));
    let url = format!("{}/items/{{{{$uuid}}}}", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Create", &url);
    });
    // Wide, so the request line is one row.
    app.harness.set_size(egui::vec2(1800.0, 900.0));
    click(&mut app.harness, "GET Create");
    settle(&mut app.harness);
    assert!(
        app.harness.query_by_label("Request as sent").is_none(),
        "nothing sent yet"
    );
    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    let line = server.requests()[0].request_line.clone();
    let uuid = line
        .split("/items/")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap()
        .to_owned();
    assert_eq!(uuid.len(), 36, "{line}");
    support::open_response_tab(&mut app.harness, "Request as sent");
    app.harness
        .get_by_label_contains(&format!("GET {}/items/{uuid}", server.url));
    app.harness.get_by_label("Copy request");

    // Another send generates another value, and the tab follows it.
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    assert!(app.harness.query_by_label_contains(&uuid).is_none());
}

#[test]
fn a_binary_body_is_described_and_saved_as_received() {
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0xff,
    ];
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response("200 OK", &[("Content-Type", "image/png")], PNG)
    }));
    let url = format!("{}/logo.png", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Logo", &url);
    });
    click(&mut app.harness, "GET Logo");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    app.harness.get_by_label("Binary body");
    app.harness.get_by_label("13 B · image/png");
    assert!(
        app.harness.query_by_label("Copy body").is_none(),
        "bytes are not copied as text"
    );
    assert!(app.harness.query_by_label("Find").is_none());

    let out = app.dir.path().join("logo.png");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(out.clone())]);
    click(&mut app.harness, "Save body…");
    settle(&mut app.harness);
    assert_eq!(std::fs::read(&out).unwrap(), PNG, "byte for byte");
    drop(server);
}

#[test]
fn a_small_text_body_can_be_saved_too() {
    let mut fixture = sent("application/json", BODY);
    let app = &mut fixture.app;
    let out = app.dir.path().join("small.json");
    app.harness
        .state_mut()
        .script_file_dialogs([Some(out.clone())]);
    click(&mut app.harness, "Save body…");
    settle(&mut app.harness);
    assert_eq!(std::fs::read(&out).unwrap(), BODY.as_bytes());
}
