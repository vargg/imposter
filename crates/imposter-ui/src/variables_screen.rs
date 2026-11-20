//! The Variables screen: globals, environments and collection variables, edited one scope at a
//! time with its own unsaved-changes guard.

use egui::{Align, Layout, Response, Ui, UiBuilder, vec2};

use crate::app::{Action, State, VariablesAction};
use crate::icons::{self, Icon};
use crate::theme::{Tone, palette};
use crate::vars::{self, Choice, Scope};
use crate::widgets::{
    REMOVE_WIDTH, SaveStatus, SaveWord, Variant, append_row, blank_cell, button, env_option_text,
    env_text, name, remove_button, save_controls, saved_label, screen_toolbar, table_text,
    tone_text,
};
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
            if state.globals_shared {
                "Used by every request. Saved to globals.yaml in the collections folder."
            } else {
                "Used by every request. globals.yaml lists the names; the values stay on this computer."
            }
            .to_owned(),
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
    let mut saves = Vec::new();
    screen_toolbar(
        ui,
        "Variables",
        |ui| {
            let font = egui::TextStyle::Button.resolve(ui.style());
            let text = palette().text;
            ui.label(tone_text("Editing", Tone::Muted));
            let gap = crate::theme::spacing().sm;
            let reserve =
                crate::widgets::action_width(ui, "+ Environment") + icons::size() + 2.0 * gap;
            let label = scope_label(state, current);
            let width = (ui.available_width() - reserve).clamp(100.0, 260.0);
            let combo = crate::widgets::toolbar_combo(
                ui,
                "variables scope",
                width,
                env_text(&label, scope_color(state, current), font.clone(), text),
                |ui| {
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
                },
            );
            // Named for what it chooses; the caption says what the screen does with it.
            name(&combo, "Variable scope");
            combo.on_hover_text(label);
            let new = button(ui, Variant::Secondary, None, "+ Environment")
                .on_hover_text("Create an environment");
            name(&new, "New environment");
            if new.clicked() {
                actions.push(Action::Variables(VariablesAction::NewEnvironment));
            }
            let has_rows = !state.variables.rows.is_empty();
            icons::menu(ui, Icon::More, "Variable actions", |ui| {
                if current == Scope::Global {
                    let mut share = state.globals_shared;
                    if crate::widgets::checkbox(ui, &mut share, "Share values").clicked() {
                        actions.push(Action::Variables(VariablesAction::ShareGlobals(share)));
                    }
                    ui.separator();
                }
                if let Scope::Environment(id) = current {
                    if button(ui, Variant::Ghost, None, "Edit name and color…").clicked() {
                        actions.push(Action::Variables(VariablesAction::RenameEnvironment(id)));
                    }
                    if button(ui, Variant::Ghost, None, "Delete environment…").clicked() {
                        actions.push(Action::Variables(VariablesAction::DeleteEnvironment(id)));
                    }
                    ui.separator();
                }
                // Asks first, naming how many go.
                if ui
                    .add_enabled_ui(has_rows, |ui| {
                        button(ui, Variant::Ghost, None, "Clear all variables…")
                    })
                    .inner
                    .clicked()
                {
                    actions.push(Action::Variables(VariablesAction::ClearAll));
                }
            });
        },
        |ui| {
            let screen = &state.variables;
            let dirty = screen.dirty();
            let word = match screen.status {
                Some(word) => word,
                None if !screen.loaded => SaveWord::Loading,
                None if screen.is_saving() => SaveWord::Saving,
                None if dirty => SaveWord::Unsaved,
                None => SaveWord::Saved,
            };
            let detail = match word {
                SaveWord::Unsaved => "Unsaved variable changes",
                SaveWord::Saved => "The variables are saved",
                _ => "",
            };
            let status = SaveStatus {
                word,
                detail,
                enabled: dirty,
            };
            if save_controls(ui, &status, "Save variables") {
                saves.push(Action::Variables(VariablesAction::Save));
            }
            let revert = ui
                .add_enabled_ui(dirty, |ui| {
                    icons::button(ui, Icon::Revert, "Revert unsaved changes", false)
                })
                .inner;
            if revert.clicked() {
                saves.push(Action::Variables(VariablesAction::Revert));
            }
        },
    );
    actions.extend(saves);
    if state.variables_changed_on_disk {
        use crate::app::DiskChoice;
        match crate::workspace::notice_banner(
            ui,
            "These variables changed on disk. Your unsaved rows are here.",
            &[
                ("Keep mine", "Keep my rows; Save writes them over the file"),
                ("Reload", "Reload the variables from the file"),
            ],
        ) {
            Some(0) => actions.push(Action::Disk(DiskChoice::KeepVariables)),
            Some(_) => actions.push(Action::Disk(DiskChoice::ReloadVariables)),
            None => {}
        }
    }
    if !state.variables.loaded {
        ui.label(tone_text("Loading…", Tone::Muted));
        state.actions.extend(actions);
        return;
    }
    let mut edited = false;
    let highlight = state.variables.highlight.take();
    let (usage, precedence) = explanation(state, current);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            crate::widgets::scroll_view_bounds(ui);
            ui.label(usage);
            ui.add(egui::Label::new(tone_text(precedence, Tone::Muted)).wrap());
            ui.add_space(crate::theme::spacing().sm);
            let height = ui.spacing().interact_size.y;
            let gap = ui.spacing().item_spacing.x;
            let available = ui.available_width() - REMOVE_WIDTH - 2.0 * gap;
            let name_width = (available * 0.35).clamp(140.0, 320.0);
            let value_width = (available - name_width).max(160.0);
            let widths = (name_width, value_width, height);
            ui.horizontal(|ui| {
                crate::widgets::heading_cell(ui, name_width, "Variable name");
                crate::widgets::heading_cell(ui, value_width, "Values");
            });
            let mut remove = None;
            for (index, row) in state.variables.rows.iter_mut().enumerate() {
                let (changed, removed) =
                    variable_row(ui, row, index + 1, widths, highlight.as_deref());
                edited |= changed;
                if removed {
                    remove = Some(index);
                }
            }
            if let Some(index) = remove {
                state.variables.rows.remove(index);
                edited = true;
            }
            let id = ui.make_persistent_id(("new variable", current));
            let mut row = ui.data_mut(|data| {
                data.get_temp_mut_or_insert_with(id, || vars::VariableRow::new(""))
                    .clone()
            });
            ui.horizontal(|ui| {
                variable_name(ui, &mut row, "New variable name", name_width);
                blank_cell(ui, value_width);
                blank_cell(ui, REMOVE_WIDTH);
            });
            if !row.name.is_empty() {
                ui.data_mut(|data| data.remove::<vars::VariableRow>(id));
                edited |= append_row(ui, &mut state.variables.rows, row);
            }
            if imposter_app::core::variables::SecretScope::of_scope(current).is_some() {
                edited |= secrets_section(
                    ui,
                    &mut state.variables.secrets,
                    widths,
                    highlight.as_deref(),
                );
                let requests = match current {
                    Scope::Environment(id) => {
                        state.environments.iter().find(|e| e.id == id).map_or_else(
                            || "requests sent with this environment".to_owned(),
                            |e| format!("requests sent with {}", e.name),
                        )
                    }
                    _ => "requests sent without an environment".to_owned(),
                };
                cookies_section(
                    ui,
                    &state.variables.cookies,
                    widths,
                    &requests,
                    &mut actions,
                );
            }
        });
    if edited {
        actions.push(Action::Variables(VariablesAction::Edited));
    }

    state.actions.extend(actions);
}

fn variable_row(
    ui: &mut Ui,
    row: &mut vars::VariableRow,
    number: usize,
    (name_width, value_width, height): (f32, f32, f32),
    highlight: Option<&str>,
) -> (bool, bool) {
    ui.horizontal_top(|ui| {
        let field = variable_name(ui, row, &format!("Variable {number} name"), name_width);
        if highlight == Some(row.name.as_str()) {
            field.scroll_to_me(Some(Align::Center));
            field.request_focus();
        }

        let values = ui.allocate_ui_with_layout(
            vec2(value_width, height),
            Layout::left_to_right(Align::Center).with_main_wrap(true),
            |ui| {
                ui.set_min_width(value_width);
                values_editor(ui, row, number, value_width, height)
            },
        );
        let id = egui::Id::unique(("remove variable", row.key));
        let removed = remove_button(ui, id, &format!("variable {number}"), height);
        (field.changed() || values.inner, removed)
    })
    .inner
}

fn variable_name(ui: &mut Ui, row: &mut vars::VariableRow, label: &str, width: f32) -> Response {
    table_text(
        ui,
        &mut row.name,
        egui::Id::unique(("variable name", row.key)),
        label,
        width,
        "name",
        false,
    )
}

/// The scope's secrets under its variables: values masked, each with an eye to show it for
/// the session, never saved values. Returns whether a row changed.
fn secrets_section(
    ui: &mut Ui,
    rows: &mut Vec<vars::Row>,
    (name_width, value_width, height): (f32, f32, f32),
    highlight: Option<&str>,
) -> bool {
    let mut edited = false;
    ui.add_space(16.0);
    ui.label(egui::RichText::new("Secrets").font(crate::theme::semibold(13.0)));
    ui.add(
        egui::Label::new(tone_text(
            "Used as {{secret:name}}. Kept only on this computer: never exported, and removed from History.",
            Tone::Muted,
        ))
        .wrap(),
    );
    ui.add_space(4.0);
    let eye = icons::size() + ui.spacing().item_spacing.x;
    let value_width = (value_width - eye).max(120.0);
    ui.horizontal(|ui| {
        crate::widgets::heading_cell(ui, name_width, "Secret name");
        crate::widgets::heading_cell(ui, value_width, "Value");
    });
    let mut remove = None;
    for (index, row) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let field = table_text(
                ui,
                &mut row.name,
                egui::Id::unique(("secret", row.key, "name")),
                &format!("Secret {} name", index + 1),
                name_width,
                "name",
                false,
            );
            if highlight.and_then(imposter_app::core::variables::secret_name)
                == Some(row.name.as_str())
            {
                field.scroll_to_me(Some(Align::Center));
                field.request_focus();
            }
            // Shown or not is remembered for the session only, never stored.
            let shown_id = ui.make_persistent_id(("secret shown", row.key));
            let mut shown: bool = ui.data(|d| d.get_temp(shown_id)).unwrap_or(false);
            let value = table_text(
                ui,
                &mut row.value,
                egui::Id::unique(("secret", row.key, "value")),
                &format!("Secret {} value", index + 1),
                value_width,
                "value",
                !shown,
            );
            let (icon, action) = if shown {
                (Icon::EyeOff, format!("Hide secret {}", index + 1))
            } else {
                (Icon::Eye, format!("Show secret {}", index + 1))
            };
            if icons::button(ui, icon, &action, false).clicked() {
                shown = !shown;
            }
            ui.data_mut(|d| d.insert_temp(shown_id, shown));
            edited |= field.changed() || value.changed();
            let what = format!("secret {}", index + 1);
            let id = egui::Id::unique(("remove secret", row.key));
            if remove_button(ui, id, &what, height) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        rows.remove(index);
        edited = true;
    }
    let id = ui.make_persistent_id("new secret");
    let mut row = ui.data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || vars::Row::new("", ""))
            .clone()
    });
    ui.horizontal(|ui| {
        table_text(
            ui,
            &mut row.name,
            egui::Id::unique(("secret", row.key, "name")),
            "New secret name",
            name_width,
            "name",
            false,
        );
        table_text(
            ui,
            &mut row.value,
            egui::Id::unique(("secret", row.key, "value")),
            "New secret value",
            value_width,
            "value",
            true,
        );
        blank_cell(ui, icons::size());
        blank_cell(ui, REMOVE_WIDTH);
    });
    if !row.name.is_empty() || !row.value.is_empty() {
        ui.data_mut(|data| data.remove::<vars::Row>(id));
        edited |= append_row(ui, rows, row);
    }
    edited
}

/// The cookies servers set for the scope's requests: name and value (masked, with an eye),
/// where and until when each is sent, and a × that asks before removing it. Not part of
/// the page's edits: a send changes them, and removing one is written at once.
fn cookies_section(
    ui: &mut Ui,
    cookies: &[imposter_app::http::cookies::StoredCookie],
    (name_width, value_width, height): (f32, f32, f32),
    requests: &str,
    actions: &mut Vec<Action>,
) {
    ui.add_space(16.0);
    ui.label(egui::RichText::new("Cookies").font(crate::theme::semibold(13.0)));
    ui.add(
        egui::Label::new(tone_text(
            format!(
                "Set by servers for {requests}, and sent back to them. Kept only on this computer."
            ),
            Tone::Muted,
        ))
        .wrap(),
    );
    ui.add_space(4.0);
    if cookies.is_empty() {
        ui.label(tone_text("No cookies yet.", Tone::Muted));
        return;
    }
    let eye = icons::size() + ui.spacing().item_spacing.x;
    let value_width = (value_width - eye).max(120.0);
    for (index, cookie) in cookies.iter().enumerate() {
        ui.horizontal(|ui| {
            // Its text names it to screen readers; the fields after it say "cookie".
            ui.add_sized(
                [name_width, height],
                egui::Label::new(cookie.name.as_str()).truncate(),
            );
            let shown_id = ui.make_persistent_id(("cookie shown", index, &cookie.name));
            let mut shown: bool = ui.data(|d| d.get_temp(shown_id)).unwrap_or(false);
            let mut value = cookie.value.as_str();
            let field = ui.add_sized(
                [value_width, height],
                egui::TextEdit::singleline(&mut value)
                    .id_salt(("cookie", index, "value"))
                    .password(!shown),
            );
            name(&field, &format!("Cookie {} value", cookie.name));
            let (icon, action) = if shown {
                (Icon::EyeOff, format!("Hide cookie {}", cookie.name))
            } else {
                (Icon::Eye, format!("Show cookie {}", cookie.name))
            };
            if icons::button(ui, icon, &action, false).clicked() {
                shown = !shown;
            }
            ui.data_mut(|d| d.insert_temp(shown_id, shown));
            let remove = ui
                .scope(|ui| {
                    ui.spacing_mut().button_padding.x = 4.0;
                    ui.add_sized([24.0, height], egui::Button::new("×"))
                })
                .inner
                .on_hover_text("Remove this cookie…");
            name(&remove, &format!("Remove cookie {}", cookie.name));
            if remove.clicked() {
                actions.push(Action::Variables(VariablesAction::RemoveCookie(index)));
            }
        });
        ui.add(egui::Label::new(tone_text(cookie_place(cookie), Tone::Muted)).truncate());
        ui.add_space(4.0);
    }
    if ui.button("Clear cookies…").clicked() {
        actions.push(Action::Variables(VariablesAction::ClearCookies));
    }
}

/// Where and until when a cookie is sent: "api.test and its subdomains · /admin · HTTPS
/// only · until 12 Dec 2026, 10:00".
pub fn cookie_place(cookie: &imposter_app::http::cookies::StoredCookie) -> String {
    let mut parts = vec![if cookie.host_only {
        cookie.domain.clone()
    } else {
        format!("{} and its subdomains", cookie.domain)
    }];
    parts.push(cookie.path.clone());
    if cookie.secure {
        parts.push("HTTPS only".to_owned());
    }
    parts.push(match cookie.expires {
        None => "until cleared".to_owned(),
        Some(seconds) => format!(
            "until {}",
            crate::viewer::local_time(imposter_app::core::execution::Timestamp(
                seconds.saturating_mul(1000)
            ))
        ),
    });
    parts.join(" · ")
}

#[derive(Clone, Copy)]
enum ChipAction {
    Select,
    Remove,
    Keep,
}

fn values_editor(
    ui: &mut Ui,
    row: &mut vars::VariableRow,
    number: usize,
    line_width: f32,
    height: f32,
) -> bool {
    let mut action = None;
    for (index, value) in row.values().iter().enumerate() {
        let choice = Choice::Ordinary(index);
        let title = format!("Variable {number} value {}", index + 1);
        if let Some(picked) = value_chip(ui, row, choice, &title, value, line_width) {
            action = Some((choice, picked));
        }
    }
    if let Some(value) = row.distinct_capture() {
        let choice = Choice::Capture;
        let title = format!("Variable {number} latest capture");
        if let Some(picked) = value_chip(ui, row, choice, &title, value, line_width) {
            action = Some((choice, picked));
        }
    }
    match action {
        Some((choice, ChipAction::Select)) => row.select(choice),
        Some((choice, ChipAction::Remove)) => row.remove(choice),
        Some((_, ChipAction::Keep)) => row.keep_capture(),
        None => {}
    }

    let composer = ui.add_sized(
        [line_width.min(160.0), height],
        egui::TextEdit::singleline(&mut row.draft)
            .id(egui::Id::unique(("variable draft", row.key)))
            .hint_text("Add value…"),
    );
    name(&composer, &format!("Add value to variable {number}"));
    let entered = composer.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = (composer.has_focus() || composer.lost_focus())
        && ui.input(|i| i.key_pressed(egui::Key::Escape));
    if entered {
        row.add_draft();
        composer.request_focus();
    } else if escape {
        row.draft.clear();
    } else if matches!(action, Some((_, ChipAction::Remove | ChipAction::Keep))) {
        // The old chip may disappear with its menu.
        composer.request_focus();
    }
    action.is_some() || composer.changed() || entered || escape
}

fn value_chip(
    ui: &mut Ui,
    row: &vars::VariableRow,
    choice: Choice,
    title: &str,
    value: &str,
    line_width: f32,
) -> Option<ChipAction> {
    let label = saved_label(value, false);
    let capture = choice == Choice::Capture;
    let text = if capture {
        format!("Latest capture · {label}")
    } else {
        label.clone()
    };
    let height = icons::size();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.clone(), font, palette().text)
            .size()
            .x
    });
    let width = (text_width + 2.0 * ui.spacing().button_padding.x)
        .ceil()
        .clamp(REMOVE_WIDTH, line_width);
    let (_, rect) = ui.allocate_space(vec2(width, height));
    let id = egui::Id::unique(("variable chip", row.key, capture, value));
    let mut chip_ui = ui.new_child(
        UiBuilder::new()
            .scope_id(id)
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let current = row.current() == Some(value);
    let button = egui::Button::selectable(current, egui::RichText::new(text).monospace())
        .small()
        .frame_when_inactive(true)
        .min_size(vec2(width, height))
        .truncate();
    let button = if current {
        button.stroke(egui::Stroke::NONE)
    } else {
        button
    };
    let chip = chip_ui.add_sized([width, height], button);
    name(&chip, &format!("{title}: {label}"));
    let menu_action = egui::accesskit::Action::ShowContextMenu;
    chip.ctx.accesskit_node_builder(chip.id, |node| {
        node.add_action(menu_action);
        if current {
            node.set_description("The current value");
        }
    });
    chip_focus(&chip_ui, &chip);

    let keyboard =
        chip.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
    let assistive = ui.input(|i| {
        i.accesskit_action_requests(chip.id, menu_action)
            .next()
            .is_some()
    });
    let open = if keyboard || assistive || chip.secondary_clicked() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if chip.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let mut action = chip.clicked().then_some(ChipAction::Select);
    egui::Popup::menu(&chip).open_memory(open).show(|ui| {
        ui.set_max_width(280.0);
        ui.add(egui::Label::new(tone_text(&label, Tone::Muted).monospace()).truncate());
        let copy = ui.button("Copy value");
        if keyboard || assistive {
            copy.request_focus();
        }
        if copy.clicked() {
            ui.ctx().copy_text(value.to_owned());
            chip.request_focus();
            ui.close();
        }
        if capture && ui.button("Keep in list").clicked() {
            action = Some(ChipAction::Keep);
            ui.close();
        }
        if ui
            .add_enabled(row.can_remove(), egui::Button::new("Remove from list"))
            .on_disabled_hover_text("Remove the variable to delete its last value")
            .clicked()
        {
            action = Some(ChipAction::Remove);
            ui.close();
        }
    });
    chip.on_hover_text(value);
    action
}

fn chip_focus(ui: &Ui, chip: &Response) {
    if chip.has_focus() {
        ui.painter().rect_stroke(
            chip.rect,
            4.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
}
