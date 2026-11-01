//! The History screen: list, details, filter, paging, restore as draft and clearing.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::execution::{ExecutionRecord, Response, Timestamp};
use imposter_app::core::model::{Request, RequestTemplate};
use imposter_app::core::settings::AppSettings;
use imposter_app::store::{collections, executions, kv};
use imposter_testkit::{Server, response};
use imposter_ui::app::{HistoryTab, Screen};
use support::{click, click_button, collection, request, settle, start_with, type_into};

fn open_history(app: &mut support::App) {
    click(&mut app.harness, "History");
    settle(&mut app.harness);
}

fn sent_app() -> (support::App, Server, tokio::runtime::Runtime) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| {
        response("201 Created", &[("Content-Type", "text/plain")], b"made it")
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Create", &url);
        request(store, api.id, "Other", "http://other.invalid");
    });
    click(&mut app.harness, "GET Create");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    (app, server, runtime)
}

#[test]
fn executions_are_listed_with_their_request_and_response() {
    let (mut app, server, _runtime) = sent_app();
    open_history(&mut app);

    app.harness.get_by_label("Executions 1-1");
    let row = app.harness.get_by_label_contains("History GET");
    row.click();
    app.harness.run_steps(1);
    settle(&mut app.harness);

    // The response first, as that is what a run is looked up for.
    assert_eq!(app.harness.state().state.history.tab, HistoryTab::Response);
    // One line: the request as sent, how it went and how long it took.
    app.harness
        .get_by_label_contains(&format!("GET {}/items  ·  201 Created  ·  ", server.url));
    // The request as sent: no JSON dump of empty template fields, and nothing printed as
    // code when there is nothing to show.
    click(&mut app.harness, "Request body");
    app.harness.get_by_label("No body");
    click(&mut app.harness, "Request headers");
    app.harness.get_by_label("No headers");
    click(&mut app.harness, "Response");
    assert_eq!(app.harness.state().state.history.tab, HistoryTab::Response);
    app.harness.get_by_label("201 Created");
    app.harness.get_by_label_contains("made it");
}

#[test]
fn restoring_opens_an_unsaved_draft_without_sending() {
    let (mut app, server, _runtime) = sent_app();
    open_history(&mut app);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);

    click(&mut app.harness, "Restore as draft");
    settle(&mut app.harness);

    assert_eq!(app.harness.state().state.screen, Screen::Requests);
    app.harness.get_by_label("Unsaved draft / Create");
    assert_eq!(server.requests().len(), 1, "restoring never sends");
    assert!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .is_draft()
    );
}

#[test]
fn restoring_over_unsaved_changes_asks_first() {
    let (mut app, _server, _runtime) = sent_app();
    app.harness
        .state()
        .store()
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
    app.harness.state_mut().state.settings.autosave = false;
    type_into(&mut app.harness, "Request URL", "/edited");
    open_history(&mut app);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);

    click(&mut app.harness, "Restore as draft");
    app.harness
        .get_by_label("Save changes to \"Create\" before continuing?");
    click(&mut app.harness, "Cancel");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::History);
}

#[test]
fn the_filter_shows_only_the_open_request() {
    let (mut app, _server, _runtime) = sent_app();
    click(&mut app.harness, "Requests");
    click(&mut app.harness, "GET Other");
    settle(&mut app.harness);
    open_history(&mut app);
    app.harness.get_by_label("Executions 1-1");

    click(&mut app.harness, "Current request only");
    settle(&mut app.harness);

    app.harness.get_by_label("No executions yet");
}

#[test]
fn clearing_asks_and_removes_everything() {
    let (mut app, _server, _runtime) = sent_app();
    open_history(&mut app);

    click(&mut app.harness, "History actions");
    click(&mut app.harness, "Clear all history…");
    click_button(&mut app.harness, "Delete history");
    settle(&mut app.harness);

    app.harness.get_by_label("No executions yet");
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| executions::count(db))
            .unwrap(),
        0
    );
}

#[test]
fn pages_move_between_newer_and_older() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                for i in 0..55 {
                    let request = Request {
                        template: RequestTemplate {
                            url: format!("http://h/{i}"),
                            ..RequestTemplate::default()
                        },
                        ..Request::default()
                    };
                    let record = ExecutionRecord {
                        timestamp: Timestamp(1_700_000_000_000 + i),
                        request_id: None,
                        template: request.clone(),
                        resolved: request,
                        settings: AppSettings::default(),
                        environment_id: None,
                        environment: "Global".into(),
                        response: Response {
                            status: 200,
                            ..Response::default()
                        },
                    };
                    executions::record(db, &record, 1000, u64::MAX)?;
                }
                Ok(())
            })
            .unwrap();
    });
    open_history(&mut app);
    app.harness.get_by_label("Executions 1-50");

    click(&mut app.harness, "Older");
    settle(&mut app.harness);
    app.harness.get_by_label("Executions 51-55");
    click(&mut app.harness, "Newer");
    settle(&mut app.harness);
    app.harness.get_by_label("Executions 1-50");
}

#[test]
fn the_filter_finds_runs_by_status_method_and_text() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|index, _| {
        let status = ["200 OK", "404 Not Found", "500 Internal Server Error"][index % 3];
        response(status, &[], b"")
    }));
    let users = format!("{}/users", server.url);
    let orders = format!("{}/orders", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Users", &users);
        request(store, api.id, "Orders", &orders);
    });
    for name in ["GET Users", "GET Users", "GET Orders"] {
        click(&mut app.harness, name);
        settle(&mut app.harness);
        click(&mut app.harness, "Send");
        settle(&mut app.harness);
    }
    open_history(&mut app);
    app.harness.get_by_label("Executions 1-3");

    let shown = |app: &support::App| -> Vec<String> {
        app.harness
            .state()
            .state
            .history
            .rows
            .iter()
            .map(|r| format!("{} {}", r.name, r.status))
            .collect()
    };
    type_into(&mut app.harness, "Filter history", "status:4xx");
    settle(&mut app.harness);
    app.harness.get_by_label("Matches 1-1");
    assert_eq!(shown(&app), ["Users 404"]);

    type_into(&mut app.harness, "Filter history", " OR orders");
    settle(&mut app.harness);
    assert_eq!(shown(&app), ["Orders 500", "Users 404"]);

    app.harness.state_mut().state.history.filter = "status:2xx -users".into();
    app.harness
        .state_mut()
        .state
        .actions
        .push(imposter_ui::app::Action::History(
            imposter_ui::app::HistoryAction::Filtered,
        ));
    settle(&mut app.harness);
    app.harness.get_by_label("No matching executions");
}

#[test]
fn leaving_a_restored_draft_offers_to_keep_it_as_a_new_request() {
    let (mut app, _server, _runtime) = sent_app();
    open_history(&mut app);
    app.harness.get_by_label_contains("History GET").click();
    app.harness.run_steps(1);
    settle(&mut app.harness);
    click(&mut app.harness, "Restore as draft");
    settle(&mut app.harness);

    click(&mut app.harness, "GET Other");
    settle(&mut app.harness);
    app.harness.get_by_label("Keep this draft?");
    app.harness.get_by_label(
        "\"Create\" is not saved yet. Save it as a new request in \"API\", discard it, or stay.",
    );
    click_button(&mut app.harness, "Stay");
    settle(&mut app.harness);
    assert!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .is_draft(),
        "staying keeps the draft"
    );

    click(&mut app.harness, "GET Other");
    settle(&mut app.harness);
    click_button(&mut app.harness, "Save as new request");
    settle(&mut app.harness);
    // Saving a draft asks for its name, prefilled.
    click_button(&mut app.harness, "Save");
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            let api = collections::collections(db)?[0].id;
            collections::tree(db, api)
        })
        .unwrap()
        .unwrap()
        .requests
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(
        names.iter().filter(|n| n.starts_with("Create")).count(),
        2,
        "the draft became a new request beside the original: {names:?}"
    );
}

/// Painted text runs and their colors.
fn painted(app: &support::App) -> Vec<(String, egui::Color32)> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.job.clone()),
            _ => None,
        })
        .flat_map(|job| {
            job.sections
                .iter()
                .map(|section| {
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    (job.text[range].to_owned(), section.format.color)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_list_is_a_table_of_name_method_url_status_and_time() {
    let (mut app, server, _runtime) = sent_app();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    open_history(&mut app);
    for title in [
        "When",
        "Name",
        "Method",
        "URL",
        "Status",
        "Time",
        "Environment",
    ] {
        app.harness
            .get_by_role_and_label(egui::accesskit::Role::Label, title);
    }
    let palette = imposter_ui::theme::palette();
    let runs = painted(&app);
    let has = |text: &str, color: egui::Color32| runs.iter().any(|(t, c)| t == text && *c == color);
    assert!(has("just now", palette.text_muted), "relative time");
    assert!(has("Create", palette.text), "the request's name");
    // The origin repeats from row to row, so it is muted; the path is not.
    assert!(has(&server.url, palette.text_muted), "{runs:?}");
    assert!(has("/items", palette.text));
    assert!(has("201", palette.ok), "a status pill");
    // The exact time is a hover away.
    let row = app.harness.get_by_label_contains("History GET");
    assert!(
        row.accesskit_node()
            .description()
            .is_some_and(|d| d.contains(':'))
    );

    // Clearing everything is not beside the everyday tools.
    assert!(app.harness.query_by_label("Clear all history…").is_none());

    // Narrow, the environment column gives way to the URL.
    app.harness.set_size(imposter_ui::layout::MIN_WINDOW.into());
    app.harness.run_steps(2);
    assert!(
        app.harness
            .query_by_role_and_label(egui::accesskit::Role::Label, "Environment")
            .is_none()
    );
}
