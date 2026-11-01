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
use support::{click, collection, request, settle, start_with, type_into};

/// The status bar's own controls.
const STATUS_BAR: [&str; 4] = [
    "Hide Collections",
    "Show Collections",
    "Stack request and response",
    "Put request and response side by side",
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
        .filter(|(_, label, rect)| rect.top() < status_top || STATUS_BAR.contains(&label.as_str()))
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
    click(&mut app.harness, "Send");
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
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    check(app, "error response");
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
        click(&mut app.harness, screen);
        settle(&mut app.harness);
        check(app, screen);
    }
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
                    // Active and colored, so the request bar and status bar show it.
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
