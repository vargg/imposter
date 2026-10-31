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
