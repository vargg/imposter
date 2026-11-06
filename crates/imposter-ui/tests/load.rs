//! The Load screen: choosing the request, running, stopping, and the production question.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::time::Duration;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{EnvColor, Variables};
use imposter_app::store::environments;
use imposter_testkit::{LoadReply, LoadServer};
use imposter_ui::app::Screen;
use support::{click, click_button, collection, request, settle, start_with};

struct Fixture {
    app: support::App,
    server: LoadServer,
    _runtime: tokio::runtime::Runtime,
}

const LONG_ENV: &str = "Development environment with a deliberately long name for the status bar";

/// API holds "Ping" against a keep-alive server answering after `delay`.
fn fixture(delay: Duration, red: bool) -> Fixture {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(LoadServer::start(move |_, _| LoadReply {
        body_delay: delay,
        ..LoadReply::ok(b"pong")
    }));
    let url = format!("{}/ping", server.url);
    let app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Ping", &url);
        store
            .call_blocking(|db| environments::create_environment(db, LONG_ENV, &Variables::new()))
            .unwrap();
        if red {
            store
                .call_blocking(|db| {
                    let prod = environments::create_environment(db, "Prod", &Variables::new())?;
                    environments::set_environment_color(db, prod.id, Some(EnvColor::Red))?;
                    environments::set_active_environment(db, Some(prod.id))
                })
                .unwrap();
        }
    });
    Fixture {
        app,
        server,
        _runtime: runtime,
    }
}

fn set_plan(app: &mut support::App, requests: &str, concurrency: &str) {
    let form = &mut app.harness.state_mut().state.load.form;
    requests.clone_into(&mut form.requests);
    concurrency.clone_into(&mut form.concurrency);
    app.harness.run_steps(1);
}

#[test]
fn the_load_menu_takes_the_open_request_and_a_run_shows_its_summary() {
    let mut f = fixture(Duration::ZERO, false);
    click(&mut f.app.harness, "GET Ping");
    settle(&mut f.app.harness);
    support::open_screen(&mut f.app.harness, "Load");
    settle(&mut f.app.harness);
    assert_eq!(f.app.harness.state().state.screen, Screen::Load);
    f.app.harness.get_by_label("Ping");

    set_plan(&mut f.app, "20", "4");
    click(&mut f.app.harness, "Start the load test");
    settle(&mut f.app.harness);

    assert_eq!(f.server.requests().len(), 20);
    f.app.harness.get_by_label_contains("req/s · ");
    f.app.harness.get_by_label_contains("errors 0%");
    f.app.harness.get_by_label("200 ×20");
    f.app.harness.get_by_label("Served within");
    f.app.harness.get_by_label_contains("Latency histogram: ");
    let stored = f
        .app
        .harness
        .state()
        .store()
        .call_blocking(|db| imposter_app::store::executions::count(db))
        .unwrap();
    assert_eq!(stored, 0, "load runs are not history");
}

#[test]
fn the_sidebar_menu_opens_the_screen_on_its_request_and_stop_ends_a_run() {
    let mut f = fixture(Duration::from_millis(100), false);
    f.app.harness.get_by_label("GET Ping").click_secondary();
    f.app.harness.run_steps(2);
    click(&mut f.app.harness, "Load test…");
    settle(&mut f.app.harness);
    assert_eq!(f.app.harness.state().state.screen, Screen::Load);
    f.app.harness.get_by_label("Ping");

    set_plan(&mut f.app, "1000", "2");
    click(&mut f.app.harness, "Start the load test");
    f.app.harness.get_by_label_contains("Load test progress: ");
    let tab = f
        .app
        .harness
        .get_by_label("Load running")
        .accesskit_node()
        .description();
    assert_eq!(tab.as_deref(), Some("Load test running"));
    click(&mut f.app.harness, "Requests");
    click(&mut f.app.harness, "Load running");
    assert_eq!(f.app.harness.state().state.screen, Screen::Load);
    assert_eq!(
        f.app
            .harness
            .state()
            .state
            .load
            .target
            .as_ref()
            .unwrap()
            .name,
        "Ping"
    );
    click(&mut f.app.harness, "Stop the load test");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label_contains("Stopped after");
    assert!(f.server.requests().len() < 1000);
}

#[test]
fn both_running_tools_remain_reachable_with_unsaved_mocks_and_a_long_result() {
    use imposter_app::core::mock::{MockConfig, MockRoute};
    use imposter_app::store::kv::{self, Table};
    use imposter_ui::app::{Action, Appearance};
    use imposter_ui::theme::{Density, ThemeChoice};
    use std::time::Instant;

    let mut f = fixture(Duration::from_secs(60), false);
    f.app
        .harness
        .state()
        .store()
        .call_blocking(|db| {
            kv::set(
                db,
                Table::Settings,
                "mock_server",
                &MockConfig {
                    port: 0,
                    routes: vec![MockRoute::default()],
                },
            )
        })
        .unwrap();
    click(&mut f.app.harness, "GET Ping");
    settle(&mut f.app.harness);
    support::open_screen(&mut f.app.harness, "Mocks");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Start mock server");
    let deadline = Instant::now() + Duration::from_secs(10);
    while f
        .app
        .harness
        .state()
        .state
        .mocks
        .run
        .as_ref()
        .and_then(|run| run.address.as_ref())
        .is_none()
    {
        assert!(Instant::now() < deadline, "mock did not start");
        f.app.harness.run_steps(2);
        std::thread::yield_now();
    }
    let address = f
        .app
        .harness
        .state()
        .state
        .mocks
        .run
        .as_ref()
        .unwrap()
        .address
        .clone()
        .unwrap();
    let mock_label = format!("Mocks :{}", address.rsplit(':').next().unwrap());
    click(&mut f.app.harness, "Mock route 1");
    support::type_into(&mut f.app.harness, "Mock response body", "unsaved body");
    support::open_screen(&mut f.app.harness, "Load");
    set_plan(&mut f.app, "1000", "2");
    click(&mut f.app.harness, "Start the load test");
    click(&mut f.app.harness, "Requests");
    f.app.harness.set_size(egui::vec2(720.0, 480.0));
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        for density in [Density::Compact, Density::Comfortable] {
            f.app
                .harness
                .state_mut()
                .state
                .actions
                .push(Action::SetAppearance(Appearance { theme, density }));
            f.app.harness.run_steps(3);
            click(&mut f.app.harness, "Environment");
            click(&mut f.app.harness, LONG_ENV);
            let result = format!("Environment: {LONG_ENV}");
            let deadline = Instant::now() + Duration::from_secs(10);
            while f.app.harness.query_by_label(&result).is_none() {
                assert!(
                    Instant::now() < deadline,
                    "environment change did not finish"
                );
                f.app.harness.run_steps(2);
                std::thread::yield_now();
            }
            let window = f.app.harness.ctx.content_rect();
            let mocks = f.app.harness.get_by_label(&mock_label).rect();
            let load = f.app.harness.get_by_label("Load running").rect();
            assert!(window.contains_rect(mocks) && window.contains_rect(load));
            assert!(!mocks.intersects(load));
            assert!(!f.app.harness.get_by_label(&result).rect().intersects(mocks));
            assert!(mocks.height() >= 24.0 && load.height() >= 24.0);
            assert_eq!(
                f.app
                    .harness
                    .get_by_label("Tools")
                    .accesskit_node()
                    .description()
                    .as_deref(),
                Some("Unsaved mocks")
            );
            click(&mut f.app.harness, "Tools");
            let item = f
                .app
                .harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Mocks");
            assert_eq!(
                item.accesskit_node().description().as_deref(),
                Some("Running · Unsaved changes")
            );
            f.app.harness.key_press(egui::Key::Escape);
            f.app.harness.run_steps(2);
            click(&mut f.app.harness, "Environment");
            click(&mut f.app.harness, "Global (no environment)");
        }
    }
    click(&mut f.app.harness, &mock_label);
    assert_eq!(f.app.harness.state().state.screen, Screen::Mocks);
    assert!(f.app.harness.state().state.mocks.dirty());
    click(&mut f.app.harness, "Stop mock server");
    click(&mut f.app.harness, "Load running");
    assert_eq!(f.app.harness.state().state.screen, Screen::Load);
    assert_eq!(
        f.app
            .harness
            .state()
            .state
            .load
            .target
            .as_ref()
            .unwrap()
            .name,
        "Ping"
    );
    click(&mut f.app.harness, "Stop the load test");
    settle(&mut f.app.harness);
    assert!(f.app.harness.query_by_label("Load running").is_none());
    assert!(f.app.harness.query_by_label(&mock_label).is_none());
}

#[test]
fn a_red_environment_asks_before_starting() {
    let mut f = fixture(Duration::ZERO, true);
    click(&mut f.app.harness, "GET Ping");
    settle(&mut f.app.harness);
    support::open_screen(&mut f.app.harness, "Load");
    settle(&mut f.app.harness);
    set_plan(&mut f.app, "5", "1");
    click(&mut f.app.harness, "Start the load test");
    f.app.harness.run_steps(2);
    f.app
        .harness
        .get_by_label_contains("This sends \"Ping\" 5 times with the Prod environment.");
    assert_eq!(f.server.requests().len(), 0, "nothing before the answer");
    click_button(&mut f.app.harness, "Start load test");
    settle(&mut f.app.harness);
    assert_eq!(f.server.requests().len(), 5);
}

#[test]
fn a_plan_that_cannot_run_says_why_and_the_palette_chooses_the_request() {
    let mut f = fixture(Duration::ZERO, false);
    support::open_screen(&mut f.app.harness, "Load");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("Choose a request to load-test");
    set_plan(&mut f.app, "5", "10");
    f.app
        .harness
        .get_by_label("Concurrency must be between 1 and 256, and no more than the requests");

    click(&mut f.app.harness, "Choose the request to load-test");
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "GET API › Ping");
    settle(&mut f.app.harness);
    f.app.harness.get_by_label("Ping");
    assert!(f.app.harness.state().state.load.target.is_some());
}

#[test]
fn the_palette_load_tests_the_open_request() {
    let mut f = fixture(Duration::ZERO, false);
    click(&mut f.app.harness, "GET Ping");
    settle(&mut f.app.harness);
    f.app
        .harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    settle(&mut f.app.harness);
    click(&mut f.app.harness, "Load Test This Request");
    settle(&mut f.app.harness);
    assert_eq!(f.app.harness.state().state.screen, Screen::Load);
    let target = f.app.harness.state().state.load.target.clone().unwrap();
    assert_eq!(
        (target.name.as_str(), target.place.as_str()),
        ("Ping", "API")
    );
}

#[test]
fn a_request_with_scripts_says_they_run_and_shows_the_first_script_error() {
    let mut f = fixture(Duration::ZERO, false);
    click(&mut f.app.harness, "GET Ping");
    settle(&mut f.app.harness);
    f.app
        .harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .scripts
        .after = "let a = 1;\nthrow \"bad answer\";".into();
    support::open_screen(&mut f.app.harness, "Load");
    settle(&mut f.app.harness);
    f.app
        .harness
        .get_by_label(imposter_ui::load_screen::SCRIPTS_RUN);
    set_plan(&mut f.app, "3", "1");
    click(&mut f.app.harness, "Start the load test");
    settle(&mut f.app.harness);
    assert_eq!(f.server.requests().len(), 3);
    f.app
        .harness
        .get_by_label("After-response script, line 2: bad answer");
}
