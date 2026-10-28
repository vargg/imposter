//! The History screen: every execution, newest first, with its executed request and response.
//! Restoring opens an unsaved draft; nothing is sent.

use egui::{Align, Layout, Rect, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::execution::Outcome;
use imposter_app::store::executions::ExecutionSummary;

use crate::app::{Action, HISTORY_PAGE, HistoryAction, HistoryTab, ResponseTab, State};
use crate::theme::{PALETTE, Tone};
use crate::viewer::{self, Language};
use crate::widgets::{self, name, tone_text};

const ROW_HEIGHT: f32 = 26.0;
const SEPARATOR: f32 = 6.0;

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
    ui.horizontal(|ui| {
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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Clear all history…").clicked() {
                actions.push(Action::History(HistoryAction::Clear));
            }
        });
    });
    ui.horizontal(|ui| {
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
    let list_height = (area.height() - SEPARATOR) * ratio;
    let list = Rect::from_min_size(area.min, vec2(area.width(), list_height));
    let handle = Rect::from_min_size(list.left_bottom(), vec2(area.width(), SEPARATOR));
    let details = Rect::from_min_max(handle.left_bottom(), area.max);
    let drag = ui
        .interact(handle, ui.scope_id().with("history split"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if drag.dragged() && area.height() > SEPARATOR {
        state.layout.history_ratio = (ratio + drag.drag_delta().y / (area.height() - SEPARATOR))
            .clamp(crate::layout::RATIO.0, crate::layout::RATIO.1);
    }
    let color = if drag.hovered() || drag.dragged() {
        PALETTE.accent
    } else {
        PALETTE.border
    };
    ui.painter().rect_filled(handle.shrink(2.0), 2.0, color);

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
                .show_rows(ui, ROW_HEIGHT, history.rows.len(), |ui, range| {
                    for index in range {
                        let row = &history.rows[index];
                        let selected = history.selected == Some(row.id);
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
                        let when = viewer::local_time(row.timestamp);
                        name(
                            &response,
                            &format!("History {} {} {}", row.method, row.url, when),
                        );
                        let fill = if selected {
                            PALETTE.selected
                        } else if response.hovered() {
                            PALETTE.hover
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, 4.0, fill);
                        let (outcome, tone) = outcome_text(row);
                        let cells = [
                            (when, PALETTE.text),
                            (row.method.clone(), PALETTE.text),
                            (row.url.clone(), PALETTE.text),
                            (outcome, PALETTE.tone(tone)),
                            (row.environment.clone(), PALETTE.text_muted),
                        ];
                        let mut x = rect.left() + 4.0;
                        for ((text, color), column) in cells.into_iter().zip(columns) {
                            let cell = Rect::from_min_size(
                                egui::pos2(x, rect.top()),
                                vec2(column - 8.0, ROW_HEIGHT),
                            );
                            let galley = ui.painter().layout(
                                text,
                                egui::FontId::proportional(13.0),
                                color,
                                f32::INFINITY,
                            );
                            ui.painter().with_clip_rect(cell).galley(
                                egui::pos2(cell.left(), cell.center().y - galley.size().y / 2.0),
                                galley,
                                color,
                            );
                            x += column;
                        }
                        if response.clicked() {
                            actions.push(Action::History(HistoryAction::Select(row.id)));
                        }
                    }
                });
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
                    crate::workspace::response_body(ui, &mut history.response, "history", true);
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
        for (tab, label) in [
            (ResponseTab::Body, "Body"),
            (ResponseTab::Headers, "Headers"),
        ] {
            let response = widgets::tab(ui, history.request_tab == tab, label);
            name(&response, &format!("Request {}", label.to_lowercase()));
            if response.clicked() {
                history.request_tab = tab;
            }
        }
        match history.request_tab {
            ResponseTab::Body => widgets::viewer_tools(
                ui,
                &mut history.request_body_view,
                &request.body,
                "Copy request body",
            ),
            ResponseTab::Headers => widgets::viewer_tools(
                ui,
                &mut history.request_headers_view,
                &request.headers,
                "Copy request headers",
            ),
        }
    });
    match history.request_tab {
        ResponseTab::Body => widgets::code_viewer(
            ui,
            "request body",
            &mut history.request_body_view,
            &request.body,
            request.language,
        ),
        ResponseTab::Headers => widgets::code_viewer(
            ui,
            "request headers",
            &mut history.request_headers_view,
            &request.headers,
            Language::Plain,
        ),
    }
}
