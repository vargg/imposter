//! The Variables screen: globals, environments and collection variables, edited one scope at a
//! time with its own unsaved-changes guard.

use egui::{Align, Layout, RichText, Ui};

use crate::app::{Action, State, VariablesAction};
use crate::theme::{PALETTE, Tone};
use crate::vars::{self, Scope};
use crate::widgets::{name, tone_text};

fn scope_label(state: &State, scope: Scope) -> String {
    match scope {
        Scope::Global => "Global".to_owned(),
        Scope::Environment(id) => state.environments.iter().find(|e| e.id == id).map_or_else(
            || "Environment".to_owned(),
            |e| format!("Environment · {}", e.name),
        ),
        Scope::Collection(id) => state.collections.iter().find(|c| c.id == id).map_or_else(
            || "Collection".to_owned(),
            |c| format!("Collection · {}", c.name),
        ),
    }
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    ui.heading("Variable scopes");
    ui.label(tone_text("Reusable values for your requests.", Tone::Muted));
    ui.add_space(12.0);
    let mut actions = Vec::new();
    let current = state.variables.scope;
    ui.horizontal(|ui| {
        let caption = ui.label("Editing");
        let combo = egui::ComboBox::from_id_salt("variables scope")
            .width(260.0)
            .selected_text(scope_label(state, current))
            .show_ui(ui, |ui| {
                let mut pick = |ui: &mut Ui, scope: Scope, label: String| {
                    if ui.selectable_label(scope == current, label).clicked() {
                        actions.push(Action::Variables(VariablesAction::SelectScope(scope)));
                    }
                };
                pick(ui, Scope::Global, "Global".into());
                for environment in &state.environments {
                    pick(
                        ui,
                        Scope::Environment(environment.id),
                        format!("Environment · {}", environment.name),
                    );
                }
                for collection in &state.collections {
                    pick(
                        ui,
                        Scope::Collection(collection.id),
                        format!("Collection · {}", collection.name),
                    );
                }
            });
        combo.response.labelled_by(caption.id);
        ui.menu_button("Manage environments", |ui| {
            if ui.button("New").clicked() {
                actions.push(Action::Variables(VariablesAction::NewEnvironment));
            }
            if let Scope::Environment(id) = current {
                if ui.button("Rename").clicked() {
                    actions.push(Action::Variables(VariablesAction::RenameEnvironment(id)));
                }
                if ui.button("Delete").clicked() {
                    actions.push(Action::Variables(VariablesAction::DeleteEnvironment(id)));
                }
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Reload").clicked() {
                actions.push(Action::Variables(VariablesAction::Reload));
            }
            let dirty = state.variables.dirty();
            if ui
                .add_enabled(dirty, egui::Button::new("Save Variables"))
                .clicked()
            {
                actions.push(Action::Variables(VariablesAction::Save));
            }
        });
    });
    let active = state
        .active_environment
        .and_then(|id| state.environments.iter().find(|e| e.id == id))
        .map_or("Global (no environment)".to_owned(), |e| e.name.clone());
    ui.label(tone_text(
        format!(
            "Editing variables for {} · Active execution environment: {active}",
            scope_label(state, current)
        ),
        Tone::Muted,
    ));
    if let Some((status, tone)) = &state.variables.status {
        ui.label(tone_text(status.as_str(), *tone));
    }
    ui.separator();
    if !state.variables.loaded {
        ui.label(tone_text("Loading…", Tone::Muted));
        state.actions.extend(actions);
        return;
    }
    let mut edited = false;
    let highlight = state.variables.highlight.take();
    egui::Frame::new()
        .fill(PALETTE.surface)
        .stroke(egui::Stroke::new(1.0, PALETTE.border))
        .corner_radius(12)
        .inner_margin(14)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let height = ui.spacing().interact_size.y;
                    let available = ui.available_width() - 40.0;
                    let name_width = (available * 0.35).clamp(140.0, 320.0);
                    let value_width = (available - name_width).max(160.0);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [name_width, height],
                            egui::Label::new(tone_text("Variable name", Tone::Muted)),
                        );
                        ui.add_sized(
                            [value_width, height],
                            egui::Label::new(tone_text("Value", Tone::Muted)),
                        );
                    });
                    let mut remove = None;
                    for (index, row) in state.variables.rows.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let field = ui.add_sized(
                                [name_width, height],
                                egui::TextEdit::singleline(&mut row.name).hint_text("name"),
                            );
                            name(&field, &format!("Variable {} name", index + 1));
                            if highlight.as_deref() == Some(row.name.as_str()) {
                                field.scroll_to_me(Some(Align::Center));
                                field.request_focus();
                            }
                            let value = ui.add_sized(
                                [value_width, height],
                                egui::TextEdit::singleline(&mut row.value).hint_text("value"),
                            );
                            name(&value, &format!("Variable {} value", index + 1));
                            edited |= field.changed() || value.changed();
                            if ui
                                .add_sized([24.0, height], egui::Button::new("×"))
                                .on_hover_text("Remove variable")
                                .clicked()
                            {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        state.variables.rows.remove(index);
                        edited = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Add Variable").clicked() {
                            state.variables.rows.push(vars::Row::default());
                        }
                        if !state.variables.rows.is_empty() {
                            if state.variables.confirm_clear {
                                ui.label(tone_text("Remove all variables?", Tone::Warning));
                                if ui.button(RichText::new("Remove all")).clicked() {
                                    state.variables.rows.clear();
                                    state.variables.confirm_clear = false;
                                    edited = true;
                                }
                                if ui.button("Keep").clicked() {
                                    state.variables.confirm_clear = false;
                                }
                            } else if ui.button("Clear All").clicked() {
                                state.variables.confirm_clear = true;
                            }
                        }
                    });
                });
        });
    if edited {
        actions.push(Action::Variables(VariablesAction::Edited));
    }

    state.actions.extend(actions);
}
