//! The History screen: list, details, filter, paging, restore as draft and clearing.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use poster_app::core::execution::{ExecutionRecord, Response, Timestamp};
use poster_app::core::model::{Request, RequestTemplate};
use poster_app::core::settings::AppSettings;
use poster_app::store::{executions, kv};
use poster_testkit::{Server, response};
use poster_ui::app::{HistoryTab, Screen};
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

    // The request as sent: no JSON dump of empty template fields.
    app.harness
        .get_by_label(&format!("GET {}/items", server.url));
    app.harness.get_by_label("No body");
    click(&mut app.harness, "Request headers");
    app.harness.get_by_label("No headers");
    app.harness.state_mut().state.history.tab = HistoryTab::Response;
    app.harness.run_steps(2);
    app.harness.get_by_label("Status: 201");
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
