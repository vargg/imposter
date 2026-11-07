//! The layout lint: in every window size, theme and density, the main screens and dialogs
//! keep their controls inside the window's width and never overlap two buttons. Vertical
//! overflow is allowed (lists scroll); horizontal overflow is the bug that hides at wide sizes.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::Rect;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_testkit::{Server, response};
use imposter_ui::app::{Action, Appearance, Screen};
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::{Density, ThemeChoice};
use support::{click, click_button, collection, request, settle, start_with, type_into};

/// The status bar's own controls.
const STATUS_BAR: [&str; 8] = [
    "Hide Collections",
    "Show Collections",
    "Stack request and response",
    "Put request and response side by side",
    "Load running",
    "Load stopping…",
    "Mocks starting…",
    "Mocks stopping…",
];

const ROLES: [Role; 5] = [
    Role::Button,
    Role::CheckBox,
    Role::ComboBox,
    Role::TextInput,
    Role::MultilineTextInput,
];

/// Every operable control that shows in the window, by role and label. Controls that cross
/// the header's bottom or the status bar's top are partly scrolled out of their pane: egui
/// clips them, but AccessKit reports their whole rectangle, so they are left out.
fn controls(app: &support::App) -> Vec<(Role, String, Rect)> {
    let window = app.harness.ctx.content_rect();
    let header_bottom = app
        .harness
        .get_by_role_and_label(Role::ComboBox, "Environment")
        .rect()
        .bottom()
        + 8.0;
    let status_top = app
        .harness
        .query_by_label("Hide Collections")
        .or_else(|| app.harness.query_by_label("Show Collections"))
        .map_or(window.bottom(), |node| node.rect().top() - 6.0);
    let crosses = |rect: &Rect, edge: f32| rect.top() < edge && rect.bottom() > edge;
    ROLES
        .iter()
        .flat_map(|role| {
            app.harness.query_all_by_role(*role).map(move |node| {
                let label = node.accesskit_node().label().unwrap_or_default();
                (*role, label, node.rect())
            })
        })
        .filter(|(_, _, rect)| rect.intersects(window) && rect.area() > 0.0)
        .filter(|(_, _, rect)| !crosses(rect, header_bottom) && !crosses(rect, status_top))
        // Below the status bar's top only its own controls show; the rest is scrolled under.
        .filter(|(_, label, rect)| {
            rect.top() < status_top
                || STATUS_BAR.contains(&label.as_str())
                || label.starts_with("Mocks :")
        })
        .collect()
}

/// Visible text labels, with the same clipping rules as [`controls`].
fn labels(app: &support::App) -> Vec<(Role, String, Rect)> {
    let window = app.harness.ctx.content_rect();
    let header_bottom = app
        .harness
        .get_by_role_and_label(Role::ComboBox, "Environment")
        .rect()
        .bottom()
        + 8.0;
    let status_top = app
        .harness
        .query_by_label("Hide Collections")
        .or_else(|| app.harness.query_by_label("Show Collections"))
        .map_or(window.bottom(), |node| node.rect().top() - 6.0);
    app.harness
        .query_all_by_role(Role::Label)
        .map(|node| {
            let ak = node.accesskit_node();
            let text = ak.label().or_else(|| ak.value()).unwrap_or_default();
            (Role::Label, text, node.rect())
        })
        .filter(|(_, _, rect)| {
            rect.intersects(window)
                && rect.area() > 0.0
                && rect.top() >= header_bottom
                && rect.bottom() <= status_top
        })
        .collect()
}

/// What is wrong with the current frame, if anything. With a dialog or a popup open, only
/// widths are checked: it sits on a layer above what it overlaps.
fn problems(app: &support::App, state: &str) -> Vec<String> {
    let window = app.harness.ctx.content_rect();
    let controls = controls(app);
    let mut found = Vec::new();
    for (role, label, rect) in &controls {
        if rect.left() < window.left() - 0.5 || rect.right() > window.right() + 0.5 {
            found.push(format!(
                "{state}: {role:?} {label:?} runs past the window: {rect:?}"
            ));
        }
    }
    // Outlined frames (panes, groups, cards) must fit too; egui clips them silently.
    for clipped in &app.harness.output().shapes {
        if let egui::Shape::Rect(frame) = &clipped.shape
            && frame.stroke.width > 0.0
            && frame.stroke.color != egui::Color32::TRANSPARENT
            && (frame.rect.left() < window.left() - 0.5
                || frame.rect.right() > window.right() + 0.5)
        {
            found.push(format!(
                "{state}: a frame runs past the window: {:?}",
                frame.rect
            ));
        }
    }
    if app.harness.state().state.dialog.is_some() || egui::Popup::is_any_open(&app.harness.ctx) {
        return found;
    }
    // Inside a pane card a control stays within its sides: the pane clips what runs past,
    // which the window's edges cannot show (the Auth tab at 720 points).
    let panes: Vec<Rect> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(frame)
                if frame.stroke.color == imposter_ui::theme::palette().border
                    && frame.corner_radius == egui::CornerRadius::same(12) =>
            {
                Some(frame.rect)
            }
            _ => None,
        })
        .collect();
    for (role, label, rect) in &controls {
        if let Some(pane) = panes.iter().find(|pane| pane.contains(rect.center()))
            && (rect.left() < pane.left() - 0.5 || rect.right() > pane.right() + 0.5)
        {
            found.push(format!(
                "{state}: {role:?} {label:?} runs past its pane {pane:?}: {rect:?}"
            ));
        }
    }
    let buttons: Vec<&(Role, String, Rect)> = controls
        .iter()
        .filter(|(role, _, _)| *role == Role::Button)
        .collect();
    // Text drawn into a button: a status running into the menu after it.
    for (role, label, rect) in labels(app) {
        for (_, button, area) in &buttons {
            let overlap = rect.intersect(*area);
            if overlap.is_positive() && overlap.area() > 4.0 {
                found.push(format!(
                    "{state}: {role:?} {label:?} runs into button {button:?}"
                ));
            }
        }
    }
    for (i, (_, a, ra)) in buttons.iter().enumerate() {
        for (_, b, rb) in &buttons[i + 1..] {
            let overlap = ra.intersect(*rb);
            if overlap.is_positive() && overlap.area() > 4.0 {
                found.push(format!("{state}: buttons {a:?} and {b:?} overlap"));
            }
        }
    }
    found
}

fn appearance(app: &mut support::App, theme: ThemeChoice, density: Density) {
    app.harness
        .state_mut()
        .state
        .actions
        .push(Action::SetAppearance(Appearance { theme, density }));
    app.harness.run_steps(3);
}

/// Send names the active, colored environment of the walk.
const SEND: &str = "Send to Production EU";

/// Walk the screens and dialogs, collecting problems at each stop.
fn walk(app: &mut support::App, label: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut check = |app: &mut support::App, state: &str| {
        app.harness.run_steps(3);
        found.extend(problems(app, &format!("{label} {state}")));
    };
    check(app, "start");
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "response");
    // Both panes have a Headers tab.
    app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Headers;
    check(app, "response headers");
    app.harness.state_mut().state.response.headers_raw = true;
    check(app, "raw response headers");
    app.harness.state_mut().state.response.headers_raw = false;
    app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Body;
    // A long status beside the runs menu.
    click(&mut app.harness, "GET Broken");
    settle(&mut app.harness);
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "error response");
    // A binary body: described, with Save body… in the tools.
    click(&mut app.harness, "GET Logo");
    settle(&mut app.harness);
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "binary response");
    click(&mut app.harness, "GET Broken");
    settle(&mut app.harness);
    // The Tests tab: checks with and without a path, one without an expected value, and a
    // capture, in whatever layout the pane's width calls for.
    {
        use imposter_app::core::capture::Capture;
        use imposter_app::core::check::{Check, Op};
        use imposter_app::core::select::Selector;
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.checks = vec![
            Check::new(Selector::Status, Op::Equals, "200"),
            Check::new(
                Selector::Json("$.users[0].name".into()),
                Op::NotContains,
                "Bo",
            ),
            Check::new(Selector::Header("Content-Type".into()), Op::Exists, ""),
        ];
        pane.editor.captures = vec![Capture::new(
            "first_user",
            Selector::Json("$.users[0].id".into()),
        )];
        pane.tab = imposter_ui::app::EditorTab::Tests;
    }
    check(app, "tests tab");
    app.harness.state_mut().state.editor.as_mut().unwrap().tab =
        imposter_ui::app::EditorTab::Captures;
    check(app, "captures tab");
    // Scripts: two editors, one with a compile error under it; then a send whose
    // after-response script fails a test, logs and leaves its error in the Tests tab.
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.scripts.before = "request.set_header(\"X-Trace\", uuid());".into();
        pane.editor.scripts.after = "let broken = ;".into();
        pane.tab = imposter_ui::app::EditorTab::Scripts;
    }
    check(app, "scripts tab");
    // Collection scripts listed above the request's, and their dialog.
    {
        let state = &mut app.harness.state_mut().state;
        let id = state.collection.unwrap();
        let collection = state.collections.iter_mut().find(|c| c.id == id).unwrap();
        collection.scripts.before = "request.set_header(\"X-Collection\", \"1\");".into();
    }
    check(app, "scripts tab with collection scripts");
    {
        let state = &mut app.harness.state_mut().state;
        let id = state.collection.unwrap();
        let collection = state.collections.iter().find(|c| c.id == id).unwrap();
        state.dialog = Some(imposter_ui::dialogs::Dialog::Scripts {
            target: imposter_ui::dialogs::ScriptTarget::Collection(id),
            owner: format!("Collection \"{}\"", collection.name),
            scripts: imposter_app::core::model::Scripts {
                before: "request.set_header(\"X-Signature\", hex(hmac_sha256(vars.get(\"secret\"), request.body)));".into(),
                after: "let broken = ;".into(),
            },
        });
    }
    check(app, "collection scripts dialog");
    {
        let state = &mut app.harness.state_mut().state;
        state.dialog = None;
        let id = state.collection.unwrap();
        state
            .collections
            .iter_mut()
            .find(|c| c.id == id)
            .unwrap()
            .scripts = imposter_app::core::model::Scripts::default();
    }
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .scripts
        .after = "print(\"a long line of console output that has to wrap in a narrow pane\");\ntest(\"the status is 200 OK\", response.status == 200);\nresponse.nope();".into();
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Tests;
    check(app, "script results");
    app.harness.state_mut().state.response.tab = imposter_ui::app::ResponseTab::Body;
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .scripts = imposter_app::core::model::Scripts::default();
    // The request's settings: one row each, with a reset icon on an override.
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = imposter_ui::app::EditorTab::Settings;
        pane.editor.options.timeout.inherit = false;
        pane.editor.options.timeout.value = "5".into();
        pane.editor.options.verify_ssl.inherit = false;
    }
    check(app, "request settings tab");
    // Auth: a label column, fields to the pane's edge, an eye on secrets.
    for kind in [
        imposter_app::core::model::AuthKind::Basic,
        imposter_app::core::model::AuthKind::BearerToken,
    ] {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.auth.kind = Some(kind);
        pane.tab = imposter_ui::app::EditorTab::Auth;
        check(app, &format!("auth {kind:?}"));
    }
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = imposter_ui::app::EditorTab::Body;
    // A custom method being typed takes the method picker's place in the request bar.
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "Custom method…");
    type_into(&mut app.harness, "Custom HTTP method", " bad");
    check(app, "custom method");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .custom_method = None;
    app.harness.run_steps(2);
    for screen in ["Variables", "History", "Settings", "Mocks"] {
        support::open_screen(&mut app.harness, screen);
        settle(&mut app.harness);
        check(app, screen);
    }
    // Mocks with a scripted route and a request log holding an error and printed lines.
    support::open_screen(&mut app.harness, "Mocks");
    settle(&mut app.harness);
    {
        let mocks = &mut app.harness.state_mut().state.mocks;
        if mocks.form.routes.is_empty() {
            mocks
                .form
                .routes
                .push(imposter_app::core::mock::MockRoute::default());
        }
        mocks.form.routes[0].script =
            "state.calls = (state.calls ?? 0) + 1;\nresponse.send_json(#{ calls: state.calls });"
                .into();
        for status in [200_u16, 500, 503] {
            mocks.logged(imposter_app::core::mock::MockLogEntry {
                method: "POST".into(),
                path: "/orders/with/a/rather/long/path".into(),
                status,
                elapsed: std::time::Duration::from_millis(12),
                error: (status == 500).then(|| {
                    "Mock script, line 2: a long error message that has to wrap in the list".into()
                }),
                console: vec!["printed a line that is long enough to wrap".into()],
            });
        }
    }
    check(app, "Mocks with a script and a log");
    app.harness.state_mut().state.mocks.log.clear();
    // The Load screen on the open request: idle, asking about production, running, and a
    // stopped run's summary with its error pills and tables.
    support::open_screen(&mut app.harness, "Load");
    settle(&mut app.harness);
    check(app, "Load");
    {
        let form = &mut app.harness.state_mut().state.load.form;
        form.requests = "1000000".into();
        form.concurrency = "2".into();
    }
    click(&mut app.harness, "Start the load test");
    check(app, "Load production question");
    click_button(&mut app.harness, "Start load test");
    check(app, "Load running");
    click(&mut app.harness, "Stop the load test");
    settle(&mut app.harness);
    check(app, "Load result");
    if let Some(run) = app.harness.state_mut().state.load.runs.first_mut() {
        run.report.script_error = Some(
            "Before-sending script, line 12: a long error that has to wrap in a narrow window"
                .into(),
        );
    }
    check(app, "Load result with a script error");
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    if let Some(row) = app
        .harness
        .query_all_by_label_contains("History GET")
        .next()
    {
        row.click();
    }
    app.harness.run_steps(1);
    settle(&mut app.harness);
    app.harness.state_mut().state.history.tab = imposter_ui::app::HistoryTab::Response;
    check(app, "History response");
    // A restored draft: its banner above the request bar, the recorded response beside it.
    click(&mut app.harness, "Restore as draft");
    settle(&mut app.harness);
    check(app, "restored draft");
    click(&mut app.harness, "History");
    settle(&mut app.harness);
    // A result file from the command line, with its summary and origin lines.
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/results/sent.imposter-result.json");
    app.harness.state_mut().script_file_dialogs([Some(result)]);
    click(&mut app.harness, "History actions");
    click(&mut app.harness, "Open result file…");
    settle(&mut app.harness);
    check(app, "History result");
    click(&mut app.harness, "Close result");
    click(&mut app.harness, "Requests");
    settle(&mut app.harness);
    for (dialog, keys) in [("palette", egui::Key::K), ("recent", egui::Key::P)] {
        app.harness
            .key_press_modifiers(egui::Modifiers::COMMAND, keys);
        app.harness.run_steps(3);
        check(app, dialog);
        app.harness.key_press(egui::Key::Escape);
        settle(&mut app.harness);
    }
    app.harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Slash,
    );
    check(app, "shortcut sheet");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    click(&mut app.harness, "New");
    click(&mut app.harness, "New Folder");
    check(app, "name dialog");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    click(&mut app.harness, "Environment");
    check(app, "environment picker");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "New environment");
    check(app, "environment dialog");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    app.harness.state_mut().state.screen = Screen::Requests;
    found
}

#[test]
fn no_control_runs_past_the_window_or_overlaps_another() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, received| {
        if received.request_line.contains("/broken") {
            return response("500 Internal Server Error", &[], b"oops");
        }
        if received.request_line.contains("/logo") {
            return response("200 OK", &[("Content-Type", "image/png")], &[0x89, 0, 1, 2]);
        }
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            br#"{"users":[{"id":1,"name":"Ann"}],"total":1}"#,
        )
    }));
    let url = format!("{}/users", server.url);
    let mut problems = Vec::new();
    for size in [egui::Vec2::from(MIN_WINDOW), egui::vec2(1500.0, 850.0)] {
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            for density in [Density::Compact, Density::Comfortable] {
                let url = url.clone();
                let mut app = start_with(move |store| {
                    // Active and colored, so the picker is tinted and Send names it.
                    store
                        .call_blocking(|db| {
                            use imposter_app::store::environments as env;
                            let prod = env::create_environment(
                                db,
                                "Production EU",
                                &imposter_app::core::model::Variables::new(),
                            )?;
                            env::set_environment_color(
                                db,
                                prod.id,
                                Some(imposter_app::core::model::EnvColor::Red),
                            )?;
                            env::set_active_environment(db, Some(prod.id))
                        })
                        .unwrap();
                    let api = collection(store, "API");
                    request(store, api.id, "Users", &url);
                    request(store, api.id, "Health", "http://127.0.0.1:9/");
                    request(store, api.id, "Broken", &format!("{url}/broken"));
                    request(store, api.id, "Logo", &format!("{url}/logo"));
                });
                app.harness.set_size(size);
                appearance(&mut app, theme, density);
                let label = format!("{}x{} {theme:?} {density:?}", size.x, size.y);
                problems.extend(walk(&mut app, &label));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
