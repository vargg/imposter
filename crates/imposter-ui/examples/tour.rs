//! A tour of the app for design reviews: the primary tasks step by step and the states a
//! review needs (empty, loading, errors, dialogs), rendered offscreen at the minimum and a
//! wide window size. `cargo run -p imposter-ui --example tour -- <output-dir>`.
//!
//! Files are named `<size>-<task>-<step>-<what>.png`; a step whose control is missing is
//! reported and skipped, so a changed label shows up as a gap rather than a crash.
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop
#![allow(clippy::unwrap_used)] // developer tool, not app code

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::capture::Capture;
use imposter_app::core::check::{Check, Op};
use imposter_app::core::mock::{MockConfig, MockRoute};
use imposter_app::core::model::{
    BodyType, EnvColor, ParamItem, RequestAuth, RequestTemplate, SavedValues, Variables,
};
use imposter_app::core::select::Selector;
use imposter_app::core::variables::Scope;
use imposter_app::paths::open_store;
use imposter_app::store::{collections, environments, kv, scopes};
use imposter_ui::app::{HistoryTab, ImposterApp, ResponseTab};
use imposter_ui::theme::ThemeChoice;

struct Tour {
    harness: Harness<'static, ImposterApp>,
    out: PathBuf,
    /// The file name prefix: the size, and the theme unless it is dark.
    size: String,
}

impl Tour {
    fn settle(&mut self) {
        for _ in 0..20 {
            self.harness
                .state()
                .bridge()
                .wait_idle(Duration::from_secs(10));
            self.harness.run_steps(2);
        }
    }

    fn shot(&mut self, name: &str) {
        let path = self.out.join(format!("{}-{name}.png", self.size));
        self.harness.render().unwrap().save(&path).unwrap();
        println!("{}", path.display());
    }

    /// Send names a colored environment ("Send to Prod"), so it is found by its start.
    fn send(&mut self) {
        let label = self
            .harness
            .query_all_by_role(egui::accesskit::Role::Button)
            .filter_map(|node| node.accesskit_node().label())
            .find(|label| label == "Send" || label.starts_with("Send to "));
        match label {
            Some(label) => {
                self.click(&label);
            }
            None => eprintln!("tour ({}): no Send button", self.size),
        }
    }

    fn click(&mut self, label: &str) -> bool {
        let Some(node) = self.harness.query_by_label(label) else {
            eprintln!("tour ({}): no control named {label:?}", self.size);
            return false;
        };
        // Rows below the fold of a small window must be scrolled to before a click lands.
        // Scrolling animates; wait until it has settled.
        node.scroll_to_me();
        self.harness.run_steps(12);
        self.harness.query_by_label(label).unwrap().click();
        self.harness.run_steps(2);
        true
    }

    fn type_into(&mut self, label: &str, text: &str) {
        if self.click(label) {
            self.harness.query_by_label(label).unwrap().type_text(text);
            self.harness.run_steps(2);
        }
    }

    /// Open the request tab `label`, from "More" when it did not fit the row.
    fn open_tab(&mut self, label: &str) {
        self.open_from("More request tabs", label);
    }

    /// Click `label`, opening the menu `more` first when it is not on its row.
    fn open_from(&mut self, more: &str, label: &str) {
        if self
            .harness
            .query_by_role_and_label(egui::accesskit::Role::Button, label)
            .is_none()
        {
            self.click(more);
        }
        self.click(label);
    }

    fn keys(&mut self, modifiers: egui::Modifiers, key: egui::Key) {
        self.harness.key_press_modifiers(modifiers, key);
        self.harness.run_steps(2);
    }
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A realistic collection: folders, auth, bodies, checks, captures, environments, saved
/// values and mock routes, against the loopback server at `base`.
fn seed(dir: &Path, base: &str) {
    let store = open_store(dir).unwrap();
    let base = base.to_owned();
    store
        .call_blocking(move |db| {
            let api = collections::create_collection(
                db,
                "Shop API",
                "Customers, orders and reports",
                &vars(&[("version", "v2")]),
            )?;
            let users = collections::create_folder(db, api.id, None, "Users", "", 0)?;
            let orders = collections::create_folder(db, api.id, None, "Orders", "", 1)?;
            let request = |name: &str, method: &str, url: &str| RequestTemplate {
                name: name.into(),
                method: method.into(),
                url: url.into(),
                body_type: BodyType::None,
                ..RequestTemplate::default()
            };
            collections::create_request(
                db,
                api.id,
                Some(users.id),
                &RequestTemplate {
                    queries: vec![ParamItem::new("page", "1")],
                    headers: vec![ParamItem::new("Accept", "application/json")],
                    auth: RequestAuth::bearer("{{token}}"),
                    checks: vec![
                        Check::new(Selector::Status, Op::Equals, "2xx"),
                        Check::new(Selector::Json("$.total".into()), Op::Equals, "2"),
                        Check::new(Selector::Json("$.users[0].name".into()), Op::Equals, "Bo"),
                        Check::new(Selector::Duration, Op::Less, "500"),
                    ],
                    captures: vec![Capture::new(
                        "user_id",
                        Selector::Json("$.users[0].id".into()),
                    )],
                    ..request("List users", "GET", "{{base}}/users?page=1")
                },
            )?;
            collections::create_request(
                db,
                api.id,
                Some(users.id),
                &RequestTemplate {
                    body: br#"{
  "id": "{{$uuid}}",
  "name": "Ann",
  "email": "ann@example.com",
  "roles": ["admin"]
}"#
                    .to_vec(),
                    body_type: BodyType::Json,
                    headers: vec![ParamItem::new("Content-Type", "application/json")],
                    auth: RequestAuth::bearer("{{token}}"),
                    position: 1,
                    ..request("Create user", "POST", "{{base}}/users")
                },
            )?;
            collections::create_request(
                db,
                api.id,
                Some(orders.id),
                &request("Order {{user_id}}", "GET", "{{base}}/orders/{{missing_id}}"),
            )?;
            for (position, (name, method, url)) in [
                ("Health", "GET", "{{base}}/health"),
                ("Slow report", "GET", "{{base}}/slow"),
                ("Export everything", "GET", "{{base}}/big"),
                ("Broken endpoint", "DELETE", "{{base}}/fail"),
                ("Offline service", "GET", "http://127.0.0.1:9/ping"),
            ]
            .into_iter()
            .enumerate()
            {
                collections::create_request(
                    db,
                    api.id,
                    None,
                    &RequestTemplate {
                        position: i64::try_from(position).unwrap() + 2,
                        ..request(name, method, url)
                    },
                )?;
            }
            // Sorted after "Shop API", which therefore opens first.
            collections::create_collection(db, "Webhooks", "", &Variables::new())?;
            let dev = environments::create_environment(
                db,
                "Dev",
                &vars(&[("base", &base), ("token", "dev-token")]),
            )?;
            environments::set_environment_color(db, dev.id, Some(EnvColor::Green))?;
            let production = environments::create_environment(
                db,
                "Production",
                &vars(&[("base", "https://api.example.com"), ("token", "")]),
            )?;
            environments::set_environment_color(db, production.id, Some(EnvColor::Red))?;
            environments::set_active_environment(db, Some(dev.id))?;
            let mut saved = SavedValues::default();
            saved.set("user_id", ["42".into(), "77".into(), "91".into()]);
            scopes::write(
                db,
                Scope::Global,
                &vars(&[("user_id", "42"), ("locale", "en")]),
                &saved,
            )?;
            kv::set(
                db,
                kv::Table::Settings,
                "mock_server",
                &MockConfig {
                    port: 0,
                    routes: vec![
                        MockRoute {
                            path: "/users".into(),
                            body: r#"{"users":[{"id":1,"name":"Ann"}]}"#.into(),
                            headers: vec![ParamItem::new("Content-Type", "application/json")],
                            ..Default::default()
                        },
                        MockRoute {
                            method: "POST".into(),
                            path: "/orders".into(),
                            status: 201,
                            ..Default::default()
                        },
                    ],
                },
            )?;
            Ok(())
        })
        .unwrap();
    store.close().unwrap();
}

fn set_theme(dir: &Path, theme: ThemeChoice) {
    let store = open_store(dir).unwrap();
    store
        .call_blocking(move |db| {
            kv::set(
                db,
                kv::Table::UiState,
                "appearance",
                &serde_json::json!({ "theme": theme }),
            )
        })
        .unwrap();
    store.close().unwrap();
}

fn harness(dir: &Path, size: egui::Vec2) -> Harness<'static, ImposterApp> {
    let store = open_store(dir).unwrap();
    Harness::builder()
        .with_size(size)
        .with_max_steps(64)
        .wgpu()
        .build_eframe(move |cc| ImposterApp::new(&cc.egui_ctx, store).unwrap())
}

fn response(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    imposter_testkit::response(
        status,
        &[("Content-Type", content_type), ("X-Request-Id", "r-7f3a")],
        body,
    )
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let big: Vec<u8> = {
        let rows: Vec<String> = (0..12_000)
            .map(|i| {
                format!(
                    r#"{{"id":{i},"name":"Customer {i}","active":{}}}"#,
                    i % 3 == 0
                )
            })
            .collect();
        format!("[{}]", rows.join(",")).into_bytes()
    };
    let server = runtime.block_on(imposter_testkit::Server::start(move |_, received| {
        let path = received.request_line.split(' ').nth(1).unwrap_or_default();
        match path.split('?').next().unwrap_or_default() {
            "/users" if received.request_line.starts_with("POST") => response(
                "201 Created",
                "application/json",
                br#"{"id":"c1d2","name":"Ann"}"#,
            ),
            "/users" => response(
                "200 OK",
                "application/json",
                br#"{"users":[{"id":1,"name":"Ann","admin":true},{"id":2,"name":"Bo","admin":false}],"total":2,"next":null}"#,
            ),
            "/slow" => {
                std::thread::sleep(Duration::from_secs(2));
                response("200 OK", "text/plain", b"done")
            }
            "/big" => response("200 OK", "application/json", &big),
            "/fail" => response(
                "500 Internal Server Error",
                "application/problem+json",
                br#"{"type":"about:blank","title":"Database unavailable","status":500}"#,
            ),
            _ => response("200 OK", "text/plain", b"ok"),
        }
    }));

    for (theme, (size, window)) in [ThemeChoice::Dark, ThemeChoice::Light]
        .into_iter()
        .flat_map(|theme| {
            [
                ("narrow", egui::Vec2::from(imposter_ui::layout::MIN_WINDOW)),
                ("wide", egui::vec2(1500.0, 850.0)),
            ]
            .map(|size| (theme, size))
        })
    {
        let size = if theme == ThemeChoice::Dark {
            size.to_owned()
        } else {
            format!("{size}-light")
        };
        // Task 1: the very first start, then a first import.
        let empty = tempfile::tempdir().unwrap();
        set_theme(empty.path(), theme);
        let mut tour = Tour {
            harness: harness(empty.path(), window),
            out: out.clone(),
            size: size.clone(),
        };
        tour.settle();
        tour.shot("1-01-first-start");
        tour.click("New");
        tour.shot("1-02-new-menu");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        let petstore = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/openapi/petstore.yaml");
        tour.harness
            .state_mut()
            .script_file_dialogs([Some(petstore)]);
        // From the first-run pane, as a new user would.
        tour.click("Import Collection…");
        tour.settle();
        tour.shot("1-03-import-review");
        tour.click("Import");
        tour.settle();
        tour.shot("1-04-imported");

        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), &server.url);
        set_theme(dir.path(), theme);
        let mut tour = Tour {
            harness: harness(dir.path(), window),
            out: out.clone(),
            size,
        };
        tour.settle();

        // Task 2: edit, send and read.
        tour.shot("2-01-start");
        tour.click("POST Create user");
        tour.settle();
        tour.shot("2-02-json-body");
        tour.open_tab("Auth");
        tour.shot("2-03-auth");
        tour.open_tab("Request settings");
        tour.shot("2-03b-request-settings");
        tour.click("GET Slow report");
        tour.settle();
        tour.send();
        tour.harness.run_steps(3);
        tour.shot("2-04-in-progress");
        tour.settle();
        tour.click("GET List users");
        tour.settle();
        tour.send();
        tour.settle();
        tour.shot("2-05-json-response");
        // The overflow menus, where the window is too small for every tab and tool.
        for (menu, name) in [
            ("More request tabs", "2-05b-more-tabs"),
            ("More tools", "2-05c-more-tools"),
        ] {
            if tour.harness.query_by_label(menu).is_some() {
                tour.click(menu);
                tour.shot(name);
                tour.harness.key_press(egui::Key::Escape);
                tour.settle();
            }
        }
        tour.harness.state_mut().state.response.tab = ResponseTab::Headers;
        tour.settle();
        tour.shot("2-06-response-headers");
        tour.harness.state_mut().state.response.tab = ResponseTab::Body;
        tour.click("DELETE Broken endpoint");
        tour.settle();
        tour.send();
        tour.settle();
        tour.shot("2-07-server-error");
        tour.click("GET Offline service");
        tour.settle();
        tour.send();
        tour.settle();
        tour.shot("2-08-connection-failure");
        tour.click("GET Order {{user_id}}");
        tour.settle();
        tour.send();
        tour.settle();
        tour.shot("2-09-unresolved-variable");
        tour.click("GET Export everything");
        tour.settle();
        tour.send();
        tour.settle();
        tour.shot("2-10-large-response");

        // Task 3: environments and saved values.
        tour.click("GET List users");
        tour.settle();
        tour.click("Environment");
        tour.shot("3-01-environment-picker");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        // Click the {{base}} token at the start of the URL, as a user would.
        if let Some(field) = tour
            .harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Request URL")
        {
            let at = egui::pos2(field.rect().left() + 30.0, field.rect().center().y);
            tour.harness.drag_at(at);
            tour.harness.drop_at(at);
        }
        tour.harness.run_steps(3);
        tour.shot("3-02-inspector");
        tour.harness.state_mut().state.inspect = None;
        tour.click("Variables");
        tour.settle();
        tour.shot("3-03-variables");
        tour.click("Variable 1 saved values");
        tour.settle();
        tour.shot("3-04-saved-values");
        tour.harness.key_press(egui::Key::Escape);
        tour.click("Variable scope");
        tour.shot("3-05-scope-picker");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        tour.click("New environment");
        tour.settle();
        // The field has focus; its caption shares the name "Name".
        if let Some(field) = tour
            .harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Name")
        {
            field.type_text("Production EU");
            tour.harness.run_steps(2);
        }
        tour.shot("3-06-new-environment");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        tour.type_into("Variable 2 value", "-edited");
        tour.shot("3-07-unsaved-variables");
        tour.click("Revert unsaved changes");
        tour.settle();
        tour.click("Requests");
        tour.settle();
        tour.click("Environment");
        tour.click("Production");
        tour.settle();
        tour.shot("3-08-production-active");
        tour.click("Environment");
        tour.click("Dev");
        tour.settle();

        // Task 4: checks and captured variables.
        tour.click("Requests");
        tour.settle();
        tour.open_tab("Tests");
        tour.send();
        tour.settle();
        tour.open_from("More response tabs", "Check results");
        tour.settle();
        tour.shot("4-01-checks-and-results");
        tour.open_tab("Captures");
        tour.settle();
        tour.shot("4-01b-captures");
        // Scripts: a before-sending signature and after-response tests, then their results.
        {
            let scripts = &mut tour
                .harness
                .state_mut()
                .state
                .editor
                .as_mut()
                .expect("a request is open")
                .editor
                .scripts;
            scripts.before = "request.set_header(\"X-Signature\", hex(hmac_sha256(\"secret\", request.body)));\n".into();
            scripts.after = "print(`${response.status} in ${response.time_ms} ms`);\ntest(\"status is 2xx\", response.status < 300);\ntest(\"the first user has a name\", || response.path(\"$.users[0].name\") != ());\n".into();
        }
        tour.open_tab("Scripts");
        tour.settle();
        tour.shot("4-01c-scripts");
        tour.send();
        tour.settle();
        tour.harness.state_mut().state.response.tab = ResponseTab::Tests;
        tour.settle();
        tour.shot("4-01d-script-results");
        // A collection's scripts, which run around every request inside it.
        {
            let state = &mut tour.harness.state_mut().state;
            if let Some(id) = state.collection
                && let Some(collection) = state.collections.iter().find(|c| c.id == id)
            {
                state.dialog = Some(imposter_ui::dialogs::Dialog::Scripts {
                    target: imposter_ui::dialogs::ScriptTarget::Collection(id),
                    owner: format!("Collection \"{}\"", collection.name),
                    scripts: imposter_app::core::model::Scripts {
                        before: "request.set_header(\"X-Api-Key\", vars.get(\"api_key\"));\n"
                            .into(),
                        after: "test(\"no server error\", response.status < 500);\n".into(),
                    },
                });
            }
        }
        tour.settle();
        tour.shot("4-01e-collection-scripts");
        tour.harness.state_mut().state.dialog = None;
        tour.settle();
        tour.harness.state_mut().state.response.tab = ResponseTab::Body;
        tour.settle();
        if let Some(node) = tour.harness.query_by_label_contains("\"name\": \"Ann\"") {
            // Below the fold of a small pane: scroll to it first, as `click` does.
            node.scroll_to_me();
            tour.harness.run_steps(12);
        }
        // From the line's "⋯", which opens the menu under the line.
        if let Some(line) = tour
            .harness
            .query_by_label_contains("\"name\": \"Ann\"")
            .map(|node| node.rect())
        {
            tour.harness.hover_at(line.center());
            tour.harness.run_steps(2);
        }
        if let Some(node) = tour.harness.query_by_label_contains("Row actions") {
            node.click();
        }
        tour.harness.run_steps(2);
        tour.shot("4-02-row-menu");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();

        // Task 5: History.
        tour.click("History");
        tour.settle();
        tour.shot("5-01-history");
        tour.type_into("Filter history", "status:5xx OR status:failed");
        tour.settle();
        tour.shot("5-02-history-filtered");
        tour.harness.state_mut().state.history.filter.clear();
        tour.harness
            .state_mut()
            .state
            .actions
            .push(imposter_ui::app::Action::History(
                imposter_ui::app::HistoryAction::Filtered,
            ));
        tour.settle();
        if let Some(row) = tour
            .harness
            .query_all_by_label_contains("History GET")
            .next()
        {
            row.click();
        }
        tour.settle();
        tour.shot("5-03-history-request");
        tour.harness.state_mut().state.history.tab = HistoryTab::Response;
        tour.settle();
        tour.shot("5-04-history-response");
        tour.click("Restore as draft");
        tour.settle();
        tour.shot("5-05-restored-draft");
        tour.click("GET Health");
        tour.settle();
        tour.shot("5-06-unsaved-guard");
        tour.click("Discard");
        tour.settle();

        // Task 6: mocks.
        tour.open_from("Tools", "Mocks");
        tour.settle();
        tour.shot("6-01-mocks");
        tour.click("Start mock server");
        tour.settle();
        tour.shot("6-02-running");
        tour.click("Stop mock server");
        tour.settle();
        tour.click("Mock route 2");
        tour.settle();
        tour.type_into("Mock path", " bad path");
        tour.shot("6-03-invalid-route");
        // A scripted route and what the server answered.
        {
            let mocks = &mut tour.harness.state_mut().state.mocks;
            if let Some(route) = mocks.form.routes.get_mut(1) {
                route.path = "/orders".into();
                route.script = "state.calls = (state.calls ?? 0) + 1;\nif state.calls == 1 {\n    response.status = 503;\n}\nresponse.send_json(#{ calls: state.calls });\n".into();
            }
            for (status, error) in [
                (503, None),
                (200, None),
                (500, Some("Mock script, line 2: no stock")),
            ] {
                mocks.logged(imposter_app::core::mock::MockLogEntry {
                    method: "POST".into(),
                    path: "/orders".into(),
                    status,
                    elapsed: Duration::from_millis(4),
                    error: error.map(str::to_owned),
                    console: vec![format!("answered {status}")],
                });
            }
        }
        tour.settle();
        tour.shot("6-03b-mock-script");

        // Task 6b: a load test of the open request.
        tour.open_from("Tools", "Load");
        tour.settle();
        tour.shot("6-04-load");
        {
            let form = &mut tour.harness.state_mut().state.load.form;
            form.requests = "200".into();
            form.concurrency = "8".into();
        }
        tour.click("Start the load test");
        tour.harness.run_steps(3);
        if tour.harness.query_by_label("Start load test").is_some() {
            tour.shot("6-05-load-production");
            tour.click("Start load test");
        }
        tour.settle();
        tour.shot("6-06-load-result");

        // Task 7: getting around.
        tour.click("Requests");
        tour.settle();
        tour.type_into("Filter requests", "method:GET -health");
        tour.settle();
        tour.shot("7-01-sidebar-filter");
        tour.click("Clear the filter");
        tour.keys(egui::Modifiers::COMMAND, egui::Key::K);
        tour.settle();
        tour.shot("7-02-palette");
        if let Some(field) = tour.harness.query_by_label("Command palette search") {
            field.type_text("ord");
        }
        tour.settle();
        tour.shot("7-03-palette-search");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        tour.keys(egui::Modifiers::COMMAND, egui::Key::P);
        tour.settle();
        tour.shot("7-04-recent");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        tour.keys(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Slash,
        );
        tour.settle();
        tour.shot("7-05-shortcuts");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();
        tour.click("Collection");
        tour.shot("7-06-collection-picker");
        tour.harness.key_press(egui::Key::Escape);
        tour.settle();

        // Settings and layout.
        tour.click("Settings");
        tour.settle();
        tour.shot("8-01-settings");
        tour.click("Requests");
        tour.settle();
        tour.click("Stack request and response");
        tour.settle();
        tour.shot("8-02-stacked");
    }
}
