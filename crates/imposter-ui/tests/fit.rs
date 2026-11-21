//! The smallest window: rows of tabs and tools stay on one line, and what does not fit moves
//! into a menu instead of wrapping onto more rows.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::check::{Check, Op};
use imposter_app::core::model::{BodyType, ParamItem};
use imposter_app::core::select::Selector;
use imposter_ui::app::{Action, Appearance, EditorTab};
use imposter_ui::layout::{MIN_PANE_WIDE, MIN_WINDOW, RAIL_WIDTH, SIDEBAR_WIDTH, WorkspaceLayout};
use imposter_ui::theme::{self, Density, ThemeChoice};
use support::{click, collection, request, settle, start_with};

const PRIMARY: [&str; 4] = ["Body", "Auth", "Query", "Headers"];
const SECONDARY: [&str; 6] = [
    "Cookies",
    "Request settings",
    "Docs",
    "Tests",
    "Captures",
    "Scripts",
];
const MORE: &str = "More request tabs";

fn editor_tab_labels() -> impl Iterator<Item = &'static str> {
    PRIMARY.into_iter().chain(SECONDARY)
}

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

#[test]
fn the_rail_and_sidebar_leave_room_for_both_pane_minimums() {
    let mut app = small();
    let preferred = WorkspaceLayout::default().sidebar_width;
    let far_right = |app: &support::App| {
        app.harness
            .get_by_label("Collection actions")
            .rect()
            .right()
    };
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        for density in [Density::Compact, Density::Comfortable] {
            app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
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
            let panes = ["Request pane", "Response pane"]
                .map(|label| app.harness.get_by_role_and_label(Role::Group, label).rect());
            assert!(panes[0].right() < panes[1].left());
            for pane in panes {
                assert!(
                    pane.width() >= MIN_PANE_WIDE,
                    "{theme:?} {density:?}: {pane:?}"
                );
            }
            let narrow_right = far_right(&app);
            assert!(narrow_right >= RAIL_WIDTH + SIDEBAR_WIDTH.0 - 24.0);
            assert!(narrow_right < RAIL_WIDTH + preferred - 24.0);
            app.harness.set_size(egui::vec2(1500.0, 850.0));
            app.harness.run_steps(3);
            assert!(
                far_right(&app) > narrow_right + 40.0,
                "the sidebar returns to its preferred width"
            );
            assert!((app.harness.state().state.layout.sidebar_width - preferred).abs() < 0.5);
        }
    }
}

/// The tops of the request tabs in the row.
fn editor_tab_tops(app: &support::App) -> Vec<f32> {
    editor_tab_labels()
        .filter_map(|label| app.harness.query_by_role_and_label(Role::Button, label))
        .map(|node| node.rect().top())
        .collect()
}

#[test]
fn request_tabs_keep_one_row_and_the_rest_are_under_more() {
    let mut app = small();
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .headers = vec![ParamItem::new("Accept", "*/*")];
    app.harness.run_steps(3);
    let tops = editor_tab_tops(&app);
    assert!(
        tops.len() < PRIMARY.len(),
        "even primaries overflow when the pane is too narrow"
    );
    assert!(
        tops.iter().all(|top| (top - tops[0]).abs() < 1.0),
        "one row: {tops:?}"
    );
    assert!(app.harness.query_by_label("Headers").is_none());
    click(&mut app.harness, MORE);
    let headers = app.harness.get_by_role_and_label(Role::Button, "Headers");
    assert_eq!(
        headers.accesskit_node().description().as_deref(),
        Some("1 in use")
    );
    click(&mut app.harness, "Tests");
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
fn a_wide_window_shows_four_primary_tabs_and_the_selected_menu_tab() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.run_steps(3);
    let hidden = |app: &support::App| {
        assert_eq!(editor_tab_tops(app).len(), PRIMARY.len());
        for label in PRIMARY.into_iter().chain([MORE]) {
            app.harness.get_by_role_and_label(Role::Button, label);
        }
        for label in SECONDARY {
            assert!(app.harness.query_by_label(label).is_none(), "{label}");
        }
    };
    hidden(&app);
    click(&mut app.harness, MORE);
    let options = SECONDARY.map(|label| {
        app.harness
            .get_by_role_and_label(Role::Button, label)
            .rect()
            .top()
    });
    assert!(options.windows(2).all(|pair| pair[0] < pair[1]));
    click(&mut app.harness, "Scripts");
    let rect = |label| {
        app.harness
            .get_by_role_and_label(Role::Button, label)
            .rect()
    };
    let scripts = app.harness.get_by_role_and_label(Role::Button, "Scripts");
    assert_eq!(scripts.accesskit_node().is_selected(), Some(true));
    assert_eq!(editor_tab_tops(&app).len(), PRIMARY.len() + 1);
    assert!(rect("Headers").right() <= scripts.rect().left());
    assert!(scripts.rect().right() <= rect(MORE).left());
    assert!((scripts.rect().top() - rect("Body").top()).abs() < 1.0);
    click(&mut app.harness, "Body");
    hidden(&app);
    click(&mut app.harness, MORE);
    app.harness.get_by_role_and_label(Role::Button, "Scripts");
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

/// The top of the stacked split's handle, found by its resize cursor along `x`.
fn handle_top(app: &mut support::App, x: f32) -> f32 {
    let mut y = 100.0;
    while y < 800.0 {
        app.harness.hover_at(egui::pos2(x, y));
        app.harness.run_steps(1);
        if app.harness.output().platform_output.cursor_icon == egui::CursorIcon::ResizeVertical {
            return y;
        }
        y += 1.0;
    }
    panic!("no split handle along x = {x}");
}

/// Drag the split handle whose top is at `top` along `x` by `by` points, in steps.
fn drag_handle(app: &mut support::App, x: f32, top: f32, by: f32) {
    let grab = top + 2.0;
    app.harness.drag_at(egui::pos2(x, grab));
    app.harness.run_steps(1);
    for step in 1..=10_u8 {
        app.harness
            .hover_at(egui::pos2(x, grab + by * f32::from(step) / 10.0));
        app.harness.run_steps(1);
    }
    app.harness.drop_at(egui::pos2(x, grab + by));
    app.harness.run_steps(2);
}

#[test]
fn a_stacked_pane_keeps_its_minimum_and_follows_the_drag_back_at_once() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.state_mut().state.layout.orientation = imposter_ui::layout::Orientation::Stacked;
    app.harness.run_steps(3);
    let x = 900.0;
    let body = app
        .harness
        .query_all_by_role_and_label(Role::Button, "Body")
        .map(|node| node.rect().top())
        .fold(f32::INFINITY, f32::min);
    let start = handle_top(&mut app, x);
    drag_handle(&mut app, x, start, -600.0);
    let lowest = handle_top(&mut app, x);
    // The request pane's tabs sit a padding below its top.
    let pane = lowest - body;
    assert!(
        (pane - imposter_ui::layout::MIN_PANE_TALL).abs() <= 20.0,
        "held near its minimum: {pane}"
    );
    drag_handle(&mut app, x, lowest, 50.0);
    let back = handle_top(&mut app, x);
    assert!(
        (back - lowest - 50.0).abs() <= 2.0,
        "the drag back moves the pane at once: {lowest} -> {back}"
    );
}

#[test]
fn body_type_and_json_tools_share_the_header_above_a_full_height_editor() {
    let mut app = small();
    assert_eq!(
        app.harness.state().state.layout.orientation,
        imposter_ui::layout::Orientation::SideBySide
    );
    let choices = [
        ("None", "None"),
        ("JSON", "JSON"),
        ("Text", "Plain text"),
        ("Form", "URL-encoded form"),
        ("Multipart", "Multipart form"),
        ("File", "Binary file"),
    ];
    for (size, segments, density) in [
        (egui::vec2(1024.0, 580.0), true, Density::Compact),
        (egui::vec2(1024.0, 580.0), true, Density::Comfortable),
        (egui::Vec2::from(MIN_WINDOW), false, Density::Compact),
        (egui::Vec2::from(MIN_WINDOW), false, Density::Comfortable),
    ] {
        app.harness.set_size(size);
        app.harness
            .state_mut()
            .state
            .actions
            .push(Action::SetAppearance(Appearance {
                theme: ThemeChoice::Dark,
                density,
                ..Appearance::default()
            }));
        {
            let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
            pane.editor.body_type = BodyType::None;
            pane.editor.body_text = r#"{"a": 1}"#.into();
        }
        app.harness.run_steps(3);
        if segments {
            let group = app.harness.get_by_role_and_label(Role::Group, "Body type");
            let options: Vec<_> = group
                .children_recursive()
                .map(|node| node.accesskit_node())
                .filter(|node| node.role() == Role::Button)
                .map(|node| (node.label().unwrap(), node.description().unwrap()))
                .collect();
            let expected = choices.map(|(short, full)| (short.to_owned(), full.to_owned()));
            assert_eq!(options, expected);
            assert!(
                app.harness
                    .query_by_role_and_label(Role::ComboBox, "Body type")
                    .is_none()
            );
            let text = app
                .harness
                .get_by_role_and_label(Role::Button, "Text")
                .rect();
            app.harness.hover_at(text.center());
            app.harness.run_steps(40);
            app.harness.get_by_label("Plain text");
        } else {
            assert!(
                app.harness
                    .query_by_role_and_label(Role::Group, "Body type")
                    .is_none()
            );
            assert!(app.harness.query_by_label("JSON").is_none());
            click(&mut app.harness, "Body type");
            for (_, full) in choices {
                app.harness.get_by_role_and_label(Role::Button, full);
            }
        }
        click(&mut app.harness, "JSON");
        assert_eq!(
            app.harness
                .state()
                .state
                .editor
                .as_ref()
                .unwrap()
                .editor
                .body_type,
            BodyType::Json
        );
        let picker = if segments {
            let json = app.harness.get_by_role_and_label(Role::Button, "JSON");
            assert_eq!(
                json.accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::True)
            );
            app.harness
                .get_by_role_and_label(Role::Group, "Body type")
                .rect()
        } else {
            app.harness
                .get_by_role_and_label(Role::ComboBox, "Body type")
                .rect()
        };
        let status = app.harness.get_by_label("Valid JSON").rect();
        let format = app
            .harness
            .get_by_role_and_label(Role::Button, "Format body")
            .rect();
        let editor = app.harness.get_by_label("Request body").rect();
        let pane = app
            .harness
            .get_by_role_and_label(Role::Group, "Request pane")
            .rect();
        assert!(picker.right() <= status.left() && status.right() <= format.left());
        assert!((picker.center().y - status.center().y).abs() < 2.0);
        assert!((picker.center().y - format.center().y).abs() < 2.0);
        assert!(format.bottom() <= editor.top());
        assert!(
            (pane.bottom() - editor.bottom() - theme::spacing().sm).abs() < 2.0,
            "{size:?}: editor {editor:?} inside {pane:?}"
        );
    }
}

#[test]
fn an_invalid_json_body_keeps_a_truncated_error_and_full_hover_text() {
    let mut app = small();
    {
        let editor = &mut app
            .harness
            .state_mut()
            .state
            .editor
            .as_mut()
            .unwrap()
            .editor;
        editor.body_type = BodyType::Json;
        editor.body_text = "{\n  \"a\": }".into();
    }
    app.harness.run_steps(3);
    let status = app.harness.get_by_label("Line 2: expected value");
    let format = app
        .harness
        .get_by_role_and_label(Role::Button, "Format body");
    assert!(format.accesskit_node().is_disabled());
    let full = "Invalid JSON: expected value at line 2 column 8";
    assert_eq!(status.accesskit_node().description().as_deref(), Some(full));
    let rect = status.rect();
    assert!(app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Text(text)
            if rect.contains(text.pos) && text.galley.elided &&
               text.galley.job.sections[0].format.color == theme::palette().error)
    }));
    app.harness.hover_at(rect.center());
    app.harness.run_steps(40);
    app.harness.get_by_label(full);
}

#[test]
fn request_tabs_count_what_they_hold() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.editor.headers = vec![
            ParamItem::new("Accept", "*/*"),
            ParamItem::new("X-Trace", "1"),
        ];
    }
    app.harness.run_steps(3);
    let headers = app.harness.get_by_role_and_label(Role::Button, "Headers");
    assert_eq!(
        headers.accesskit_node().description().as_deref(),
        Some("2 in use")
    );
    let auth = app.harness.get_by_role_and_label(Role::Button, "Auth");
    assert_eq!(auth.accesskit_node().description(), None, "nothing set");
}

#[test]
fn more_marks_hidden_tabs_that_hold_something() {
    let mut app = small();
    app.harness.set_size(egui::vec2(1500.0, 850.0));
    app.harness.run_steps(3);
    let more = |app: &support::App| {
        let button = app.harness.get_by_role_and_label(Role::Button, MORE);
        (button.rect(), button.accesskit_node().description())
    };
    assert_eq!(more(&app).1.as_deref(), Some("More tabs"), "nothing set");
    assert!(
        !painted(&app, more(&app).0)
            .iter()
            .any(|(text, _)| text == "●")
    );
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .checks = vec![
        Check::new(Selector::Status, Op::Equals, "200"),
        Check::new(Selector::Header("Content-Type".into()), Op::Exists, ""),
    ];
    app.harness.run_steps(3);
    let (rect, hint) = more(&app);
    assert_eq!(hint.as_deref(), Some("More tabs: Tests: 2 checks"));
    assert!(
        painted(&app, rect)
            .iter()
            .any(|(text, format)| text == "●" && format.color == theme::palette().ok)
    );
    click(&mut app.harness, MORE);
    let tests = app.harness.get_by_role_and_label(Role::Button, "Tests");
    assert_eq!(
        tests.accesskit_node().description().as_deref(),
        Some("2 checks")
    );
    assert!(
        painted(&app, tests.rect())
            .iter()
            .any(|(text, _)| text == "2")
    );
    click(&mut app.harness, "Tests");
    assert_eq!(
        more(&app).1.as_deref(),
        Some("More tabs"),
        "Tests is visible"
    );
}

#[test]
fn the_check_marks_keep_their_place_in_the_smallest_window() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(imposter_testkit::Server::start(|_, _| {
        imposter_testkit::response("200 OK", &[("Content-Type", "application/json")], b"{}")
    }));
    let url = format!("{}/items", server.url);
    let mut app = start_with(move |store| {
        let api = collection(store, "API");
        request(store, api.id, "Items", &url);
    });
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    click(&mut app.harness, "GET Items");
    settle(&mut app.harness);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .checks = vec![
        Check::new(Selector::Status, Op::Equals, "200"),
        Check::new(Selector::Status, Op::Equals, "201"),
    ];
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    let checks = app
        .harness
        .get_by_label("Checks: 1 passed, 1 failed")
        .rect();
    let runs = app.harness.get_by_label("Recorded runs").rect();
    assert!(checks.right() <= runs.left(), "{checks:?} before {runs:?}");
}

/// The text painted inside `area`, with the format of each part.
fn painted(app: &support::App, area: egui::Rect) -> Vec<(String, egui::TextFormat)> {
    app.harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if area.contains(text.pos) => Some(text.galley.job.clone()),
            _ => None,
        })
        .flat_map(|job| {
            job.sections
                .iter()
                .map(|section| {
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    (job.text[range].to_owned(), section.format.clone())
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn a_set_tab_has_a_raised_green_dot_and_more_counts_what_it_hides() {
    let mut app = small();
    {
        let editor = &mut app
            .harness
            .state_mut()
            .state
            .editor
            .as_mut()
            .unwrap()
            .editor;
        editor.body_type = BodyType::Json;
        editor.body_text = "{}".into();
    }
    app.harness.run_steps(3);

    // A muted "•" at mid-height read as a separator between tabs.
    let body = app
        .harness
        .get_by_role_and_label(Role::Button, "Body")
        .rect();
    let marks: Vec<egui::TextFormat> = painted(&app, body)
        .into_iter()
        .filter(|(text, _)| text == "●")
        .map(|(_, format)| format)
        .collect();
    assert_eq!(marks.len(), 1, "one dot on Body");
    assert_eq!(marks[0].valign, egui::Align::TOP);
    assert_eq!(marks[0].color, theme::palette().ok);

    // "+n", unlike the tools' "⋯" that can sit beside it.
    let shown = editor_tab_tops(&app).len();
    let more = app
        .harness
        .get_by_role_and_label(Role::Button, "More request tabs")
        .rect();
    let hidden = editor_tab_labels().count() - shown;
    assert!(hidden > 0);
    assert!(
        painted(&app, more)
            .iter()
            .any(|(text, _)| *text == format!("+{hidden}")),
        "{:?}",
        painted(&app, more)
    );
}

#[test]
fn the_flat_response_centres_its_empty_message_on_both_sides_of_the_switch() {
    let mut app = small();
    for (layout, side_by_side) in [
        ("Put request and response side by side", true),
        ("Stack request and response", false),
    ] {
        click(&mut app.harness, layout);
        let pane = |label| app.harness.get_by_role_and_label(Role::Group, label).rect();
        let request = pane("Request pane");
        let response = pane("Response pane");
        let (title, detail) = (
            app.harness
                .get_by_label("Your response will appear here")
                .rect(),
            app.harness
                .get_by_label("Send a request to inspect its status, headers and body.")
                .rect(),
        );
        assert!((title.union(detail).center().y - response.center().y).abs() <= 1.0);
        let color = theme::palette().field_border;
        let shapes = &app.harness.output().shapes;
        assert!(
            shapes.iter().any(|clipped| {
                matches!(clipped.shape, egui::Shape::LineSegment { points: [a, b], stroke }
                if stroke.color == color && (stroke.width - 1.0).abs() < f32::EPSILON
                && if side_by_side {
                    request.right() < a.x && a.x < response.left()
                        && (b.y - a.y - request.height()).abs() < f32::EPSILON
                } else {
                    request.bottom() < a.y && a.y < response.top()
                        && (b.x - a.x - request.width()).abs() < f32::EPSILON
                })
            }),
            "a divider between the panes in {layout}"
        );
        assert!(shapes.iter().all(|clipped| {
            !matches!(&clipped.shape, egui::Shape::Rect(rect)
                if [request, response].contains(&rect.rect) && rect.fill == theme::palette().surface)
        }));
    }
}
