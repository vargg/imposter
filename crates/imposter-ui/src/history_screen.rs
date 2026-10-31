//! The History screen: every execution, newest first, with its executed request and response.
//! Restoring opens an unsaved draft; nothing is sent.

use egui::{Rect, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::execution::Outcome;
use imposter_app::store::executions::ExecutionSummary;

use crate::app::{Action, HISTORY_PAGE, HistoryAction, HistoryTab, SentTab, State};
use crate::theme::{Tone, palette};
use crate::viewer::{self, Language};
use crate::widgets::{self, name, tone_text};

const SEPARATOR: f32 = 6.0;
/// Below this width the toolbar wraps rather than pinning "Clear all" to the right edge.
const TOOLBAR_ONE_ROW: f32 = 900.0;

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
        Outcome::TransportError => ("Connection failed".into(), Tone::Error),
        Outcome::Cancelled => ("Cancelled".into(), Tone::Warning),
    }
}

fn megabytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // a display approximation
    let mib = bytes as f64 / (1024.0 * 1024.0);
    format!("{mib:.0} MiB")
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let mut actions = Vec::new();
    let history = &mut state.history;
    // One row with "Clear all" at the right edge; in a narrow window the row wraps instead,
    // so its buttons never overlap.
    let tools =
        |ui: &mut Ui, actions: &mut Vec<Action>, history: &mut crate::app::HistoryScreen| {
            let mut only = history.only_current;
            if ui.checkbox(&mut only, "Current request only").changed() {
                actions.push(Action::History(HistoryAction::OnlyCurrent(only)));
            }
            if ui.button("Refresh").clicked() {
                actions.push(Action::History(HistoryAction::Refresh));
            }
            if ui
                .add_enabled(history.offset > 0, egui::Button::new("Newer"))
                .clicked()
            {
                actions.push(Action::History(HistoryAction::Newer));
            }
            if ui
                .add_enabled(
                    history.rows.len() as u64 == HISTORY_PAGE,
                    egui::Button::new("Older"),
                )
                .clicked()
            {
                actions.push(Action::History(HistoryAction::Older));
            }
            let restore = ui
                .add_enabled(
                    history.record.is_some(),
                    egui::Button::new("Restore as draft"),
                )
                .on_hover_text("Open an editable request draft without sending it.");
            if restore.clicked() {
                actions.push(Action::History(HistoryAction::Restore));
            }
            if ui
                .add_enabled(history.record.is_some(), egui::Button::new("Copy as curl"))
                .on_hover_text("Copy the request as it was sent, as a curl command")
                .clicked()
            {
                actions.push(Action::History(HistoryAction::CopyCurl));
            }
        };
    let clear = |ui: &mut Ui, actions: &mut Vec<Action>| {
        if ui.button("Clear all history…").clicked() {
            actions.push(Action::History(HistoryAction::Clear));
        }
    };
    if ui.available_width() >= TOOLBAR_ONE_ROW {
        ui.horizontal(|ui| {
            tools(ui, &mut actions, history);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clear(ui, &mut actions);
            });
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            tools(ui, &mut actions, history);
            clear(ui, &mut actions);
        });
    }
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut history.filter)
                .hint_text("Filter: text, status:4xx, method:POST, env:…")
                .desired_width(260.0_f32.min(ui.available_width() * 0.5)),
        );
        name(&field, "Filter history");
        let field = field.on_hover_text(
            "Words match the request name and URL. status:404, status:4xx, status:failed, \
             method:, url:, name:, env:, OR, NOT, -word and \"phrases\". Bodies are not searched.",
        );
        if field.changed() {
            actions.push(Action::History(HistoryAction::Filtered));
        }
        if let Some((status, tone)) = &history.status {
            ui.label(tone_text(status.as_str(), *tone));
        }
        ui.label(tone_text(
            format!(
                "· Keeps up to {} executions and {}",
                state.settings.history_max_entries,
                megabytes(state.settings.history_max_bytes)
            ),
            Tone::Muted,
        ));
    });

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

    ui.scope_builder(
        UiBuilder::new().max_rect(list).id_salt("history list"),
        |ui| {
            let width = ui.available_width();
            let columns = [
                180.0,
                70.0,
                (width - 180.0 - 70.0 - 130.0 - 140.0).max(160.0),
                130.0,
                140.0,
            ];
            let header = ["Time", "Method", "URL", "Outcome", "Environment"];
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(4.0);
                for (title, width) in header.into_iter().zip(columns) {
                    widgets::heading_cell(ui, width, title);
                }
            });
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(
                    ui,
                    crate::theme::spacing().row,
                    history.rows.len(),
                    |ui, range| {
                        for index in range {
                            let row = &history.rows[index];
                            let selected = history.selected == Some(row.id);
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(width, crate::theme::spacing().row),
                                Sense::click(),
                            );
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
                            let (outcome, tone) = outcome_text(row);
                            let cells = [
                                (when, palette().text),
                                (row.method.clone(), palette().text),
                                (row.url.clone(), palette().text),
                                (outcome, palette().tone(tone)),
                                (row.environment.clone(), palette().text_muted),
                            ];
                            let mut x = rect.left() + 4.0;
                            for ((text, color), column) in cells.into_iter().zip(columns) {
                                let cell = Rect::from_min_size(
                                    egui::pos2(x, rect.top()),
                                    vec2(column - 8.0, crate::theme::spacing().row),
                                );
                                let galley = ui.painter().layout(
                                    text,
                                    egui::FontId::proportional(13.0),
                                    color,
                                    f32::INFINITY,
                                );
                                ui.painter().with_clip_rect(cell).galley(
                                    egui::pos2(
                                        cell.left(),
                                        cell.center().y - galley.size().y / 2.0,
                                    ),
                                    galley,
                                    color,
                                );
                                x += column;
                            }
                            if response.clicked() {
                                actions.push(Action::History(HistoryAction::Select(row.id)));
                            }
                        }
                    },
                );
        },
    );

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(details)
            .id_salt("history details"),
        |ui| {
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
            if history.record.is_none() {
                ui.label(tone_text("Loading…", Tone::Muted));
                return;
            }
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (HistoryTab::Request, "Request"),
                    (HistoryTab::Response, "Response"),
                ] {
                    if widgets::tab(ui, history.tab == tab, label).clicked() {
                        history.tab = tab;
                    }
                }
            });
            match history.tab {
                HistoryTab::Request => sent_request(ui, history),
                HistoryTab::Response => {
                    // History responses are not the open request's: no row adds a check.
                    if let Some(crate::workspace::PaneAction::SaveBody) =
                        crate::workspace::response_body(
                            ui,
                            &mut history.response,
                            "history",
                            true,
                            false,
                        )
                    {
                        actions.push(crate::workspace::save_body_action(&history.response));
                    }
                }
            }
        },
    );
    ui.allocate_rect(area, Sense::hover());
    state.actions.extend(actions);
}

/// The request as sent: `METHOD URL`, then its body or headers, like the response.
fn sent_request(ui: &mut Ui, history: &mut crate::app::HistoryScreen) {
    let Some(request) = &history.request else {
        return;
    };
    ui.add(
        egui::Label::new(
            egui::RichText::new(request.summary.as_str())
                .monospace()
                .strong(),
        )
        .selectable(true)
        .truncate(),
    )
    .on_hover_text(request.summary.as_str());
    ui.horizontal(|ui| {
        for (tab, label) in [(SentTab::Body, "Body"), (SentTab::Headers, "Headers")] {
            let response = widgets::tab(ui, history.request_tab == tab, label);
            name(&response, &format!("Request {}", label.to_lowercase()));
            if response.clicked() {
                history.request_tab = tab;
            }
        }
        match history.request_tab {
            SentTab::Body => widgets::viewer_tools(
                ui,
                &mut history.request_body_view,
                &request.body,
                "Copy request body",
            ),
            SentTab::Headers => widgets::viewer_tools(
                ui,
                &mut history.request_headers_view,
                &request.headers,
                "Copy request headers",
            ),
        }
    });
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
