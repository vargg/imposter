//! Editing and controlling the local static mock server.

use egui::{Align, Layout, Ui};
use poster_app::core::mock::{MockConfig, MockRoute};
use poster_app::core::model::ParamItem;
use poster_app::runtime::TaskId;

use crate::app::{Action, State};
use crate::theme::Tone;
use crate::widgets::{self, SaveStatus};

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
}

impl MockScreen {
    pub fn dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|base| base != &self.form)
    }

    pub fn loaded(&self) -> bool {
        self.baseline.is_some()
    }
}

fn text(ui: &mut Ui, label: &str, value: &mut String, width: f32) -> bool {
    let response = ui.add(egui::TextEdit::singleline(value).desired_width(width));
    widgets::name(&response, label);
    response.changed()
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let screen = &mut state.mocks;
    let mut edited = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let status = SaveStatus {
                text: if screen.saving {
                    "Saving…"
                } else if screen.error.is_some() {
                    "Save failed"
                } else if !screen.loaded() {
                    "Loading…"
                } else if screen.dirty() {
                    "Unsaved"
                } else {
                    "Saved"
                },
                detail: screen
                    .error
                    .as_deref()
                    .unwrap_or("Save the configuration; restart the server to apply changes"),
                tone: if screen.error.is_some() {
                    Tone::Error
                } else if screen.dirty() {
                    Tone::Warning
                } else {
                    Tone::Muted
                },
                enabled: screen.loaded() && screen.dirty() && !screen.saving && !screen.loading,
            };
            if widgets::save_controls(ui, &status, "Save mocks") {
                state.actions.push(Action::Save);
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("127.0.0.1");
        let editable = screen.loaded() && !screen.loading;
        let port = ui.add_enabled(
            editable,
            egui::DragValue::new(&mut screen.form.port)
                .range(0..=65535)
                .prefix("Port "),
        );
        widgets::name(&port, "Mock port");
        edited |= port.changed();
        if let Some(run) = &screen.run {
            if ui
                .add_enabled(!run.stopping, egui::Button::new("Stop mock server"))
                .clicked()
            {
                state.actions.push(Action::Mock(MockAction::Stop));
            }
            ui.label(if run.stopping {
                "Stopping…"
            } else {
                run.address.as_deref().unwrap_or("Starting…")
            });
            if screen
                .baseline
                .as_ref()
                .is_some_and(|base| *base != run.config)
            {
                ui.label("Restart to apply");
            }
        } else if ui
            .add_enabled(
                screen.loaded() && !screen.dirty() && !screen.saving && !screen.loading,
                egui::Button::new("Start mock server"),
            )
            .on_disabled_hover_text("Save changes before starting")
            .clicked()
        {
            state.actions.push(Action::Mock(MockAction::Start));
        }
        if ui
            .add_enabled(
                !screen.saving && !screen.loading,
                egui::Button::new("Reload mocks"),
            )
            .clicked()
        {
            state.actions.push(Action::Mock(MockAction::Reload));
        }
    });
    if let Some(notice) = &screen.notice {
        ui.label(notice);
    }
    ui.separator();
    let loaded = screen.loaded() && !screen.loading;
    ui.add_enabled_ui(loaded, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, route) in screen.form.routes.iter().enumerate() {
                    let response = ui.add(
                        egui::Button::selectable(
                            screen.selected == Some(index),
                            format!(
                                "{} {}{}",
                                route.method,
                                route.path,
                                if route.enabled { "" } else { " (disabled)" }
                            ),
                        )
                        .truncate(),
                    );
                    widgets::name(&response, &format!("Mock route {}", index + 1));
                    if response.clicked() {
                        screen.selected = Some(index);
                    }
                }
                if ui.button("Add mock route").clicked() {
                    screen.form.routes.push(MockRoute::default());
                    screen.selected = Some(screen.form.routes.len() - 1);
                    edited = true;
                }
                let Some(index) = screen
                    .selected
                    .filter(|index| *index < screen.form.routes.len())
                else {
                    return;
                };
                let route = &mut screen.form.routes[index];
                ui.separator();
                let mut remove = false;
                ui.horizontal(|ui| {
                    edited |= ui.checkbox(&mut route.enabled, "Route enabled").changed();
                    ui.label("Status");
                    let response = ui.add(egui::DragValue::new(&mut route.status).range(200..=599));
                    widgets::name(&response, "Mock response status");
                    edited |= response.changed();
                    remove = widgets::remove_button(
                        ui,
                        ui.scope_id().with(("route", index)),
                        "mock route",
                        24.0,
                    );
                });
                ui.horizontal(|ui| {
                    edited |= text(ui, "Mock method", &mut route.method, 85.0);
                    edited |= text(ui, "Mock path", &mut route.path, ui.available_width());
                });
                ui.label("Response headers");
                let mut remove_header = None;
                ui.push_id(("mock headers", index), |ui| {
                    for (row, header) in route.headers.iter_mut().enumerate() {
                        ui.push_id(row, |ui| {
                            ui.horizontal(|ui| {
                                let response = ui.checkbox(&mut header.is_enabled, "");
                                widgets::name(
                                    &response,
                                    &format!("Mock header {} enabled", row + 1),
                                );
                                edited |= response.changed();
                                edited |= text(
                                    ui,
                                    &format!("Mock header {} name", row + 1),
                                    &mut header.name,
                                    160.0,
                                );
                                edited |= text(
                                    ui,
                                    &format!("Mock header {} value", row + 1),
                                    &mut header.value,
                                    (ui.available_width() - 40.0).max(80.0),
                                );
                                if widgets::remove_button(
                                    ui,
                                    ui.scope_id().with("remove"),
                                    &format!("mock header {}", row + 1),
                                    24.0,
                                ) {
                                    remove_header = Some(row);
                                }
                            });
                        });
                    }
                });
                if let Some(row) = remove_header {
                    route.headers.remove(row);
                    edited = true;
                }
                if ui.button("Add mock header").clicked() {
                    route.headers.push(ParamItem::new("", ""));
                    edited = true;
                }
                ui.label("Response body");
                let response = ui.add(
                    egui::TextEdit::multiline(&mut route.body)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10),
                );
                widgets::name(&response, "Mock response body");
                edited |= response.changed();
                if remove {
                    screen.form.routes.remove(index);
                    screen.selected = None;
                    edited = true;
                }
            });
    });
    if edited {
        state.actions.push(Action::Mock(MockAction::Edited));
    }
}
