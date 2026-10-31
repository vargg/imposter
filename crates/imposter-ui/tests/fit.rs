//! The smallest window: rows of tabs and tools stay on one line, and what does not fit moves
//! into a menu instead of wrapping onto more rows.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_ui::app::EditorTab;
use imposter_ui::layout::MIN_WINDOW;
use support::{click, collection, request, settle, start_with};

const EDITOR_TABS: [&str; 7] = [
    "Body", "Auth", "Query", "Headers", "Cookies", "Docs", "Tests",
];

fn small() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Users", "http://127.0.0.1:9/users");
    });
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    app
}

/// The tops of the request tabs in the row, by role so the screen tabs are left out.
fn editor_tab_tops(app: &support::App) -> Vec<f32> {
    let mut labels: Vec<&str> = EDITOR_TABS.to_vec();
    labels.push("Request settings");
    app.harness
        .query_all_by_role(Role::Button)
        .filter(|node| {
            let label = node.accesskit_node().label().unwrap_or_default();
            labels.contains(&label.as_str()) && node.rect().top() > 60.0
        })
        .map(|node| node.rect().top())
        .collect()
}

#[test]
fn request_tabs_keep_one_row_and_the_rest_are_under_more() {
    let mut app = small();
    let tops = editor_tab_tops(&app);
    assert!(tops.len() < 8, "some tabs moved into the menu");
    assert!(
        tops.iter().all(|top| (top - tops[0]).abs() < 1.0),
        "one row: {tops:?}"
    );
    app.harness
        .get_by_role_and_label(Role::Button, "More request tabs")
        .click();
    app.harness.run_steps(2);
    click(&mut app.harness, "Tests");
    app.harness.run_steps(2);
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .map(|pane| pane.tab),
        Some(EditorTab::Tests)
    );
    let tops = editor_tab_tops(&app);
    assert!(
        tops.iter().all(|top| (top - tops[0]).abs() < 1.0),
        "the chosen tab joins the row: {tops:?}"
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Tests")
        .rect();
}

#[test]
fn a_wide_window_shows_every_request_tab() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.run_steps(3);
    assert_eq!(editor_tab_tops(&app).len(), 8);
    assert!(app.harness.query_by_label("More request tabs").is_none());
}

#[test]
fn the_status_and_runs_share_one_row_and_an_earlier_run_can_be_shown() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = runtime.block_on(imposter_testkit::Server::start(move |_, _| {
        let n = sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let status = if n == 0 {
            "201 Created"
        } else {
            "404 Not Found"
        };
        imposter_testkit::response(status, &[("Content-Type", "application/json")], b"{}")
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    for _ in 0..2 {
        click(&mut app.harness, "Send");
        settle(&mut app.harness);
    }
    let status = app.harness.get_by_label("404 Not Found").rect();
    let runs = app.harness.get_by_label("Recorded runs").rect();
    assert!(
        (status.center().y - runs.center().y).abs() < 2.0,
        "{status:?} and {runs:?} share a row"
    );
    assert!(runs.right() <= app.harness.ctx.content_rect().right());
    click(&mut app.harness, "Recorded runs");
    app.harness.get_by_label_contains("· 201").click();
    app.harness.run_steps(2);
    settle(&mut app.harness);
    app.harness.get_by_label("201 Created");
    assert!(app.harness.query_by_label("404 Not Found").is_none());
}

/// The top of the stacked split's handle, found by its resize cursor along `x`.
fn handle_top(app: &mut support::App, x: f32) -> f32 {
    let mut y = 100.0;
    while y < 800.0 {
        app.harness.hover_at(egui::pos2(x, y));
        app.harness.run_steps(1);
        if app.harness.output().platform_output.cursor_icon == egui::CursorIcon::ResizeVertical {
            return y;
        }
        y += 1.0;
    }
    panic!("no split handle along x = {x}");
}

/// Drag the split handle whose top is at `top` along `x` by `by` points, in steps.
fn drag_handle(app: &mut support::App, x: f32, top: f32, by: f32) {
    let grab = top + 2.0;
    app.harness.drag_at(egui::pos2(x, grab));
    app.harness.run_steps(1);
    for step in 1..=10_u8 {
        app.harness
            .hover_at(egui::pos2(x, grab + by * f32::from(step) / 10.0));
        app.harness.run_steps(1);
    }
    app.harness.drop_at(egui::pos2(x, grab + by));
    app.harness.run_steps(2);
}

#[test]
fn a_stacked_pane_keeps_its_minimum_and_follows_the_drag_back_at_once() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.state_mut().state.layout.orientation = imposter_ui::layout::Orientation::Stacked;
    app.harness.run_steps(3);
    let x = 900.0;
    let body = app
        .harness
        .query_all_by_role_and_label(Role::Button, "Body")
        .map(|node| node.rect().top())
        .fold(f32::INFINITY, f32::min);
    let start = handle_top(&mut app, x);
    drag_handle(&mut app, x, start, -600.0);
    let lowest = handle_top(&mut app, x);
    // The request pane's tabs sit a padding below its top.
    let pane = lowest - body;
    assert!(
        (pane - imposter_ui::layout::MIN_PANE_TALL).abs() <= 20.0,
        "held near its minimum: {pane}"
    );
    drag_handle(&mut app, x, lowest, 50.0);
    let back = handle_top(&mut app, x);
    assert!(
        (back - lowest - 50.0).abs() <= 2.0,
        "the drag back moves the pane at once: {lowest} -> {back}"
    );
}

#[test]
fn the_body_editor_fills_its_pane_with_its_footer_below() {
    let mut app = small();
    for size in [egui::vec2(1500.0, 850.0), egui::Vec2::from(MIN_WINDOW)] {
        app.harness.set_size(size);
        {
            let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
            pane.editor.body_type = imposter_app::core::model::BodyType::Json;
            pane.editor.body_text = "{\"a\": 1}".into();
            pane.tab = EditorTab::Body;
        }
        app.harness.run_steps(3);
        let footer = app.harness.get_by_label("Valid JSON").rect();
        let status_bar = app.harness.get_by_label("Hide Collections").rect().top();
        // The pane ends above the status bar, inside its frame and padding.
        let gap = status_bar - footer.bottom();
        assert!(
            (0.0..60.0).contains(&gap),
            "{size:?}: the footer ends {gap} above the status bar"
        );
    }
}

#[test]
fn request_tabs_count_what_they_hold() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.headers = vec![
            imposter_app::core::model::ParamItem::new("Accept", "*/*"),
            imposter_app::core::model::ParamItem::new("X-Trace", "1"),
        ];
    }
    app.harness.run_steps(3);
    let headers = app.harness.get_by_role_and_label(Role::Button, "Headers");
    assert_eq!(
        headers.accesskit_node().description().as_deref(),
        Some("2 in use")
    );
    let auth = app.harness.get_by_role_and_label(Role::Button, "Auth");
    assert_eq!(auth.accesskit_node().description(), None, "nothing set");
}

#[test]
fn more_marks_hidden_tabs_that_hold_something() {
    let mut app = small();
    let more = |app: &support::App| {
        app.harness
            .get_by_role_and_label(Role::Button, "More request tabs")
            .accesskit_node()
            .description()
    };
    assert_eq!(more(&app).as_deref(), Some("More tabs"), "nothing set");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .headers = vec![imposter_app::core::model::ParamItem::new("Accept", "*/*")];
    app.harness.run_steps(3);
    assert_eq!(more(&app).as_deref(), Some("More tabs: Headers: 1 in use"));
}

#[test]
fn the_check_marks_keep_their_place_in_the_smallest_window() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(imposter_testkit::Server::start(|_, _| {
        imposter_testkit::response("200 OK", &[("Content-Type", "application/json")], b"{}")
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .checks = vec![
        imposter_app::core::check::Check::new(
            imposter_app::core::select::Selector::Status,
            imposter_app::core::check::Op::Equals,
            "200",
        ),
        imposter_app::core::check::Check::new(
            imposter_app::core::select::Selector::Status,
            imposter_app::core::check::Op::Equals,
            "201",
        ),
    ];
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    let checks = app
        .harness
        .get_by_label("Checks: 1 passed, 1 failed")
        .rect();
    let runs = app.harness.get_by_label("Recorded runs").rect();
    assert!(checks.right() <= runs.left(), "{checks:?} before {runs:?}");
}
