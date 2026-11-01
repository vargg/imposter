//! The Variables screen: globals, environments and collection variables, edited one scope at a
//! time with its own unsaved-changes guard.

use egui::{Align, Layout, RichText, Ui};

use crate::app::{Action, State, VariablesAction};
use crate::icons::{self, Icon};
use crate::theme::{Tone, palette};
use crate::vars::{self, Scope};
use crate::widgets::{SaveStatus, env_option_text, env_text, name, save_controls, tone_text};
use imposter_app::core::model::EnvColor;

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

fn scope_color(state: &State, scope: Scope) -> Option<EnvColor> {
    match scope {
        Scope::Environment(id) => state
            .environments
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.color),
        Scope::Global | Scope::Collection(_) => None,
    }
}

/// What the scope is for, in a sentence, and where it stands in precedence (the active
/// environment, then the collection, then Global). The picker above names the scope.
pub fn explanation(state: &State, scope: Scope) -> (String, &'static str) {
    match scope {
        Scope::Global => (
            "Used by every request.".to_owned(),
            "A collection's variables and the active environment override these.",
        ),
        Scope::Environment(id) => {
            let name = state
                .environments
                .iter()
                .find(|e| e.id == id)
                .map_or("This environment", |e| e.name.as_str());
            let usage = if state.active_environment == Some(id) {
                format!("{name} is the active environment: requests use these now.")
            } else {
                format!("{name} is not active: requests use these only while it is.")
            };
            (
                usage,
                "While active, these override the collection's variables and Global.",
            )
        }
        Scope::Collection(id) => (
            format!(
                "Used by the requests in {}.",
                state
                    .collections
                    .iter()
                    .find(|c| c.id == id)
                    .map_or("this collection", |c| c.name.as_str())
            ),
            "The active environment overrides these, and they override Global.",
        ),
    }
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let mut actions = Vec::new();
    let current = state.variables.scope;
    ui.horizontal(|ui| {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text = palette().text;
        ui.label(tone_text("Editing", Tone::Muted));
        let combo = egui::ComboBox::from_id_salt("variables scope")
            .width(260.0)
            .selected_text(env_text(
                &scope_label(state, current),
                scope_color(state, current),
                font.clone(),
                text,
            ))
            .show_ui(ui, |ui| {
                let mut pick = |ui: &mut Ui, scope: Scope, label: String| {
                    let shown =
                        env_option_text(&label, scope_color(state, scope), font.clone(), text);
                    let option = crate::widgets::option(ui, scope == current, shown);
                    name(&option, &label);
                    if option.clicked() {
                        actions.push(Action::Variables(VariablesAction::SelectScope(scope)));
                    }
                };
                pick(ui, Scope::Global, "Global".into());
                for environment in &state.environments {
                    let active = if state.active_environment == Some(environment.id) {
                        " (active)"
                    } else {
                        ""
                    };
                    pick(
                        ui,
                        Scope::Environment(environment.id),
                        format!("Environment · {}{active}", environment.name),
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
        // Named for what it chooses; the caption says what the screen does with it.
        name(&combo.response, "Variable scope");
        let new = ui
            .button("+ Environment")
            .on_hover_text("Create an environment");
        name(&new, "New environment");
        if new.clicked() {
            actions.push(Action::Variables(VariablesAction::NewEnvironment));
        }
        let has_rows = !state.variables.rows.is_empty();
        icons::menu(ui, Icon::More, "Variable actions", |ui| {
            if let Scope::Environment(id) = current {
                if ui.button("Edit name and color…").clicked() {
                    actions.push(Action::Variables(VariablesAction::RenameEnvironment(id)));
                }
                if ui.button("Delete environment…").clicked() {
                    actions.push(Action::Variables(VariablesAction::DeleteEnvironment(id)));
                }
                ui.separator();
            }
            // Asks below the rows before anything is removed.
            if ui
                .add_enabled(has_rows, egui::Button::new("Clear all variables…"))
                .clicked()
            {
                state.variables.confirm_clear = true;
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let screen = &state.variables;
            let dirty = screen.dirty();
            let (text, tone) = match &screen.status {
                Some((text, tone)) => (text.as_str(), *tone),
                None if !screen.loaded => ("Loading…", Tone::Muted),
                None if dirty => ("Unsaved", Tone::Warning),
                None => ("Saved", Tone::Muted),
            };
            let detail = match text {
                "Unsaved" => "Unsaved variable changes",
                "Saved" => "The variables are saved",
                _ => "",
            };
            let status = SaveStatus {
                text,
                detail,
                tone,
                enabled: dirty,
            };
            if save_controls(ui, &status, "Save variables") {
                actions.push(Action::Variables(VariablesAction::Save));
            }
            let revert = ui
                .add_enabled_ui(dirty, |ui| {
                    icons::button(ui, Icon::Revert, "Revert unsaved changes", false)
                })
                .inner;
            if revert.clicked() {
                actions.push(Action::Variables(VariablesAction::Revert));
            }
        });
    });
    ui.separator();
    if !state.variables.loaded {
        ui.label(tone_text("Loading…", Tone::Muted));
        state.actions.extend(actions);
        return;
    }
    let mut edited = false;
    let highlight = state.variables.highlight.take();
    let (usage, precedence) = explanation(state, current);
    egui::Frame::new()
        .fill(palette().surface)
        .stroke(egui::Stroke::new(1.0, palette().border))
        .corner_radius(12)
        .inner_margin(14)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.label(usage);
                    ui.add(egui::Label::new(tone_text(precedence, Tone::Muted)).wrap());
                    ui.add_space(8.0);
                    let height = ui.spacing().interact_size.y;
                    // Room for the saved values and remove buttons after the fields.
                    let available = ui.available_width() - 76.0;
                    let name_width = (available * 0.35).clamp(140.0, 320.0);
                    let value_width = (available - name_width).max(160.0);
                    ui.horizontal(|ui| {
                        crate::widgets::heading_cell(ui, name_width, "Variable name");
                        crate::widgets::heading_cell(ui, value_width, "Value");
                    });
                    let mut remove = None;
                    for (index, row) in state.variables.rows.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let field = ui.add_sized(
                                [name_width, height],
                                egui::TextEdit::singleline(&mut row.name)
                                    .id_salt(("variable", row.key, "name"))
                                    .hint_text("name"),
                            );
                            name(&field, &format!("Variable {} name", index + 1));
                            if highlight.as_deref() == Some(row.name.as_str()) {
                                field.scroll_to_me(Some(Align::Center));
                                field.request_focus();
                            }
                            let value = ui.add_sized(
                                [value_width, height],
                                egui::TextEdit::singleline(&mut row.value)
                                    .id_salt(("variable", row.key, "value"))
                                    .hint_text("value"),
                            );
                            name(&value, &format!("Variable {} value", index + 1));
                            edited |= field.changed() || value.changed();
                            edited |= saved_values(ui, row, index, height);
                            let what = format!("variable {}", index + 1);
                            let id = egui::Id::unique(("remove variable", row.key));
                            if crate::widgets::remove_button(ui, id, &what, height) {
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
                            state.variables.rows.push(vars::Row::new("", ""));
                        }
                        if state.variables.confirm_clear && !state.variables.rows.is_empty() {
                            ui.label(tone_text("Remove all variables?", Tone::Warning));
                            if ui.button(RichText::new("Remove all")).clicked() {
                                state.variables.rows.clear();
                                state.variables.confirm_clear = false;
                                edited = true;
                            }
                            if ui.button("Keep").clicked() {
                                state.variables.confirm_clear = false;
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

/// Wide enough for a value, "current" and the remove button.
const SAVED_MENU_WIDTH: f32 = 280.0;

/// The row's saved values: pick one as the current value, save the current one, or remove
/// one. Returns whether the row changed. The menu stays open while it is used, since
/// removing takes two clicks.
fn saved_values(ui: &mut Ui, row: &mut vars::Row, index: usize, height: f32) -> bool {
    let label = format!("Variable {} saved values", index + 1);
    let hint = match row.saved.len() {
        0 => "Saved values: none yet".to_owned(),
        count => format!("Saved values: {count}"),
    };
    let button = icons::count_button(ui, Icon::Bookmark, &label, &hint, row.saved.len());
    let mut changed = false;
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(SAVED_MENU_WIDTH);
            ui.set_max_width(SAVED_MENU_WIDTH);
            if row.saved.is_empty() {
                ui.label(tone_text("No saved values yet", Tone::Muted));
            }
            let mut remove = None;
            for (position, value) in row.saved.iter().enumerate() {
                ui.horizontal(|ui| {
                    let text = crate::widgets::saved_label(value, false);
                    let current = *value == row.value;
                    // The value on the left, "current" after it, and × at the far right, so a
                    // click meant for the value never lands on remove.
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        &text,
                        0.0,
                        egui::TextFormat::simple(
                            egui::TextStyle::Monospace.resolve(ui.style()),
                            palette().text,
                        ),
                    );
                    if current {
                        job.append(
                            "current",
                            8.0,
                            egui::TextFormat::simple(
                                egui::TextStyle::Small.resolve(ui.style()),
                                palette().text_muted,
                            ),
                        );
                    }
                    let width = ui.available_width() - height - ui.spacing().item_spacing.x;
                    let choice = ui.add_sized(
                        [width, height],
                        egui::Button::selectable(current, (job, egui::Atom::grow()))
                            .stroke(egui::Stroke::NONE)
                            .truncate(),
                    );
                    if current {
                        crate::widgets::selection_bar(ui, choice.rect);
                    }
                    name(
                        &choice,
                        &format!("Use saved value {}: {text}", position + 1),
                    );
                    let choice = choice.on_hover_text(if current {
                        "The current value"
                    } else {
                        "Make this the current value"
                    });
                    if choice.clicked() && !current {
                        row.value.clone_from(value);
                        changed = true;
                    }
                    let id = egui::Id::unique(("remove saved value", row.key, position));
                    let what = format!("saved value {}", position + 1);
                    if crate::widgets::remove_button(ui, id, &what, height) {
                        remove = Some(position);
                    }
                });
            }
            if let Some(position) = remove {
                row.saved.remove(position);
                changed = true;
            }
            if !row.saved.is_empty() {
                ui.separator();
            }
            let can_save = !row.saved.contains(&row.value);
            let save = ui.add_enabled(
                can_save,
                egui::Button::selectable(false, "+ Save current value").stroke(egui::Stroke::NONE),
            );
            name(&save, "Save current value");
            let save = save.on_disabled_hover_text("The current value is already saved");
            if save.clicked() {
                row.save_current();
                changed = true;
            }
        });
    changed
}
