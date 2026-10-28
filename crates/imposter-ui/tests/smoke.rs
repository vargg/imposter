//! `--smoke-test`: the app sends one request as soon as it started, then closes by itself.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::sync::{Arc, OnceLock};

use egui_kittest::Harness;
use imposter_app::paths::open_store;
use imposter_app::store::executions;
use imposter_testkit::{Server, response};
use imposter_ui::app::{CloseState, ImposterApp, SmokeResult};

/// Build the app the way `main` does: the smoke test is set before the first frame.
fn smoke(
    url: String,
) -> (
    tempfile::TempDir,
    Harness<'static, ImposterApp>,
    Arc<OnceLock<SmokeResult>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path()).unwrap();
    let report = Arc::new(OnceLock::new());
    let shared = Arc::clone(&report);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .build_eframe(move |cc| {
            let mut app = ImposterApp::new(&cc.egui_ctx, store).unwrap();
            app.smoke_test(url, shared);
            app
        });
    support::settle(&mut harness);
    (dir, harness, report)
}

#[test]
fn a_response_is_reported_recorded_and_the_app_closes() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("204 No Content", &[], b"")));

    let (_dir, harness, report) = smoke(format!("{}/smoke", server.url));

    assert_eq!(report.get(), Some(&Ok(204)));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(harness.state().close_state(), CloseState::Ready);
    let recorded = harness
        .state()
        .store()
        .call_blocking(|db| executions::count(db))
        .unwrap();
    assert_eq!(recorded, 1, "the send went through history like any other");
}

#[test]
fn a_failed_exchange_is_reported_and_the_app_still_closes() {
    // A port that was just free: nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let (_dir, harness, report) = smoke(format!("http://127.0.0.1:{port}/"));

    let error = report.get().unwrap().as_ref().unwrap_err();
    assert!(error.contains("TransportError"), "{error}");
    assert_eq!(harness.state().close_state(), CloseState::Ready);
}
