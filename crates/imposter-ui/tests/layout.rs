//! The layout lint: in every window size, theme and density, the main screens and dialogs
//! keep their controls inside the window's width and never overlap two buttons. Vertical
//! overflow is allowed (lists scroll); horizontal overflow is the bug that hides at wide sizes.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::Rect;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{BodyType, FormItem, ParamItem};
use imposter_testkit::{Server, response};
use imposter_ui::app::{Action, Appearance, EditorTab, Screen};
use imposter_ui::editor::OPTION_GROUPS;
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::{self, Density, ThemeChoice};
use support::{click, click_button, collection, request, settle, start_with, type_into};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::watch;

/// The status bar's own controls.
const STATUS_BAR: [&str; 6] = [
    "Load running",
    "Load stopping…",
    "Run running",
    "Run stopping…",
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

/// Preserve each control's full width so clipping can't mask horizontal overflow.
fn visible_vertical_bounds(node: &egui_kittest::Node<'_>) -> Rect {
    let mut rect = node.rect();
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.accesskit_node().role() == Role::ScrollView {
            let clip = parent.rect();
            rect.min.y = rect.min.y.max(clip.min.y);
            rect.max.y = rect.max.y.min(clip.max.y);
        }
        ancestor = parent.parent();
    }
    rect
}

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
    let status_top = support::status_bar(app).top() - 5.0;
    let crosses = |rect: &Rect, edge: f32| rect.top() < edge && rect.bottom() > edge;
    ROLES
        .iter()
        .flat_map(|role| {
            app.harness.query_all_by_role(*role).map(move |node| {
                let label = node.accesskit_node().label().unwrap_or_default();
                (*role, label, visible_vertical_bounds(&node))
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
    let status_top = support::status_bar(app).top() - 5.0;
    app.harness
        .query_all_by_role(Role::Label)
        .map(|node| {
            let ak = node.accesskit_node();
            let text = ak.label().or_else(|| ak.value()).unwrap_or_default();
            (Role::Label, text, visible_vertical_bounds(&node))
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
    if app.harness.state().state.dialog.is_some() && !egui::Popup::is_any_open(&app.harness.ctx) {
        let palette = theme::palette();
        let modal = app
            .harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(frame)
                    if frame.fill == palette.surface
                        && frame.stroke.color == palette.field_border
                        && frame.corner_radius.nw == 8 =>
                {
                    Some(frame.rect)
                }
                _ => None,
            })
            .expect("painted modal frame");
        if !window.expand(0.5).contains_rect(modal) {
            found.push(format!("{state}: modal runs past the window: {modal:?}"));
        }
        let boundary = modal.expand(0.5);
        for node in app.harness.get_by_role(Role::Dialog).children_recursive() {
            let widget = node.accesskit_node();
            if !ROLES.contains(&widget.role()) && widget.role() != Role::DisclosureTriangle {
                continue;
            }
            let rect = node.rect();
            if rect.width() > 0.0
                && (rect.left() < boundary.left() || rect.right() > boundary.right())
            {
                found.push(format!(
                    "{state}: {:?} {:?} runs past the modal {modal:?}: {rect:?}",
                    widget.role(),
                    widget.label()
                ));
            }
        }
        return found;
    }
    if egui::Popup::is_any_open(&app.harness.ctx) {
        return found;
    }
    let toolbar = app.harness.get_by_role(Role::Toolbar);
    let row = toolbar.rect().expand(0.5);
    for node in toolbar.children_recursive() {
        let ak = node.accesskit_node();
        if (ROLES.contains(&ak.role()) || ak.role() == Role::Label)
            && !row.contains_rect(node.rect())
        {
            found.push(format!(
                "{state}: {:?} {:?} runs outside the toolbar {row:?}: {:?}",
                ak.role(),
                ak.label().or_else(|| ak.value()),
                node.rect(),
            ));
        }
    }
    // Ask which scope owns each control so a clipped control can't be attributed to
    // the pane next door.
    for name in ["Request pane", "Response pane"] {
        if let Some(pane) = app.harness.query_by_role_and_label(Role::Group, name) {
            if name == "Request pane"
                && app
                    .harness
                    .state()
                    .state
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.tab == EditorTab::Settings)
            {
                found.extend(request_settings_labels(app, &pane, state));
            }
            let boundary = pane.rect();
            for node in pane.children_recursive() {
                let ak = node.accesskit_node();
                if !ROLES.contains(&ak.role()) {
                    continue;
                }
                let rect = node.rect();
                if rect.intersects(boundary)
                    && (rect.left() < boundary.left() - 0.5
                        || rect.right() > boundary.right() + 0.5)
                {
                    found.push(format!(
                        "{state}: {:?} {:?} runs past {name} {boundary:?}: {rect:?}",
                        ak.role(),
                        ak.label(),
                    ));
                }
            }
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

fn request_settings_labels(
    app: &support::App,
    pane: &egui_kittest::Node<'_>,
    state: &str,
) -> Vec<String> {
    let mut found = Vec::new();
    for field in OPTION_GROUPS.iter().flat_map(|(_, fields)| fields.iter()) {
        let label = field.label();
        let width = app.harness.ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    label.to_owned(),
                    egui::FontId::proportional(theme::BODY),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x
        });
        let caption = pane.get_by_role_and_label(Role::Label, label);
        if caption.rect().width() + 0.5 < width
            || caption.rect().right() > pane.rect().right() + 0.5
        {
            found.push(format!(
                "{state}: settings label {label:?} needs {width:.1} pt: {:?}",
                caption.rect()
            ));
        }
    }
    found
}

fn appearance(app: &mut support::App, theme: ThemeChoice, density: Density) {
    app.harness
        .state_mut()
        .state
        .actions
        .push(Action::SetAppearance(Appearance {
            theme,
            density,
            ..Appearance::default()
        }));
    app.harness.run_steps(3);
}

fn assert_tool_panes(app: &support::App, setting: Rect, result: Rect) {
    if app.harness.ctx.content_rect().width() <= MIN_WINDOW[0] {
        assert!(
            result.top() >= setting.bottom(),
            "stacked: {setting:?}, {result:?}"
        );
    } else {
        assert!(
            result.left() >= setting.right(),
            "side by side: {setting:?}, {result:?}"
        );
    }
}

const SEND: &str = "Send";

const LONG_ENVIRONMENT: &str =
    "A very long environment name for sample requests with a scope that has to truncate";

/// Walk the screens and dialogs, collecting problems at each stop.
fn walk(
    app: &mut support::App,
    label: &str,
    stream_url: &str,
    send_event: &watch::Sender<bool>,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut check = |app: &mut support::App, state: &str| {
        app.harness.run_steps(3);
        found.extend(problems(app, &format!("{label} {state}")));
    };
    check(app, "start");
    click_button(&mut app.harness, "Toggle folder Examples");
    check(app, "closed request tree folder");
    type_into(&mut app.harness, "Filter requests", "method:DELETE");
    check(
        app,
        "filtered tree with a reopened folder and a nested request",
    );
    app.harness.get_by_label("DELETE Remove user");
    click_button(&mut app.harness, "Clear the filter");
    click_button(&mut app.harness, "Toggle folder Examples");
    check(app, "expanded request tree");
    click(&mut app.harness, "Hide Collections");
    check(app, "no request or sidebar");
    click(&mut app.harness, "Show Collections");
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    check(app, "empty response");
    walk_send_controls(app, stream_url, send_event, &mut check);
    click(&mut app.harness, "More request tabs");
    check(app, "request More menu");
    click(&mut app.harness, "Docs");
    check(app, "selected secondary request tab");
    support::open_tab(&mut app.harness, "Body");
    let (original_type, original_text) = {
        let editor = &mut app
            .harness
            .state_mut()
            .state
            .editor
            .as_mut()
            .unwrap()
            .editor;
        (
            std::mem::replace(&mut editor.body_type, BodyType::Json),
            std::mem::replace(&mut editor.body_text, r#"{"id":1}"#.into()),
        )
    };
    check(app, "valid JSON body header");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_text = "{\n  \"id\": }".into();
    check(app, "invalid JSON body header");
    if app
        .harness
        .query_by_role_and_label(Role::ComboBox, "Body type")
        .is_some()
    {
        click(&mut app.harness, "Body type");
        check(app, "body type menu");
        click(&mut app.harness, "JSON");
    }
    click(&mut app.harness, "Stack request and response");
    check(app, "stacked JSON body header");
    click(&mut app.harness, "Put request and response side by side");
    {
        let editor = &mut app
            .harness
            .state_mut()
            .state
            .editor
            .as_mut()
            .unwrap()
            .editor;
        editor.body_type = BodyType::Binary;
    }
    check(app, "binary body header and file picker");
    {
        let editor = &mut app
            .harness
            .state_mut()
            .state
            .editor
            .as_mut()
            .unwrap()
            .editor;
        editor.body_type = original_type;
        editor.body_text = original_text;
    }
    click(&mut app.harness, "Hide Collections");
    check(app, "empty response without sidebar");
    click(&mut app.harness, "Show Collections");
    click(&mut app.harness, "Stack request and response");
    check(app, "stacked empty response");
    click(&mut app.harness, "Put request and response side by side");
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "response");
    click(&mut app.harness, "More tools");
    check(app, "response tools menu");
    click(&mut app.harness, "Filter with JSONPath");
    check(app, "response filter open");
    support::viewer_tool(&mut app.harness, "Filter with JSONPath");
    // Table composers and the inline confirmation need to fit the narrow request pane.
    for (tab, title) in [
        (EditorTab::Params, "query composer"),
        (EditorTab::Cookies, "request cookies composer"),
        (EditorTab::Headers, "request headers composer"),
    ] {
        app.harness.state_mut().state.editor.as_mut().unwrap().tab = tab;
        check(app, title);
    }
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .headers = vec![ParamItem::new("Accept", "application/json")];
    check(app, "request headers with clear button");
    click(&mut app.harness, "Clear all");
    check(app, "clear table confirmation");
    click(&mut app.harness, "Keep");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .headers
        .clear();
    let original = {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = EditorTab::Body;
        std::mem::replace(&mut pane.editor.body_type, BodyType::Urlencoded)
    };
    check(app, "form field composer");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Multipart;
    check(app, "multipart field composer");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .multipart = vec![FormItem::file("upload", "report.txt")];
    check(app, "multipart metadata and header actions");
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.multipart.clear();
        pane.editor.body_type = original;
    }
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
    click(&mut app.harness, "GET Unresolved");
    settle(&mut app.harness);
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "blocked send with variable actions");
    click(&mut app.harness, "GET Users");
    settle(&mut app.harness);
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    click(&mut app.harness, "Environment · Production EU");
    settle(&mut app.harness);
    type_into(&mut app.harness, "New variable name", "first_user");
    support::open_screen(&mut app.harness, "Requests");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .captures = vec![imposter_app::core::capture::Capture::new(
        "first_user",
        imposter_app::core::select::Selector::Json("$.users[0].id".into()),
    )];
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "response with a capture warning badge");
    app.harness.get_by_label("1 not set").hover();
    check(app, "capture warning tooltip");
    click(&mut app.harness, "1 not set");
    click(&mut app.harness, "Revert unsaved changes");
    click(&mut app.harness, "Discard changes");
    settle(&mut app.harness);
    support::open_screen(&mut app.harness, "Requests");
    // A binary body: described, with Save body… in the tools.
    click(&mut app.harness, "GET Logo");
    settle(&mut app.harness);
    click(&mut app.harness, SEND);
    settle(&mut app.harness);
    check(app, "binary response");
    // An event stream on its Events tab, one event chosen.
    {
        use imposter_app::core::sse::{Event, Stream, StreamEnd};
        let event = |n: u64| Event {
            at_ms: n * 1250,
            event: "a-rather-long-event-name".into(),
            id: n.to_string(),
            data: format!(
                "{{\"price\": {n}, \"note\": \"a long line of data that runs past the pane\"}}"
            ),
        };
        let events: Vec<Event> = (1..40).map(event).collect();

        let response = &mut app.harness.state_mut().state.response;
        response.show(
            imposter_app::core::execution::Response {
                status: 200,
                reason: "OK".into(),
                headers: vec![("content-type".into(), "text/event-stream".into())],
                stream: Some(Stream {
                    end: StreamEnd::Cut,
                    events,
                    more_events: 3,
                }),
                ..Default::default()
            },
            imposter_ui::viewer::PreviewLimit::default(),
        );
        response.event_selected = Some(1);
    }
    check(app, "event stream");
    click(&mut app.harness, "GET Broken");
    settle(&mut app.harness);
    for (tab, title) in [
        (EditorTab::Tests, "empty tests tab"),
        (EditorTab::Captures, "empty captures tab"),
    ] {
        app.harness.state_mut().state.editor.as_mut().unwrap().tab = tab;
        check(app, title);
    }
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
        pane.editor.captures = vec![
            Capture::new("first_user", Selector::Json("$.users[0].id".into())),
            // Stored as a secret: its lock is on and its target names the secrets.
            Capture {
                secret: true,
                ..Capture::new("session_token", Selector::Header("Set-Cookie".into()))
            },
        ];
        pane.tab = EditorTab::Tests;
    }
    check(app, "tests tab");
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Captures;
    check(app, "captures tab");
    // Scripts: two editors, one with a compile error in its header; then a send whose
    // after-response script fails a test, logs and leaves its error in the Tests tab.
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.scripts.before = "request.set_header(\"X-Trace\", uuid());".into();
        pane.editor.scripts.after = "let broken = ;".into();
        pane.tab = EditorTab::Scripts;
    }
    check(app, "scripts tab");
    click(
        &mut app.harness,
        "Insert an example into the Before-sending script",
    );
    check(app, "script examples menu");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    click(&mut app.harness, "Stack request and response");
    check(app, "scripts tab in stacked panes");
    click(&mut app.harness, "Put request and response side by side");
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
        pane.tab = EditorTab::Settings;
        pane.editor.options.timeout.inherit = false;
        pane.editor.options.timeout.value = "5".into();
        pane.editor.options.verify_ssl.inherit = false;
    }
    check(app, "request settings tab");
    click(&mut app.harness, "Stack request and response");
    check(app, "request settings tab with stacked panes");
    click(&mut app.harness, "Put request and response side by side");
    // The banners over a request whose file changed, or went, on disk.
    let open = app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .and_then(|pane| pane.editor.id());
    if let Some(id) = open {
        for deleted in [false, true] {
            app.harness.state_mut().state.disk_change = Some(imposter_ui::app::DiskChange {
                id,
                name: "A request with a long name that has to be cut in a narrow window".into(),
                deleted,
            });
            check(
                app,
                if deleted {
                    "deleted on disk banner"
                } else {
                    "changed on disk banner"
                },
            );
        }
        app.harness.state_mut().state.disk_change = None;
    }
    // Auth: a label column, fields to the pane's edge, an eye on secrets.
    for kind in [
        imposter_app::core::model::AuthKind::Basic,
        imposter_app::core::model::AuthKind::BearerToken,
        imposter_app::core::model::AuthKind::ApiKey,
        imposter_app::core::model::AuthKind::OAuth2,
    ] {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.auth.kind = Some(kind);
        // The longest place, so its picker is measured at its widest.
        pane.editor.auth.key_in = imposter_app::core::model::ApiKeyIn::Query;
        pane.tab = EditorTab::Auth;
        check(app, &format!("auth {kind:?}"));
    }
    {
        // The authorization code grant's extra rows and its sign-in buttons.
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.auth.oauth.grant = imposter_app::core::model::OAuthGrant::AuthorizationCode;
        pane.editor.auth.oauth.redirect_port = Some(53682);
        check(app, "auth OAuth2 authorization code");
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.auth.oauth = imposter_app::core::model::OAuth2::default();
    }
    {
        // The screens after this send the request: a key without a name could not go.
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.auth.kind = None;
        pane.tab = EditorTab::Body;
    }
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
        if screen == "Settings" {
            for category in [
                "Appearance",
                "Collections",
                "General",
                "Network",
                "TLS",
                "History",
                "Command line",
            ] {
                support::open_settings_category(&mut app.harness, category);
                check(app, &format!("{category} settings"));
                if category == "Collections" {
                    app.harness.state_mut().state.folder_error = Some(
                        "Could not read the collections folder. Check the file named in the error before reopening the project.".into()
                    );
                    check(app, "collections folder read error");
                    app.harness.state_mut().state.folder_error = None;
                }
                if category == "TLS" {
                    app.harness
                        .get_by_label("Add host certificate")
                        .scroll_to_me();
                    check(app, "host certificates header");
                    click(&mut app.harness, "Add host certificate");
                    check(app, "host certificate block");
                    support::text_input(&app.harness, "Host certificate 1 Key").scroll_to_me();
                    check(app, "host certificate fields");
                    app.harness
                        .state_mut()
                        .state
                        .settings_screen
                        .form
                        .host_identities
                        .clear();
                }
            }
        }
    }
    let routes = std::mem::take(&mut app.harness.state_mut().state.mocks.form.routes);
    check(app, "empty mock routes");
    app.harness.state_mut().state.mocks.form.routes = routes;
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
        mocks.form.routes[0].body = "{\"calls\": 1}".into();
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
    app.harness.state_mut().state.mocks.form.routes[0].script = "let bad = ;".into();
    app.harness
        .get_by_label("Response script (optional)")
        .scroll_to_me();
    app.harness.run_steps(12);
    check(app, "mock script compile error");
    assert!(
        app.harness
            .ctx
            .content_rect()
            .contains_rect(app.harness.get_by_label_contains("Line 1, column").rect())
    );
    app.harness.state_mut().state.mocks.log.clear();
    // Revert the script edit before starting, then dirty the running configuration.
    click(&mut app.harness, "Revert to saved mocks");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    click(&mut app.harness, "Start mock server");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .harness
        .state()
        .state
        .mocks
        .run
        .as_ref()
        .is_none_or(|run| run.address.is_none())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "mock server did not start"
        );
        app.harness.run_steps(2);
        std::thread::yield_now();
    }
    app.harness.state_mut().state.mocks.form.routes[0].body = "unsaved".into();
    check(app, "Mocks running with unsaved changes");
    support::open_screen(&mut app.harness, "Requests");
    check(app, "Mock server in global status");
    support::open_screen(&mut app.harness, "Mocks");
    click(&mut app.harness, "Stop mock server");
    settle(&mut app.harness);
    // The Load screen on the open request: idle, asking about production, running, and a
    // stopped run's summary with its error pills and tables.
    support::open_screen(&mut app.harness, "Load");
    settle(&mut app.harness);
    check(app, "Load");
    let load_target = app.harness.state().state.load.target.clone();
    {
        let target = app.harness.state_mut().state.load.target.as_mut().unwrap();
        target.name = "A request with a very long name ".repeat(10);
        target.method = "CUSTOM-".repeat(40);
    }
    check(app, "Load with a long request name and method");
    app.harness.state_mut().state.load.target = load_target;
    {
        let form = &mut app.harness.state_mut().state.load.form;
        form.requests = "1000000".into();
        form.concurrency = "2".into();
    }
    click(&mut app.harness, "Start the load test");
    check(app, "Load production question");
    click_button(&mut app.harness, "Start load test");
    check(app, "Load running");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .harness
        .state()
        .state
        .load
        .run
        .as_ref()
        .unwrap()
        .progress
        .done()
        == 0
    {
        assert!(
            std::time::Instant::now() < deadline,
            "load test did not finish a request"
        );
        app.harness.run_steps(2);
        std::thread::yield_now();
    }
    click(&mut app.harness, "Stop the load test");
    settle(&mut app.harness);
    check(app, "Load result");
    assert_tool_panes(
        app,
        support::text_input(&app.harness, "Total requests").rect(),
        app.harness.get_by_label("Result").rect(),
    );
    app.harness.get_by_label("Result").scroll_to_me();
    app.harness.run_steps(12);
    app.harness.get_by_label("Served within");
    check(app, "Load metrics");
    if let Some(run) = app.harness.state_mut().state.load.runs.first_mut() {
        run.report.script_error = Some(
            "Before-sending script, line 12: a long error that has to wrap in a narrow window"
                .into(),
        );
    }
    check(app, "Load result with a script error");
    // The Run screen on the collection: idle, asking about production, running, and a
    // finished run (no checks, a refused connection, the rest skipped) with a row open.
    support::open_screen(&mut app.harness, "Run");
    settle(&mut app.harness);
    check(app, "Run");
    assert_tool_panes(
        app,
        app.harness.get_by_label("Environment for the run").rect(),
        app.harness.get_by_label("No run yet").rect(),
    );
    let run_target = app.harness.state().state.run.target.clone();
    let environment_name = {
        let state = &mut app.harness.state_mut().state;
        state.run.target.as_mut().unwrap().label = "A collection with a very long name ".repeat(10);
        std::mem::replace(
            &mut state.environments[0].name,
            "A very long environment name ".repeat(10),
        )
    };
    check(app, "Run with long target and environment names");
    app.harness.state_mut().state.run.target = run_target;
    app.harness.state_mut().state.environments[0].name = environment_name;
    click(&mut app.harness, "Start the run");
    check(app, "Run production question");
    click_button(&mut app.harness, "Start run");
    check(app, "Run running");
    settle(&mut app.harness);
    check(app, "Run result");
    if let Some(row) = app.harness.query_all_by_label_contains("Run step ").next() {
        row.click();
    }
    settle(&mut app.harness);
    check(app, "Run result with a response");
    support::open_screen(&mut app.harness, "History");
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
    support::viewer_tool(&mut app.harness, "Wrap");
    check(app, "History response with Wrap off");
    let history_tabs = [
        "Headers",
        "Check results",
        "Request body",
        "Request headers",
        "Body",
    ];
    for tab in history_tabs {
        if app
            .harness
            .query_by_role_and_label(Role::Button, tab)
            .is_none()
        {
            click(&mut app.harness, "More History tabs");
        }
        click_button(&mut app.harness, tab);
        check(app, &format!("History {tab} tab"));
        let y = app
            .harness
            .get_by_role_and_label(Role::Button, tab)
            .rect()
            .center()
            .y;
        for caption in history_tabs
            .into_iter()
            .chain(["More History tabs", "More tools"])
        {
            if let Some(button) = app.harness.query_by_role_and_label(Role::Button, caption) {
                assert!(
                    (button.rect().center().y - y).abs() <= 1.0,
                    "{tab} shares a row with {caption}"
                );
            }
        }
    }
    // A restored draft: its banner above the request bar, the recorded response beside it.
    click(&mut app.harness, "Restore as draft");
    settle(&mut app.harness);
    check(app, "restored draft");
    support::open_screen(&mut app.harness, "History");
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
    support::open_screen(&mut app.harness, "Requests");
    settle(&mut app.harness);
    for (dialog, modifiers) in [
        ("palette", egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
        ("recent", egui::Modifiers::COMMAND),
    ] {
        app.harness.key_press_modifiers(modifiers, egui::Key::P);
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
    walk_variable_scopes(app, &mut check);
    app.harness.state_mut().state.screen = Screen::Requests;
    walk_modals(app, &mut check);
    found
}

fn walk_send_controls(
    app: &mut support::App,
    stream_url: &str,
    send_event: &watch::Sender<bool>,
    check: &mut impl FnMut(&mut support::App, &str),
) {
    let send = app.harness.get_by_role_and_label(Role::Button, SEND).rect();
    let state = &mut app.harness.state_mut().state;
    let active = state.active_environment.unwrap();
    let environment = state
        .environments
        .iter_mut()
        .find(|env| env.id == active)
        .unwrap();
    let original_name = std::mem::replace(&mut environment.name, LONG_ENVIRONMENT.into());
    check(app, "request bar with a long active environment name");
    assert_eq!(
        app.harness.get_by_role_and_label(Role::Button, SEND).rect(),
        send
    );
    let state = &mut app.harness.state_mut().state;
    state
        .environments
        .iter_mut()
        .find(|env| env.id == active)
        .unwrap()
        .name = original_name;
    let editor = &mut state.editor.as_mut().unwrap().editor;
    let original_url = std::mem::replace(&mut editor.url, stream_url.into());
    let original_response = std::mem::take(&mut state.response);
    app.harness.run_steps(2);
    click(&mut app.harness, SEND);
    app.harness.get_by_role_and_label(Role::Button, "Cancel");
    check(app, "Cancel while waiting for a response");

    send_event.send_replace(true);
    // Do not wait_idle here: the stream keeps a background read running until Stop.
    // llvm-cov and busy CI runners need more than a few seconds for the first event.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app
        .harness
        .query_by_role_and_label(Role::Button, "Stop")
        .is_none()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "no stream event arrived"
        );
        app.harness.run_steps(2);
        std::thread::yield_now();
    }
    check(app, "Stop while receiving events");
    click_button(&mut app.harness, "Stop");
    settle(&mut app.harness);
    send_event.send_replace(false);
    let state = &mut app.harness.state_mut().state;
    state.editor.as_mut().unwrap().editor.url = original_url;
    state.response = original_response;
    app.harness.run_steps(2);
}

fn walk_variable_scopes(app: &mut support::App, check: &mut impl FnMut(&mut support::App, &str)) {
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
    check(app, "Global variables");
    app.harness.state_mut().state.variables.loaded = false;
    check(app, "Variables loading");
    app.harness.state_mut().state.variables.loaded = true;
    click(&mut app.harness, "Collection · API");
    settle(&mut app.harness);
    check(app, "Collection variables");
    click(
        &mut app.harness,
        &format!("Environment · {LONG_ENVIRONMENT}"),
    );
    settle(&mut app.harness);
    check(app, "Variables with a long inactive environment name");
    app.harness
        .get_by_label(&format!("Environment · {LONG_ENVIRONMENT}"))
        .hover();
    check(app, "Variables with a visible environment menu button");
    support::open_environment_menu(&mut app.harness, LONG_ENVIRONMENT);
    check(app, "environment actions");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    click(&mut app.harness, "Global");
    settle(&mut app.harness);
    let long_value = "a-saved-value-".repeat(8);
    let rows = std::mem::replace(
        &mut app.harness.state_mut().state.variables.rows,
        vec![
            imposter_ui::vars::VariableRow::from_stored(
                "user_id",
                Some("42".into()),
                vec!["77".into(), long_value, "91".into()],
                Some(imposter_app::store::scopes::CaptureSlot {
                    value: "from response".into(),
                    active: false,
                }),
            ),
            imposter_ui::vars::VariableRow::from_stored("locale", Some("en".into()), vec![], None),
        ],
    );
    check(app, "variable chips");
    let chip = "Variable 1 latest capture: from response";
    app.harness.get_by_label(chip).click_secondary();
    check(app, "variable chip menu");
    app.harness.get_by_label("Remove from list");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    app.harness.state_mut().state.variables.rows = rows;
    // A long name and a masked value on Secrets.
    app.harness.state_mut().state.variables.secrets = vec![
        imposter_ui::vars::Row::new("api_key", "k-123456"),
        imposter_ui::vars::Row::new("a_rather_long_secret_name_for_the_column", "v-123456"),
    ];
    click(&mut app.harness, "Secrets");
    check(app, "Secrets tab");
    // A long name, a domain with subdomains and an expiry on Cookies.
    app.harness.state_mut().state.variables.cookies =
        vec![imposter_app::http::cookies::StoredCookie {
            domain: "api.a-rather-long-domain-name.example.com".into(),
            host_only: false,
            path: "/v1/accounts/sessions".into(),
            name: "a_rather_long_session_cookie_name".into(),
            value: "abcdef-123456".into(),
            expires: Some(4_102_444_800),
            secure: true,
            http_only: true,
        }];
    click(&mut app.harness, "Cookies");
    check(app, "Cookies tab with a cookie");
    app.harness.state_mut().state.variables.cookies.clear();
    check(app, "Cookies tab empty");
    click(&mut app.harness, "Variables tab");
    app.harness.state_mut().state.variables_changed_on_disk = true;
    check(app, "variables changed on disk banner");
    app.harness.state_mut().state.variables_changed_on_disk = false;
    app.harness.state_mut().state.variables.secrets.clear();
    // The Global page's menu, with its Share values switch.
    click(&mut app.harness, "Variable actions");
    check(app, "global variable actions");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
    click(&mut app.harness, "New environment");
    check(app, "environment dialog");
    app.harness.key_press(egui::Key::Escape);
    settle(&mut app.harness);
}

fn walk_modals(app: &mut support::App, check: &mut impl FnMut(&mut support::App, &str)) {
    use imposter_app::core::ids::RequestId;
    use imposter_ui::dialogs::Dialog;
    use imposter_ui::tree::NodeId;

    for (name, dialog) in [
        (
            "curl paste",
            Dialog::Curl {
                text: "curl https://example.com".into(),
            },
        ),
        ("script reference", Dialog::ScriptReference),
        ("delete confirmation", Dialog::clear_history(Some(42))),
        (
            "message",
            Dialog::error("Import failed", "Could not open the collection"),
        ),
        (
            "move dialog",
            Dialog::Move {
                node: NodeId::Request(RequestId(1)),
                options: vec![("A long folder path / ".repeat(8), None)],
                choice: 0,
            },
        ),
        ("application settings guard", Dialog::SettingsGuard),
        ("variables guard", Dialog::VariablesGuard),
        ("mocks guard", Dialog::MocksGuard),
        (
            "draft guard",
            Dialog::Guard {
                request: "Users".into(),
                draft: Some("API".into()),
                failed: None,
            },
        ),
        (
            "failed save guard",
            Dialog::Guard {
                request: "Users".into(),
                draft: None,
                failed: Some("Could not save the request".into()),
            },
        ),
        (
            "companion files",
            Dialog::Companions {
                path: std::path::PathBuf::new(),
            },
        ),
        (
            "export options",
            Dialog::Export {
                environments: app
                    .harness
                    .state()
                    .state
                    .environments
                    .iter()
                    .map(|env| (env.id, env.name.clone(), false))
                    .collect(),
                globals: false,
                postman: false,
            },
        ),
        ("import review with few notes", support::import_review(3)),
        (
            "import review with collapsed notes",
            support::import_review(12),
        ),
    ] {
        app.harness.state_mut().state.dialog = Some(dialog);
        check(app, name);
    }
    click(&mut app.harness, "12 notes");
    check(app, "import review with expanded notes");
    click_button(&mut app.harness, "Close");
}

#[test]
fn no_control_runs_past_the_window_or_overlaps_another() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let stream_url = format!("http://{}/stream", listener.local_addr().unwrap());
    let (send_event, mut event_requested) = watch::channel(false);
    runtime.spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            event_requested.wait_for(|ready| *ready).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: ready\n\n",
                )
                .await
                .unwrap();
            event_requested.wait_for(|ready| !*ready).await.unwrap();
        }
    });
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
                            env::set_active_environment(db, Some(prod.id))?;
                            env::create_environment(
                                db,
                                LONG_ENVIRONMENT,
                                &imposter_app::core::model::Variables::new(),
                            )?;
                            imposter_app::store::kv::set(
                                db,
                                imposter_app::store::kv::Table::Settings,
                                "mock_server",
                                &imposter_app::core::mock::MockConfig {
                                    port: 0,
                                    routes: vec![imposter_app::core::mock::MockRoute::default()],
                                },
                            )
                        })
                        .unwrap();
                    let api = collection(store, "API");
                    request(store, api.id, "Users", &url);
                    request(store, api.id, "Health", "http://127.0.0.1:9/");
                    request(store, api.id, "Broken", &format!("{url}/broken"));
                    request(store, api.id, "Logo", &format!("{url}/logo"));
                    request(
                        store,
                        api.id,
                        "Unresolved",
                        &format!("{url}/{{{{unknown}}}}"),
                    );
                    store
                        .call_blocking(move |db| {
                            use imposter_app::core::model::RequestTemplate;
                            use imposter_app::store::collections::{create_folder, create_request};
                            let folder = create_folder(db, api.id, None, "Examples", "", 5)?;
                            let nested =
                                create_folder(db, api.id, Some(folder.id), "Users", "", 0)?;
                            create_request(
                                db,
                                api.id,
                                Some(nested.id),
                                &RequestTemplate {
                                    name: "Remove user".into(),
                                    method: "DELETE".into(),
                                    url,
                                    ..RequestTemplate::default()
                                },
                            )?;
                            Ok(())
                        })
                        .unwrap();
                });
                app.harness.set_size(size);
                appearance(&mut app, theme, density);
                let label = format!("{}x{} {theme:?} {density:?}", size.x, size.y);
                let mut first_start = support::start();
                first_start.harness.set_size(size);
                appearance(&mut first_start, theme, density);
                problems.extend(self::problems(
                    &first_start,
                    &format!("{label} first start"),
                ));
                problems.extend(walk(&mut app, &label, &stream_url, &send_event));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
