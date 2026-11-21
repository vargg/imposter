//! Editing and controlling the local static mock server.

use egui::{Align, Layout, Ui};
use imposter_app::core::mock::{MockConfig, MockRoute, RouteField, RouteProblem};
use imposter_app::core::model::ParamItem;
use imposter_app::runtime::TaskId;

use crate::app::{Action, State};
use crate::icons::{self, Icon};
use crate::layout::MOCK_LIST_WIDTH;
use crate::theme::{self, Tone, palette};
use crate::widgets::{self, SaveStatus, SaveWord, Variant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockAction {
    Start,
    Stop,
    Reload,
    Edited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transition {
    Reload,
    Close,
}

pub struct Run {
    pub task: TaskId,
    pub config: MockConfig,
    pub address: Option<String>,
    pub stopping: bool,
}

#[derive(Default)]
pub struct MockScreen {
    pub form: MockConfig,
    pub(crate) baseline: Option<MockConfig>,
    pub(crate) saving: bool,
    pub(crate) loading: bool,
    pub(crate) pending: Option<Transition>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub run: Option<Run>,
    pub(crate) generation: u64,
    selected: Option<usize>,
    /// A custom method being typed for the selected route.
    custom_method: Option<String>,
    /// Requests the running server answered, newest first; cleared on Start.
    pub log: std::collections::VecDeque<LogLine>,
}

/// An answered request and the time it arrived in the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// "14:02:03".
    pub when: String,
    pub entry: imposter_app::core::mock::MockLogEntry,
}

/// How many answered requests the screen keeps.
pub const LOG_KEPT: usize = 100;

impl MockScreen {
    pub fn dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|base| base != &self.form)
    }

    pub fn loaded(&self) -> bool {
        self.baseline.is_some()
    }

    /// Keep an answered request, newest first.
    pub fn logged(&mut self, entry: imposter_app::core::mock::MockLogEntry) {
        self.log.push_front(LogLine {
            when: crate::viewer::clock_seconds(imposter_app::executor::now()),
            entry,
        });
        self.log.truncate(LOG_KEPT);
    }
}

/// What keeps the server from starting besides `MockConfig::problems`: an enabled route
/// whose script does not compile, as `imposter_app::mock::script_problem` refuses it.
pub fn script_problems(config: &MockConfig) -> Vec<RouteProblem> {
    config
        .routes
        .iter()
        .enumerate()
        .filter(|(_, route)| route.enabled && route.is_scripted())
        .filter_map(|(index, route)| {
            imposter_app::script::check(&route.script)
                .err()
                .map(|error| RouteProblem {
                    route: index,
                    field: RouteField::Script,
                    reason: error.describe(imposter_app::mock::MOCK_SCRIPT),
                })
        })
        .collect()
}

/// A field outlined in the error color while `invalid`.
fn checked_text(ui: &mut Ui, label: &str, value: &mut String, width: f32, invalid: bool) -> bool {
    ui.scope(|ui| {
        if invalid {
            let stroke = egui::Stroke::new(1.0, palette().error);
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                state.bg_stroke = stroke;
            }
            ui.visuals_mut().selection.stroke = stroke;
        }
        let response = ui.add(egui::TextEdit::singleline(value).desired_width(width));
        widgets::name(&response, label);
        response.changed()
    })
    .inner
}

/// The reasons `problems` give for `field` of route `route`, one line each.
fn problem_lines(ui: &mut Ui, problems: &[RouteProblem], route: usize, field: RouteField) {
    for problem in problems
        .iter()
        .filter(|p| p.route == route && p.field == field)
    {
        ui.label(widgets::tone_text(problem.reason.as_str(), Tone::Error));
    }
}

// The original split leaves this much room for the editor at the minimum window width.
const MIN_EDITOR_WIDTH: f32 = 372.0;
const HALF_DIVIDER: i8 = 6;

pub fn draw(state: &mut State, ui: &mut Ui) {
    let screen = &mut state.mocks;
    let mut edited = false;
    // Checked every frame, so a problem shows while it is typed, not only on Save.
    let mut problems = screen.form.problems();
    problems.extend(script_problems(&screen.form));
    let mut actions = Vec::new();
    let mut port = screen.form.port;
    let mut server_action = None;
    widgets::screen_toolbar(
        ui,
        "Mocks",
        |ui| {
            port_field(ui, screen, &mut port);
            server_state(ui, screen);
        },
        |ui| {
            server_action = server_controls(ui, screen, &problems);
            save_controls(ui, screen, &mut actions);
        },
    );
    actions.extend(server_action.map(Action::Mock));
    edited |= port != screen.form.port;
    screen.form.port = port;
    if let Some(run) = &screen.run
        && screen
            .baseline
            .as_ref()
            .is_some_and(|base| *base != run.config)
    {
        ui.label(widgets::tone_text("Restart to apply", Tone::Warning));
    }
    if let Some(notice) = &screen.notice {
        ui.label(notice);
    }
    ui.add_space(6.0);
    let loaded = screen.loaded() && !screen.loading;
    ui.add_enabled_ui(loaded, |ui| {
        let area = ui.available_rect_before_wrap();
        let (min, max) = MOCK_LIST_WIDTH;
        let inset = f32::from(HALF_DIVIDER);
        let max_width = (area.width() - 2.0 * inset - MIN_EDITOR_WIDTH)
            .floor()
            .clamp(min, max);
        let width = state.layout.mock_list_width.min(max_width);
        let frame = egui::Frame::new().inner_margin(egui::Margin {
            // The panel's separator already reserves one point of the gutter.
            right: HALF_DIVIDER - 1,
            ..egui::Margin::ZERO
        });
        if let Some(resized) = widgets::resizable_list_panel(
            ui,
            "mock routes",
            frame,
            width + inset,
            min + inset..=max_width + inset,
            |ui| {
                edited |= routes(ui, screen, &problems);
            },
        ) {
            state.layout.mock_list_width = resized - inset;
        }
        let mut editor = ui.available_rect_before_wrap();
        editor.min.x += inset;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(editor)
                .id_salt("mock route"),
            |ui| {
                edited |= route_editor(ui, screen, &problems);
            },
        );
        ui.allocate_rect(area, egui::Sense::hover());
    });
    if edited {
        actions.push(Action::Mock(MockAction::Edited));
    }
    state.actions.extend(actions);
}

fn port_field(ui: &mut Ui, screen: &MockScreen, port: &mut u16) {
    ui.label(widgets::tone_text("Port", Tone::Muted));
    let editable = screen.loaded() && !screen.loading;
    let field = ui
        .add_enabled_ui(editable, |ui| {
            ui.add_sized(
                [64.0, theme::spacing().control],
                egui::DragValue::new(port)
                    .range(0..=65535)
                    .custom_formatter(|port, _| {
                        if port == 0.0 {
                            "auto".to_owned()
                        } else {
                            format!("{port}")
                        }
                    }),
            )
        })
        .inner
        .on_hover_text("auto: a free port chosen when the server starts");
    widgets::name(&field, "Mock port");
}

fn server_controls(
    ui: &mut Ui,
    screen: &MockScreen,
    problems: &[RouteProblem],
) -> Option<MockAction> {
    if let Some(run) = &screen.run {
        return ui
            .add_enabled_ui(!run.stopping, |ui| {
                let button = widgets::button(ui, Variant::Danger, Some(Icon::Stop), "Stop");
                widgets::name(&button, "Stop mock server");
                button
            })
            .inner
            .clicked()
            .then_some(MockAction::Stop);
    }
    let ready = screen.loaded()
        && !screen.dirty()
        && !screen.saving
        && !screen.loading
        && problems.is_empty();
    ui.add_enabled_ui(ready, |ui| {
        let button = widgets::button(ui, Variant::Primary, Some(Icon::Play), "Start");
        widgets::name(&button, "Start mock server");
        button
    })
    .inner
    .on_disabled_hover_text(match problems.first() {
        Some(problem) => format!("Fix route {} first: {}", problem.route + 1, problem.reason),
        None => "Save changes before starting".to_owned(),
    })
    .clicked()
    .then_some(MockAction::Start)
}

fn save_controls(ui: &mut Ui, screen: &MockScreen, actions: &mut Vec<Action>) {
    let status = SaveStatus {
        word: if screen.saving {
            SaveWord::Saving
        } else if screen.error.is_some() {
            SaveWord::Failed
        } else if !screen.loaded() {
            SaveWord::Loading
        } else if screen.dirty() {
            SaveWord::Unsaved
        } else {
            SaveWord::Saved
        },
        detail: screen
            .error
            .as_deref()
            .unwrap_or("Save the configuration; restart the server to apply changes"),
        enabled: screen.loaded() && screen.dirty() && !screen.saving && !screen.loading,
    };
    if widgets::save_controls(ui, &status, "Save mocks") {
        actions.push(Action::Save);
    }
    let revert = ui
        .add_enabled_ui(!screen.saving && !screen.loading, |ui| {
            icons::button(ui, Icon::Revert, "Revert to saved mocks", false)
        })
        .inner;
    if revert.clicked() {
        actions.push(Action::Mock(MockAction::Reload));
    }
}

/// Whether the server runs and where.
fn server_state(ui: &mut Ui, screen: &MockScreen) {
    let Some(run) = &screen.run else {
        ui.label(widgets::tone_text("Stopped", Tone::Muted));
        return;
    };
    let (dot, text) = match (&run.address, run.stopping) {
        (_, true) => (palette().warning, "Stopping…".to_owned()),
        (None, false) => (palette().warning, "Starting…".to_owned()),
        (Some(address), false) => (palette().ok, format!("Running at {address}")),
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, dot);
    let copy_space = run
        .address
        .as_ref()
        .map_or(0.0, |_| icons::size() + ui.spacing().item_spacing.x);
    let width = widgets::text_width(ui, theme::body(&text))
        .min((ui.available_width() - copy_space).max(0.0));
    ui.add_sized(
        [width, theme::spacing().control],
        egui::Label::new(theme::body(&text))
            .truncate()
            .halign(Align::Min),
    )
    .on_hover_text(&text);
    if let Some(address) = &run.address
        && icons::button(ui, Icon::Copy, "Copy mock server URL", false).clicked()
    {
        ui.ctx().copy_text(address.to_owned());
    }
}

/// The routes, one row each: enabled, method in its color, path, status; a route with a
/// problem is marked. Returns whether anything changed.
fn routes(ui: &mut Ui, screen: &mut MockScreen, problems: &[RouteProblem]) -> bool {
    let mut edited = false;
    if widgets::add_row_header(ui, "Routes", "Add mock route", None) {
        screen.form.routes.push(MockRoute::default());
        screen.selected = Some(screen.form.routes.len() - 1);
        screen.custom_method = None;
        edited = true;
    }
    let height = theme::spacing().row;
    // The log takes the lower half while it has something to show.
    let list_height = if screen.log.is_empty() {
        ui.available_height()
    } else {
        (ui.available_height() * 0.45).max(3.0 * height)
    };
    egui::ScrollArea::vertical()
        .id_salt("mock route list")
        .auto_shrink([false, true])
        .max_height(list_height)
        .show(ui, |ui| {
            if screen.form.routes.is_empty() {
                ui.label(widgets::tone_text("No routes yet", Tone::Muted));
            }
            for (index, route) in screen.form.routes.iter_mut().enumerate() {
                let selected = screen.selected == Some(index);
                let broken = problems.iter().any(|p| p.route == index);
                ui.horizontal(|ui| {
                    let enabled = widgets::checkbox(ui, &mut route.enabled, "");
                    widgets::name(&enabled, &format!("Mock route {} enabled", index + 1));
                    edited |= enabled.changed();
                    let mono = egui::TextStyle::Monospace.resolve(ui.style());
                    let body = egui::TextStyle::Body.resolve(ui.style());
                    let mut job = egui::text::LayoutJob::default();
                    let muted = !route.enabled;
                    let fade =
                        |color: egui::Color32| if muted { palette().text_muted } else { color };
                    job.append(
                        &format!("{:<7}", route.method),
                        0.0,
                        egui::TextFormat::simple(
                            mono.clone(),
                            fade(theme::method_color(&route.method)),
                        ),
                    );
                    job.append(
                        &route.path,
                        0.0,
                        egui::TextFormat::simple(mono, fade(palette().text)),
                    );
                    let status = if broken {
                        ("  ⚠".to_owned(), palette().error)
                    } else if route.is_scripted() {
                        (format!("  {} · script", route.status), palette().text_muted)
                    } else {
                        (format!("  {}", route.status), palette().text_muted)
                    };
                    let row = ui.add_sized(
                        [ui.available_width(), height],
                        egui::Button::selectable(
                            selected,
                            (
                                job,
                                egui::Atom::grow(),
                                egui::RichText::new(status.0).font(body).color(status.1),
                            ),
                        )
                        .stroke(egui::Stroke::NONE)
                        .truncate(),
                    );
                    if selected {
                        widgets::selection_bar(ui, row.rect);
                    }
                    widgets::name(&row, &format!("Mock route {}", index + 1));
                    // What the row shows after the path, for screen readers.
                    let described = if broken {
                        "Has a problem".to_owned()
                    } else if route.is_scripted() {
                        format!("Status {}, answered by its script", route.status)
                    } else {
                        format!("Status {}", route.status)
                    };
                    ui.ctx().accesskit_node_builder(row.id, |node| {
                        node.set_description(described);
                    });
                    let row = if broken {
                        row.on_hover_text("This route has a problem; select it to see what")
                    } else {
                        row
                    };
                    if row.clicked() {
                        screen.selected = Some(index);
                        screen.custom_method = None;
                    }
                });
            }
        });
    if !screen.log.is_empty() {
        ui.add_space(8.0);
        request_log(ui, screen);
    }
    edited
}

/// Requests the server answered, newest first: what came in, what went out, how long it took,
/// and a script's error and printed lines. Never bodies.
fn request_log(ui: &mut Ui, screen: &mut MockScreen) {
    ui.horizontal(|ui| {
        // Not "Requests", which names the header's tab.
        ui.label(egui::RichText::new("Answered requests").font(theme::semibold(13.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let clear = ui.small_button("Clear");
            widgets::name(&clear, "Clear the mock request log");
            if clear.clicked() {
                screen.log.clear();
            }
        });
    });
    egui::ScrollArea::vertical()
        .id_salt("mock request log")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in &screen.log {
                let entry = &line.entry;
                let tone = match entry.status {
                    200..=299 => Tone::Ok,
                    300..=399 => Tone::Warning,
                    _ => Tone::Error,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(widgets::tone_text(line.when.as_str(), Tone::Muted).small());
                    ui.label(
                        egui::RichText::new(format!("{} {}", entry.method, entry.path)).monospace(),
                    );
                    ui.label(widgets::tone_text(format!("→ {}", entry.status), tone));
                    ui.label(
                        widgets::tone_text(
                            crate::viewer::format_duration(entry.elapsed.as_secs_f64()),
                            Tone::Muted,
                        )
                        .small(),
                    );
                });
                if let Some(error) = &entry.error {
                    ui.add(
                        egui::Label::new(widgets::tone_text(error.as_str(), Tone::Error)).wrap(),
                    );
                }
                for printed in &entry.console {
                    ui.add(
                        egui::Label::new(
                            widgets::tone_text(printed.as_str(), Tone::Muted).monospace(),
                        )
                        .wrap(),
                    );
                }
            }
        });
}

/// The selected route: method and path, status, headers, body, and Delete route.
fn route_editor(ui: &mut Ui, screen: &mut MockScreen, problems: &[RouteProblem]) -> bool {
    let mut edited = false;
    // The first route is open until another is chosen, so the editor is never idle.
    if screen.selected.is_none() && !screen.form.routes.is_empty() {
        screen.selected = Some(0);
    }
    let Some(index) = screen
        .selected
        .filter(|index| *index < screen.form.routes.len())
    else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(widgets::tone_text(
                if screen.form.routes.is_empty() {
                    "Add a route to answer requests with a fixed response."
                } else {
                    "Select a route to edit its response."
                },
                Tone::Muted,
            ));
        });
        return false;
    };
    let invalid = |field| {
        problems
            .iter()
            .any(|p| p.route == index && p.field == field)
    };
    let mut remove = false;
    let height = ui.available_height();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::scroll_view_bounds(ui);
            let top = ui.cursor().top();
            let route = &mut screen.form.routes[index];
            ui.horizontal(|ui| {
                ui.label(theme::section(format!("Route {}", index + 1)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    remove = widgets::delete_button(
                        ui,
                        ui.scope_id().with(("route", index)),
                        "mock route",
                        "Delete route",
                    );
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let picker =
                    widgets::method_picker(ui, &mut route.method, &mut screen.custom_method);
                edited |= picker;
                edited |= checked_text(
                    ui,
                    "Mock path",
                    &mut route.path,
                    ui.available_width(),
                    invalid(RouteField::Path) || invalid(RouteField::Route),
                );
            });
            for field in [RouteField::Method, RouteField::Path, RouteField::Route] {
                problem_lines(ui, problems, index, field);
            }
            ui.horizontal(|ui| {
                ui.label(theme::body("Enabled"));
                let enabled = widgets::toggle(ui, &mut route.enabled);
                widgets::name(&enabled, "Enabled");
                edited |= enabled.changed();
                ui.add_space(theme::spacing().sm);
                ui.label("Status");
                let response = ui.add(egui::DragValue::new(&mut route.status).range(200..=599));
                widgets::name(&response, "Mock response status");
                edited |= response.changed();
            });
            problem_lines(ui, problems, index, RouteField::Status);
            ui.add_space(8.0);
            ui.label(theme::section("Response headers"));
            edited |= response_headers(ui, &mut route.headers, index, problems);
            ui.add_space(8.0);
            ui.label(theme::section("Response body"));
            let control = theme::spacing().control;
            let gap = theme::spacing().sm;
            // Measure from the form's start so scrolling cannot grow either editor.
            let script_header = control + gap + 2.0 * ui.spacing().item_spacing.y;
            let available = height - (ui.cursor().top() - top) - script_header;
            let body_height = (available / 2.0).max(4.0 * control);
            let script_height = (available - body_height).max(4.0 * control);
            let json = route.body.trim_start().starts_with(['{', '['])
                || route.headers.iter().any(|header| {
                    header.is_enabled
                        && header.name.eq_ignore_ascii_case("content-type")
                        && crate::viewer::language(&header.value) == crate::viewer::Language::Json
                });
            edited |= widgets::plain_body_editor(
                ui, &mut route.body, json, "Mock response body", body_height,
            );
            ui.add_space(gap);
            ui.push_id(("mock script", index), |ui| {
                edited |= crate::scripts_tab::script_editor(
                    ui,
                    imposter_app::script::Phase::Mock,
                    "Response script (optional)",
                    "Runs for each matched request; it starts from the status, headers and body above",
                    "Mock response script",
                    &mut route.script,
                    script_height,
                );
            });
            // The editor says why it does not compile; the list's ⚠ and Start say it too.
        });
    if remove {
        screen.form.routes.remove(index);
        screen.selected = None;
        edited = true;
    }
    edited
}

fn response_headers(
    ui: &mut Ui,
    rows: &mut Vec<ParamItem>,
    route: usize,
    problems: &[RouteProblem],
) -> bool {
    use crate::widgets::{ENABLED_WIDTH, blank_cell, heading_cell};
    let widths = widgets::key_value_widths(ui, 0.0);
    let height = theme::spacing().control;
    let mut edited = false;
    let mut remove = None;
    ui.horizontal(|ui| {
        blank_cell(ui, ENABLED_WIDTH);
        heading_cell(ui, widths[0], "Name");
        heading_cell(ui, widths[1], "Value");
    });
    for (index, header) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let enabled = widgets::checkbox(ui, &mut header.is_enabled, "");
            widgets::name(&enabled, &format!("Mock header {} enabled", index + 1));
            edited |= enabled.changed();
            edited |= header_fields(
                ui,
                header,
                (route, index),
                widths,
                &format!("Mock header {}", index + 1),
            );
            if widgets::remove_button(
                ui,
                egui::Id::unique(("remove mock header", route, index)),
                &format!("mock header {}", index + 1),
                height,
            ) {
                remove = Some(index);
            }
        });
        problem_lines(ui, problems, route, RouteField::Header(index));
    }
    let mut row = ParamItem::new("", "");
    ui.horizontal(|ui| {
        blank_cell(ui, ENABLED_WIDTH);
        header_fields(ui, &mut row, (route, rows.len()), widths, "New mock header");
        blank_cell(ui, icons::size());
    });
    if !row.name.is_empty() || !row.value.is_empty() {
        edited |= widgets::append_row(ui, rows, row);
    }
    if let Some(index) = remove {
        rows.remove(index);
        edited = true;
    }
    edited
}

fn header_fields(
    ui: &mut Ui,
    row: &mut ParamItem,
    index: (usize, usize),
    widths: [f32; 2],
    label: &str,
) -> bool {
    let mut edited = false;
    for ((column, text), width) in [("name", &mut row.name), ("value", &mut row.value)]
        .into_iter()
        .zip(widths)
    {
        edited |= widgets::table_text(
            ui,
            text,
            egui::Id::unique(("mock header", index, column)),
            &format!("{label} {column}"),
            width,
            column,
            false,
        )
        .changed();
    }
    edited
}
