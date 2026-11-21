//! The History screen: every execution, newest first, with its executed request and response.
//! Restoring opens an unsaved draft; nothing is sent.

use egui::{Rect, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::execution::Outcome;
use imposter_app::core::run_report::Step as ReportStep;
use imposter_app::core::run_result::Source;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::executions::{ExecutionSummary, Usage};
use imposter_app::transfer::HeadlessResult;

use crate::app::{Action, HISTORY_PAGE, HistoryAction, HistoryTab, ResponseTab, SentTab, State};
use crate::icons::{self, Icon};
use crate::theme::{self, Tone, palette};
use crate::viewer::{self, Language};
use crate::widgets::{self, TabSpec, Variant, name, tone_text};

const SEPARATOR: f32 = 6.0;
/// The environment column shows from this width on; below it the URL needs the room.
const ENVIRONMENT_COLUMN_FROM: f32 = 900.0;

/// A row's status as a pill: the code, or what happened when no response came.
fn outcome_text(row: &ExecutionSummary) -> (String, Tone) {
    match row.outcome {
        Outcome::Response => (
            row.status.to_string(),
            match row.status {
                200..=299 => Tone::Ok,
                300..=399 => Tone::Warning,
                _ => Tone::Error,
            },
        ),
        Outcome::TransportError => ("Failed".into(), Tone::Error),
        Outcome::Cancelled => ("Cancelled".into(), Tone::Warning),
    }
}

fn megabytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // a display approximation
    let mib = bytes as f64 / (1024.0 * 1024.0);
    format!("{mib:.0} MiB")
}

/// What history holds against what it keeps: "312 of 1000 executions · 41 MB of 250 MiB".
/// Without a reading yet, only the limits.
pub fn retention(settings: &AppSettings, usage: Option<Usage>) -> String {
    match usage {
        Some(usage) => format!(
            "{} of {} executions · {} of {}",
            usage.entries,
            settings.history_max_entries,
            viewer::format_size(usize::try_from(usage.bytes).unwrap_or(usize::MAX)),
            megabytes(settings.history_max_bytes)
        ),
        None => format!(
            "Keeps up to {} executions and {}",
            settings.history_max_entries,
            megabytes(settings.history_max_bytes)
        ),
    }
}

/// The table's columns, left to right, with their widths for a list `width` wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    When,
    Name,
    Method,
    Url,
    Status,
    Duration,
    Environment,
}

impl Column {
    fn title(self) -> &'static str {
        match self {
            Self::When => "When",
            Self::Name => "Name",
            Self::Method => "Method",
            Self::Url => "URL",
            Self::Status => "Status",
            Self::Duration => "Time",
            Self::Environment => "Environment",
        }
    }
}

/// Fixed columns at their width, Name and URL sharing the rest (URL twice as much).
fn columns(width: f32) -> Vec<(Column, f32)> {
    let environment = width >= ENVIRONMENT_COLUMN_FROM;
    let fixed = 96.0 + 72.0 + 92.0 + 72.0 + if environment { 120.0 } else { 0.0 } + 8.0;
    let free = (width - fixed).max(160.0);
    let mut columns = vec![
        (Column::When, 96.0),
        (Column::Name, (free / 3.0).floor()),
        (Column::Method, 72.0),
        (Column::Url, (free * 2.0 / 3.0).floor()),
        (Column::Status, 92.0),
        (Column::Duration, 72.0),
    ];
    if environment {
        columns.push((Column::Environment, 120.0));
    }
    columns
}

/// One line of text cut with "…" to fit `width`.
fn cut(job: &mut egui::text::LayoutJob, width: f32) {
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
}

fn text_job(text: &str, color: egui::Color32, font: &egui::FontId) -> egui::text::LayoutJob {
    egui::text::LayoutJob::simple_singleline(text.to_owned(), font.clone(), color)
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let mut actions = Vec::new();
    let now = imposter_app::core::execution::Timestamp(jiff::Timestamp::now().as_millisecond());
    let history = &mut state.history;
    toolbar(ui, history, &mut actions);

    let area = ui.available_rect_before_wrap();
    let ratio = state.layout.history_ratio;
    let list_height = crate::layout::split(
        area.height() - SEPARATOR,
        ratio,
        crate::layout::MIN_HISTORY_LIST,
        crate::layout::MIN_HISTORY_DETAILS,
    );
    let list = Rect::from_min_size(area.min, vec2(area.width(), list_height));
    let handle = Rect::from_min_size(list.left_bottom(), vec2(area.width(), SEPARATOR));
    let details = Rect::from_min_max(handle.left_bottom(), area.max);
    let drag = ui
        .interact(handle, ui.scope_id().with("history split"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if drag.dragged() && area.height() > SEPARATOR {
        // From the list's height as drawn, which a minimum may hold away from the ratio.
        state.layout.history_ratio = ((list_height + drag.drag_delta().y)
            / (area.height() - SEPARATOR))
            .clamp(crate::layout::RATIO.0, crate::layout::RATIO.1);
    }
    widgets::split_grip(ui, handle, false, drag.hovered() || drag.dragged());

    let retention = retention(&state.settings, history.usage);
    ui.scope_builder(
        UiBuilder::new().max_rect(list).id_salt("history list"),
        |ui| table(ui, history, now, &retention, &mut actions),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(details)
            .id_salt("history details"),
        |ui| detail(ui, history, &mut actions),
    );
    ui.allocate_rect(area, Sense::hover());
    state.actions.extend(actions);
}

/// The filter, "Current request only" and Refresh; clearing everything is in the menu at
/// the end, away from them.
fn toolbar(ui: &mut Ui, history: &mut crate::app::HistoryScreen, actions: &mut Vec<Action>) {
    let mut right_actions = Vec::new();
    widgets::screen_toolbar(
        ui,
        "History",
        |ui| {
            let gap = theme::spacing().sm;
            let chip_width = widgets::action_width(ui, "Current request only");
            let width = (ui.available_width() - chip_width - gap).min(360.0);
            let field = ui.add_sized(
                [width, theme::spacing().control],
                egui::TextEdit::singleline(&mut history.filter).hint_text("Filter"),
            );
            name(&field, "Filter history");
            let field = widgets::describe(
                field,
                "Words match the request name and URL. Use status:404, status:4xx, status:failed, \
                 status:cancelled, method:POST, url:…, name:…, env:…, via:cli, via:app, \
                 AND, OR, NOT, -word, \"phrases\" or parentheses.",
            );
            if field.changed() {
                actions.push(Action::History(HistoryAction::Filtered));
            }
            let mut only = history.only_current;
            if widgets::toggle_button(ui, &mut only, "Current request only")
                .on_hover_text("Show only executions of the open request")
                .clicked()
            {
                actions.push(Action::History(HistoryAction::OnlyCurrent(only)));
            }
        },
        |ui| {
            icons::menu(ui, Icon::More, "History actions", |ui| {
                if widgets::button(ui, Variant::Ghost, None, "Open result file…")
                    .on_hover_text("Show a result that imposter send wrote with --out")
                    .clicked()
                {
                    right_actions.push(Action::History(HistoryAction::OpenResult));
                }
                ui.separator();
                if widgets::button(ui, Variant::Ghost, None, "Clear all history…").clicked() {
                    right_actions.push(Action::History(HistoryAction::Clear));
                }
            });
            if icons::button(ui, Icon::Refresh, "Refresh", false).clicked() {
                right_actions.push(Action::History(HistoryAction::Refresh));
            }
        },
    );
    actions.extend(right_actions);
}

/// The executions as a table, newest first, with the pager under it.
fn table(
    ui: &mut Ui,
    history: &mut crate::app::HistoryScreen,
    now: imposter_app::core::execution::Timestamp,
    retention: &str,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let columns = columns(width);
    let row_height = theme::spacing().row;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(8.0);
        for (column, width) in &columns {
            widgets::heading_cell(ui, *width, column.title());
        }
    });
    // The scroll area can grow around its content. Reserve the pager independently so it
    // stays inside the list pane even when there is room for less than one full row.
    let area = ui.available_rect_before_wrap();
    let footer_top = (area.bottom() - theme::spacing().control).max(area.top());
    let footer = Rect::from_min_max(egui::pos2(area.left(), footer_top), area.max);
    let body_bottom = (footer_top - ui.spacing().item_spacing.y).max(area.top());
    let body = Rect::from_min_max(area.min, egui::pos2(area.right(), body_bottom));
    ui.scope_builder(
        UiBuilder::new().max_rect(body).id_salt("history rows"),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body));
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .min_scrolled_height(body.height())
                .max_height(body.height())
                .show_rows(ui, row_height, history.rows.len(), |ui, range| {
                    widgets::scroll_view_bounds(ui);
                    for index in range {
                        let row = &history.rows[index];
                        let selected = history.selected == Some(row.id);
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(width, row_height), Sense::click());
                        let when = viewer::local_time(row.timestamp);
                        let via = match row.via {
                            imposter_app::core::execution::Via::App => "",
                            imposter_app::core::execution::Via::CommandLine => {
                                ", from the command line"
                            }
                        };
                        name(
                            &response,
                            &format!("History {} {} {when}{via}", row.method, row.url),
                        );
                        let fill = if selected {
                            palette().selected
                        } else if response.hovered() {
                            palette().hover
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, 4.0, fill);
                        if selected {
                            widgets::selection_bar(ui, rect);
                        }
                        row_cells(ui, rect, row, &columns, now);
                        let response = response.on_hover_text(&when);
                        if response.clicked() {
                            actions.push(Action::History(HistoryAction::Select(row.id)));
                        }
                    }
                });
        },
    );
    ui.scope_builder(
        UiBuilder::new().max_rect(footer).id_salt("history pager"),
        |ui| {
            pager(ui, history, retention, actions);
        },
    );
}

/// One row's cells, painted: the list is virtualized, so nothing is laid out off screen.
fn row_cells(
    ui: &Ui,
    rect: Rect,
    row: &ExecutionSummary,
    columns: &[(Column, f32)],
    now: imposter_app::core::execution::Timestamp,
) {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let palette = palette();
    let mut x = rect.left() + 8.0;
    for (column, width) in columns {
        let cell = Rect::from_min_size(egui::pos2(x, rect.top()), vec2(width - 8.0, rect.height()));
        x += width;
        let mut job = match column {
            Column::When => text_job(
                &viewer::relative_time(row.timestamp, now),
                palette.text_muted,
                &small,
            ),
            Column::Name => {
                let name = if row.name.is_empty() {
                    "Untitled request"
                } else {
                    &row.name
                };
                let mut job = egui::text::LayoutJob::default();
                // A run from the command line (an agent, a script) says so before its name,
                // where a cut name cannot hide it.
                if row.via == imposter_app::core::execution::Via::CommandLine {
                    job.append(
                        "CLI  ",
                        0.0,
                        egui::TextFormat::simple(small.clone(), palette.text_muted),
                    );
                }
                job.append(
                    name,
                    0.0,
                    egui::TextFormat::simple(body.clone(), palette.text),
                );
                job
            }
            Column::Method => text_job(&row.method, theme::method_color(&row.method), &mono),
            Column::Url => {
                let (origin, path) = viewer::split_origin(&row.url);
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    origin,
                    0.0,
                    egui::TextFormat::simple(mono.clone(), palette.text_muted),
                );
                job.append(
                    path,
                    0.0,
                    egui::TextFormat::simple(mono.clone(), palette.text),
                );
                job
            }
            Column::Status => {
                let (text, tone) = outcome_text(row);
                let color = palette.tone(tone);
                let galley = ui
                    .painter()
                    .layout_no_wrap(text, theme::semibold(12.0), color);
                let pill = Rect::from_center_size(
                    egui::pos2(cell.left() + galley.size().x / 2.0 + 8.0, cell.center().y),
                    galley.size() + vec2(16.0, 2.0),
                );
                ui.painter()
                    .rect_filled(pill, 10.0, color.gamma_multiply(widgets::PILL_TINT));
                ui.painter()
                    .galley(pill.min + vec2(8.0, 1.0), galley, color);
                continue;
            }
            Column::Duration => {
                let text = if row.outcome == Outcome::Cancelled {
                    String::new()
                } else {
                    viewer::format_duration(row.elapsed)
                };
                text_job(&text, palette.text_muted, &small)
            }
            Column::Environment => text_job(&row.environment, palette.text_muted, &small),
        };
        cut(&mut job, cell.width());
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(
            egui::pos2(cell.left(), cell.center().y - galley.size().y / 2.0),
            galley,
            palette.text,
        );
    }
}

/// Which executions show, how many history keeps, and Newer and Older.
fn pager(
    ui: &mut Ui,
    history: &mut crate::app::HistoryScreen,
    retention: &str,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let older = ui.add_enabled_ui(history.rows.len() as u64 == HISTORY_PAGE, |ui| {
                icons::button(ui, Icon::ChevronRight, "Older", false)
            });
            if older.inner.clicked() {
                actions.push(Action::History(HistoryAction::Older));
            }
            let newer = ui.add_enabled_ui(history.offset > 0, |ui| {
                icons::button(ui, Icon::ChevronLeft, "Newer", false)
            });
            if newer.inner.clicked() {
                actions.push(Action::History(HistoryAction::Newer));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if let Some((status, tone)) = &history.status {
                    ui.add(egui::Label::new(tone_text(status, *tone)).truncate())
                        .on_hover_text(status);
                }
                if let Some(usage) = history.usage {
                    let count = usage.entries;
                    let noun = if count == 1 {
                        "execution"
                    } else {
                        "executions"
                    };
                    let size =
                        viewer::format_size(usize::try_from(usage.bytes).unwrap_or(usize::MAX));
                    let text =
                        theme::meta(format!("{count} {noun} · {size}")).color(palette().text_muted);
                    let hint = if history.rows.is_empty() {
                        retention.to_owned()
                    } else {
                        let title = if history.filter.trim().is_empty() {
                            "Executions"
                        } else {
                            "Matches"
                        };
                        let first = history.offset + 1;
                        let last = history.offset + history.rows.len() as u64;
                        format!("{title} {first}-{last} · {retention}")
                    };
                    let footer = ui.add(egui::Label::new(text).truncate());
                    footer.ctx.accesskit_node_builder(footer.id, |node| {
                        node.set_description(hint.as_str());
                    });
                    footer.on_hover_text(hint);
                }
            });
        });
    });
}

/// The selected execution: one line with what was sent and how it went, and its actions;
/// then one row of tabs for the response and the request as sent.
fn detail(ui: &mut Ui, history: &mut crate::app::HistoryScreen, actions: &mut Vec<Action>) {
    if let Some(opened) = &history.opened {
        result_header(ui, opened, actions);
        if let HeadlessResult::Run(report) = &opened.document {
            if report.steps.is_empty() {
                return;
            }
            result_picker(ui, report, opened.step, actions);
        }
    } else if !record_header(ui, history, actions) {
        return;
    }
    let empty_response = history.opened.as_ref().and_then(|opened| {
        if let HeadlessResult::Run(report) = &opened.document
            && let Some(ReportStep::Skipped { reason, .. }) = report.steps.get(opened.step)
        {
            return Some(tone_text(reason, Tone::Muted));
        }
        opened
            .current()
            .filter(|result| result.response.is_none())
            .map(|result| egui::RichText::new(&result.error))
    });
    let view = HistoryViewer {
        tab: &mut history.tab,
        request_tab: &mut history.request_tab,
        request: history.request.as_ref(),
        body: &mut history.request_body_view,
        headers: &mut history.request_headers_view,
        empty_response,
    };
    if let Some(crate::workspace::PaneAction::SaveBody) =
        crate::workspace::history_response(ui, &mut history.response, view)
    {
        actions.push(crate::workspace::save_body_action(&history.response));
    }
}

/// What a result file says about its run: the summary in the outcome's tone, then where it
/// came from, and a close button.
fn result_header(ui: &mut Ui, opened: &crate::app::OpenedResult, actions: &mut Vec<Action>) {
    let (summary, outcome, definition, info, command) = match &opened.document {
        HeadlessResult::Send(result) => (
            &result.summary,
            result.outcome,
            &result.definition,
            &result.run,
            "imposter send",
        ),
        HeadlessResult::Run(report) => (
            &report.summary,
            report.outcome,
            &report.definition,
            &report.run,
            if report.run.tool.starts_with("imposter mcp ") {
                "imposter mcp"
            } else {
                "imposter run"
            },
        ),
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Icon::Close, "Close result", false).clicked() {
                actions.push(Action::History(HistoryAction::CloseResult));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(tone_text(summary, viewer::outcome_tone(outcome)))
                        .truncate()
                        .selectable(true),
                )
                .on_hover_text(summary);
            });
        });
    });
    let environment = definition
        .environment
        .as_deref()
        .unwrap_or("no environment");
    let run: String = info.id.chars().take(8).collect();
    // A run against the profile is also in History; a file run never is.
    let kept = match definition.source {
        Source::Profile => "also in History",
        Source::File => "from a collection file",
        Source::Project => "from a project folder",
    };
    let origin = format!(
        "From {command} · {} · {environment} · run {run} · {kept}",
        definition.request
    );
    let mut details = vec![format!("Opened from {}", opened.file)];
    if let Some(file) = &definition.file {
        details.push(format!("Ran {file}"));
    }
    details.push(format!("Started {} by {}", info.started, info.tool));
    let details = details.join("\n");
    ui.add(egui::Label::new(tone_text(origin, Tone::Muted)).truncate())
        .on_hover_text(details);
    if let Some(sha256) = &definition.sha256 {
        let short: String = sha256.chars().take(12).collect();
        ui.label(tone_text(format!("Revision {short}"), Tone::Muted))
            .on_hover_text(format!("SHA-256 {sha256}"));
    }
}

fn result_picker(
    ui: &mut Ui,
    report: &imposter_app::core::run_report::RunReport,
    selected: usize,
    actions: &mut Vec<Action>,
) {
    fn label(step: &ReportStep) -> String {
        match step {
            ReportStep::Ran(result) => format!("{} · {}", step.path(), result.summary),
            ReportStep::Skipped { .. } => format!("{} · Skipped", step.path()),
        }
    }
    let response = widgets::combo("headless run steps")
        .selected_text(report.steps.get(selected).map(label).unwrap_or_default())
        .truncate()
        .show_ui(ui, |ui| {
            for (index, step) in report.steps.iter().enumerate() {
                if ui
                    .selectable_label(index == selected, label(step))
                    .clicked()
                {
                    actions.push(Action::History(HistoryAction::ResultStep(index)));
                }
            }
        })
        .response;
    name(&response, "Result request");
}

/// The selected execution's line and actions; `false` while there is nothing to show.
fn record_header(
    ui: &mut Ui,
    history: &mut crate::app::HistoryScreen,
    actions: &mut Vec<Action>,
) -> bool {
    if history.selected.is_none() {
        ui.add_space(theme::spacing().lg);
        ui.vertical_centered(|ui| {
            ui.label(tone_text(
                "Select an execution to see what was sent and received.",
                Tone::Muted,
            ))
        });
        return false;
    }
    let (Some(record), Some(request)) = (&history.record, &history.request) else {
        ui.label(tone_text("Loading…", Tone::Muted));
        return false;
    };
    let checks = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button_with_hint(
                    ui,
                    Icon::Terminal,
                    "Copy as curl",
                    "Copy the request as it was sent, as a curl command",
                    false,
                )
                .clicked()
                {
                    actions.push(Action::History(HistoryAction::CopyCurl));
                }
                if widgets::button(ui, Variant::Secondary, None, "Restore as draft")
                    .on_hover_text("Open an editable request draft without sending it.")
                    .clicked()
                {
                    actions.push(Action::History(HistoryAction::Restore));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    crate::workspace::history_status(ui, &record.response, &request.summary)
                })
                .inner
            })
            .inner
        })
        .inner;
    if checks {
        history.tab = HistoryTab::Response;
        history.response.tab = ResponseTab::Tests;
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Response(ResponseTab),
    Request(SentTab),
}

pub(crate) struct HistoryViewer<'a> {
    tab: &'a mut HistoryTab,
    request_tab: &'a mut SentTab,
    request: Option<&'a viewer::SentRequest>,
    body: &'a mut widgets::ViewerState,
    headers: &'a mut widgets::ViewerState,
    pub empty_response: Option<egui::RichText>,
}

impl HistoryViewer<'_> {
    pub fn shows_response(&self) -> bool {
        *self.tab == HistoryTab::Response
    }

    pub fn tabs(
        &mut self,
        ui: &mut Ui,
        tabs: Vec<TabSpec<'_, ResponseTab>>,
        response_tab: &mut ResponseTab,
        response_reserve: f32,
        draw_tools: impl FnOnce(&mut Ui, ResponseTab),
    ) {
        let mut selected = match *self.tab {
            HistoryTab::Response => DetailTab::Response(*response_tab),
            HistoryTab::Request => DetailTab::Request(*self.request_tab),
        };
        let mut tabs: Vec<_> = tabs
            .into_iter()
            .map(|tab| TabSpec {
                value: DetailTab::Response(tab.value),
                label: tab.label,
                name: tab.name,
                badge: tab.badge,
            })
            .collect();
        if self.request.is_some() {
            tabs.extend([
                TabSpec::new(DetailTab::Request(SentTab::Body), "Request body"),
                TabSpec::new(DetailTab::Request(SentTab::Headers), "Request headers"),
            ]);
        }
        ui.horizontal(|ui| {
            let reserve = match selected {
                DetailTab::Response(_) => response_reserve,
                DetailTab::Request(tab)
                    if self
                        .request
                        .as_ref()
                        .is_some_and(|request| !request_text(request, tab).is_empty()) =>
                {
                    3.0 * (icons::size() + ui.spacing().item_spacing.x)
                }
                DetailTab::Request(_) => 0.0,
            };
            widgets::tab_strip(
                ui,
                &tabs,
                &mut selected,
                tabs.len(),
                reserve,
                "More History tabs",
            );
            match selected {
                DetailTab::Response(tab) => {
                    *self.tab = HistoryTab::Response;
                    *response_tab = tab;
                    draw_tools(ui, tab);
                }
                DetailTab::Request(tab) => {
                    *self.tab = HistoryTab::Request;
                    *self.request_tab = tab;
                    if let Some(request) = self.request {
                        let text = request_text(request, tab);
                        if !text.is_empty() {
                            let (state, label) = match tab {
                                SentTab::Body => (&mut *self.body, "Copy request body"),
                                SentTab::Headers => (&mut *self.headers, "Copy request headers"),
                            };
                            widgets::viewer_tools(ui, state, text, label);
                        }
                    }
                }
            }
        });
    }

    pub fn show_request(&mut self, ui: &mut Ui) {
        let Some(request) = self.request else { return };
        let (state, id, language, empty) = match *self.request_tab {
            SentTab::Body => (&mut *self.body, "request body", request.language, "No body"),
            SentTab::Headers => (
                &mut *self.headers,
                "request headers",
                Language::Plain,
                "No headers",
            ),
        };
        let text = request_text(request, *self.request_tab);
        if text.is_empty() {
            ui.add_space(theme::spacing().lg);
            ui.vertical_centered(|ui| ui.label(tone_text(empty, Tone::Muted)));
        } else {
            let _ = widgets::code_viewer(ui, id, state, text, language, None);
        }
    }
}

fn request_text(request: &viewer::SentRequest, tab: SentTab) -> &str {
    match tab {
        SentTab::Body => &request.body,
        SentTab::Headers => &request.headers,
    }
}
