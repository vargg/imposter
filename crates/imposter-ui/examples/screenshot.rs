//! Render the real app with sample data to PNG files, for looking at the layout without a
//! display: `cargo run -p imposter-ui --example screenshot -- <output-dir>`.
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop
#![allow(clippy::unwrap_used)] // developer tool, not app code

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use imposter_app::core::capture::Capture;
use imposter_app::core::check::{Check, Op};
use imposter_app::core::model::{BodyType, ParamItem, RequestTemplate};
use imposter_app::core::select::Selector;
use imposter_app::paths::open_store;
use imposter_app::store::collections;
use imposter_ui::app::{ImposterApp, ResponseTab};

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(imposter_testkit::Server::start(|_, _| {
        imposter_testkit::response(
            "200 OK",
            &[("Content-Type", "application/json"), ("X-Request-Id", "abc-123")],
            br#"{"users":[{"id":1,"name":"Ann","admin":true},{"id":2,"name":"Bo","admin":false}],"total":2,"next":null}"#,
        )
    }));
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open_store(dir.path()).unwrap();
        let url = format!("{}/users?page=1", server.url);
        store
            .call_blocking(move |db| {
                let api = collections::create_collection(
                    db,
                    "Example API",
                    "",
                    &imposter_app::core::model::Variables::new(),
                )?;
                let users = collections::create_folder(db, api.id, None, "Users", "", 1)?;
                collections::create_folder(db, api.id, Some(users.id), "Admin", "", 0)?;
                let list = RequestTemplate {
                    name: "List users".into(),
                    url,
                    queries: vec![ParamItem::new("page", "1")],
                    headers: vec![ParamItem::new("Accept", "application/json")],
                    body_type: BodyType::None,
                    checks: vec![
                        Check::new(Selector::Status, Op::Equals, "2xx"),
                        Check::new(Selector::Json("$.total".into()), Op::Equals, "2"),
                        Check::new(Selector::Json("$.users[0].name".into()), Op::Equals, "Bo"),
                        Check::new(Selector::Duration, Op::Less, "500"),
                    ],
                    captures: vec![Capture::new(
                        "first_user",
                        Selector::Json("$.users[0].id".into()),
                    )],
                    ..RequestTemplate::default()
                };
                collections::create_request(db, api.id, Some(users.id), &list)?;
                for (name, method, position) in [
                    ("Health", "GET", 0),
                    ("Create user", "POST", 2),
                    ("Delete user", "DELETE", 3),
                ] {
                    let template = RequestTemplate {
                        name: name.into(),
                        method: method.into(),
                        position,
                        ..RequestTemplate::default()
                    };
                    collections::create_request(db, api.id, None, &template)?;
                }
                let mut saved = imposter_app::core::model::SavedValues::default();
                saved.set("user_id", ["42".into(), "77".into(), "91".into()]);
                imposter_app::store::scopes::write(
                    db,
                    imposter_app::core::variables::Scope::Global,
                    &[
                        ("user_id".to_owned(), "42".to_owned()),
                        ("token".to_owned(), String::new()),
                    ]
                    .into_iter()
                    .collect(),
                    &saved,
                )?;
                imposter_app::store::kv::set(
                    db,
                    imposter_app::store::kv::Table::Settings,
                    "mock_server",
                    &imposter_app::core::mock::MockConfig {
                        port: 8080,
                        routes: vec![imposter_app::core::mock::MockRoute {
                            path: "/users".into(),
                            body: r#"{"users":[{"id":1,"name":"Ann"}]}"#.into(),
                            headers: vec![ParamItem::new("Content-Type", "application/json")],
                            ..Default::default()
                        }],
                    },
                )?;
                Ok(())
            })
            .unwrap();
        store.close().unwrap();
    }
    for (label, size) in [
        ("narrow", egui::Vec2::from(imposter_ui::layout::MIN_WINDOW)),
        ("wide", egui::vec2(1500.0, 850.0)),
    ] {
        let store = open_store(dir.path()).unwrap();
        let mut harness = Harness::builder()
            .with_size(size)
            .with_max_steps(64)
            .wgpu()
            .build_eframe(move |cc| ImposterApp::new(&cc.egui_ctx, store).unwrap());
        let settle = |harness: &mut Harness<'_, ImposterApp>| {
            for _ in 0..20 {
                harness.state().bridge().wait_idle(Duration::from_secs(10));
                harness.run_steps(2);
            }
        };
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-empty.png")))
            .unwrap();
        harness.get_by_label("GET List users").click();
        settle(&mut harness);
        harness.get_by_label("Send").click();
        settle(&mut harness);
        let image = harness.render().unwrap();
        let path = out.join(format!("{label}.png"));
        image.save(&path).unwrap();
        println!("{}", path.display());
        harness.get_by_label("Tests").click();
        harness.get_by_label("Check results").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-checks.png")))
            .unwrap();
        harness.state_mut().state.response.tab = ResponseTab::Body;
        settle(&mut harness);
        harness.get_by_label("Fold $.users[0]").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-folded.png")))
            .unwrap();
        harness.get_by_label("Filter with JSONPath").click();
        settle(&mut harness);
        harness
            .get_by_label("Filter the response with JSONPath")
            .type_text("$.users[?@.admin == false].name");
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-json-filter.png")))
            .unwrap();
        harness.get_by_label("Close").click();
        settle(&mut harness);
        harness.get_by_label("New").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-new-menu.png")))
            .unwrap();
        harness.key_press(egui::Key::Escape);
        settle(&mut harness);
        harness.get_by_label("Hide Collections").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-sidebar-hidden.png")))
            .unwrap();
        harness.get_by_label("Stack request and response").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-stacked.png")))
            .unwrap();
        harness
            .get_by_label("Put request and response side by side")
            .click();
        settle(&mut harness);
        harness.get_by_label("Show Collections").click();
        settle(&mut harness);
        harness.get_by_label("HTTP method").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-method-menu.png")))
            .unwrap();
        harness.get_by_label("Custom method…").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-custom-method.png")))
            .unwrap();
        harness.get_by_label("Mocks").click();
        settle(&mut harness);
        harness.get_by_label("Mock route 1").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-mocks.png")))
            .unwrap();
        harness.get_by_label("Variables").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-variables.png")))
            .unwrap();
        harness.get_by_label("Variable 1 saved values").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-saved-values.png")))
            .unwrap();
        harness.get_by_label("History").click();
        settle(&mut harness);
        harness
            .get_all_by_label_contains("History GET")
            .next()
            .unwrap()
            .click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-history.png")))
            .unwrap();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
        settle(&mut harness);
        harness
            .get_by_label("Command palette search")
            .type_text("user");
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-palette.png")))
            .unwrap();
        harness.key_press(egui::Key::Escape);
        settle(&mut harness);
        harness.get_by_label("Requests").click();
        settle(&mut harness);
        harness.state_mut().state.sidebar_filter = "method:POST".into();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-filter.png")))
            .unwrap();
    }
}
