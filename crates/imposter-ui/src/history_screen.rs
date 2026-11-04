//! The History screen: every execution, newest first, with its executed request and response.
//! Restoring opens an unsaved draft; nothing is sent.

use egui::{Rect, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::execution::Outcome;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::executions::{ExecutionSummary, Usage};

use crate::app::{Action, HISTORY_PAGE, HistoryAction, HistoryTab, SentTab, State};
use crate::theme::{Tone, palette};
use crate::viewer::{self, Language};
use crate::widgets::{self, name, tone_text};

const SEPARATOR: f32 = 6.0;
/// The environment column shows from this width on; below it the URL needs the room.
const ENVIRONMENT_COLUMN_FROM: f32 = 900.0;
/// Height of the pager under the list.
const PAGER: f32 = 30.0;

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
    // A hairline across, and a grip that says it can be dragged.
    ui.painter().hline(
        handle.x_range(),
        handle.center().y,
        egui::Stroke::new(1.0, palette().border),
    );
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
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 280.0).clamp(140.0, 360.0);
        let field = ui.add(
            egui::TextEdit::singleline(&mut history.filter)
                .hint_text("Filter: text, status:4xx, method:POST, env:…")
                .desired_width(width),
        );
        name(&field, "Filter history");
        let field = field.on_hover_text(
            "Words match the request name and URL. status:404, status:4xx, status:failed, \
             method:, url:, name:, env:, OR, NOT, -word and \"phrases\". Bodies are not searched.",
        );
        if field.changed() {
            actions.push(Action::History(HistoryAction::Filtered));
        }
        let mut only = history.only_current;
        if ui.checkbox(&mut only, "Current request only").changed() {
            actions.push(Action::History(HistoryAction::OnlyCurrent(only)));
        }
        if crate::icons::button(ui, crate::icons::Icon::Refresh, "Refresh", false).clicked() {
            actions.push(Action::History(HistoryAction::Refresh));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::icons::menu(ui, crate::icons::Icon::More, "History actions", |ui| {
                if ui.button("Clear all history…").clicked() {
                    actions.push(Action::History(HistoryAction::Clear));
                }
            });
        });
    });
    ui.add_space(4.0);
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
    let row_height = crate::theme::spacing().row;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(8.0);
        for (column, width) in &columns {
            widgets::heading_cell(ui, *width, column.title());
        }
    });
    let body = ui.available_height() - PAGER;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(body.max(row_height))
        .show_rows(ui, row_height, history.rows.len(), |ui, range| {
            for index in range {
                let row = &history.rows[index];
                let selected = history.selected == Some(row.id);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(width, row_height), Sense::click());
                let when = viewer::local_time(row.timestamp);
                name(
                    &response,
                    &format!("History {} {} {}", row.method, row.url, when),
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
    pager(ui, history, retention, actions);
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
            Column::Name => text_job(
                if row.name.is_empty() {
                    "Untitled request"
                } else {
                    &row.name
                },
                palette.text,
                &body,
            ),
            Column::Method => text_job(&row.method, crate::theme::method_color(&row.method), &mono),
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
                    .layout_no_wrap(text, crate::theme::semibold(12.0), color);
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
        ui.set_height(PAGER);
        if let Some((status, tone)) = &history.status {
            ui.label(tone_text(status.as_str(), *tone));
        }
        ui.add(egui::Label::new(tone_text(format!("· {retention}"), Tone::Muted)).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let older = ui.add_enabled_ui(history.rows.len() as u64 == HISTORY_PAGE, |ui| {
                crate::icons::button(ui, crate::icons::Icon::ChevronRight, "Older", false)
            });
            if older.inner.clicked() {
                actions.push(Action::History(HistoryAction::Older));
            }
            let newer = ui.add_enabled_ui(history.offset > 0, |ui| {
                crate::icons::button(ui, crate::icons::Icon::ChevronLeft, "Newer", false)
            });
            if newer.inner.clicked() {
                actions.push(Action::History(HistoryAction::Newer));
            }
        });
    });
}

/// The selected execution: one line with what was sent and how it went, and its actions;
/// then one row of tabs for the response and the request as sent.
fn detail(ui: &mut Ui, history: &mut crate::app::HistoryScreen, actions: &mut Vec<Action>) {
    if history.selected.is_none() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(tone_text(
                "Select an execution to see what was sent and received.",
                Tone::Muted,
            ))
        });
        return;
    }
    let (Some(record), Some(request)) = (&history.record, &history.request) else {
        ui.label(tone_text("Loading…", Tone::Muted));
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The request bar's icon for the same action.
            let curl = crate::icons::button_with_hint(
                ui,
                crate::icons::Icon::Terminal,
                "Copy as curl",
                "Copy the request as it was sent, as a curl command",
                false,
            );
            if curl.clicked() {
                actions.push(Action::History(HistoryAction::CopyCurl));
            }
            if ui
                .button("Restore as draft")
                .on_hover_text("Open an editable request draft without sending it.")
                .clicked()
            {
                actions.push(Action::History(HistoryAction::Restore));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let status = viewer::status(&record.response);
                let mut job = egui::text::LayoutJob::default();
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                job.append(
                    &request.summary,
                    0.0,
                    egui::TextFormat::simple(mono, palette().text),
                );
                job.append(
                    &format!("  ·  {}", status.text),
                    0.0,
                    egui::TextFormat::simple(
                        egui::TextStyle::Body.resolve(ui.style()),
                        palette().tone(status.tone),
                    ),
                );
                job.append(
                    &format!("  ·  {}", viewer::format_duration(record.response.elapsed)),
                    0.0,
                    egui::TextFormat::simple(
                        egui::TextStyle::Body.resolve(ui.style()),
                        palette().text_muted,
                    ),
                );
                ui.add(egui::Label::new(job).truncate().selectable(true))
                    .on_hover_text(request.summary.as_str());
            });
        });
    });
    // One row of tabs: the response, then the request's body and headers as sent.
    ui.horizontal(|ui| {
        for (tab, sent, label, accessible) in [
            (HistoryTab::Response, None, "Response", "Response"),
            (
                HistoryTab::Request,
                Some(SentTab::Body),
                "Request body",
                "Request body",
            ),
            (
                HistoryTab::Request,
                Some(SentTab::Headers),
                "Request headers",
                "Request headers",
            ),
        ] {
            let selected =
                history.tab == tab && sent.is_none_or(|sent| history.request_tab == sent);
            let response = widgets::tab(ui, selected, label);
            name(&response, accessible);
            if response.clicked() {
                history.tab = tab;
                if let Some(sent) = sent {
                    history.request_tab = sent;
                }
            }
        }
        if history.tab == HistoryTab::Request {
            let (view, text, copy) = match history.request_tab {
                SentTab::Body => (
                    &mut history.request_body_view,
                    &request.body,
                    "Copy request body",
                ),
                SentTab::Headers => (
                    &mut history.request_headers_view,
                    &request.headers,
                    "Copy request headers",
                ),
            };
            if !text.is_empty() {
                widgets::viewer_tools(ui, view, text, copy);
            }
        }
    });
    match history.tab {
        HistoryTab::Request => sent_request(ui, history),
        HistoryTab::Response => {
            // History responses are not the open request's: no row adds a check.
            if let Some(crate::workspace::PaneAction::SaveBody) =
                crate::workspace::response_body(ui, &mut history.response, "history", true, false)
            {
                actions.push(crate::workspace::save_body_action(&history.response));
            }
        }
    }
}

/// The request's body or headers as sent; an empty one says so in words, not as code.
fn sent_request(ui: &mut Ui, history: &mut crate::app::HistoryScreen) {
    let Some(request) = &history.request else {
        return;
    };
    let (text, empty) = match history.request_tab {
        SentTab::Body => (&request.body, "No body"),
        SentTab::Headers => (&request.headers, "No headers"),
    };
    if text.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| ui.label(tone_text(empty, Tone::Muted)));
        return;
    }
    let _ = match history.request_tab {
        SentTab::Body => widgets::code_viewer(
            ui,
            "request body",
            &mut history.request_body_view,
            &request.body,
            request.language,
            None,
        ),
        SentTab::Headers => widgets::code_viewer(
            ui,
            "request headers",
            &mut history.request_headers_view,
            &request.headers,
            Language::Plain,
            None,
        ),
    };
}
