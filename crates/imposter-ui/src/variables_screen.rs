//! The Variables screen: globals, environments and collection variables, edited one scope at a
//! time with its own unsaved-changes guard.

use egui::{Align, Layout, Response, Ui, UiBuilder, vec2};

use crate::app::{Action, State, VariablesAction};
use crate::icons::{self, Icon};
use crate::theme::{Tone, palette};
use crate::vars::{self, Scope};
use crate::widgets::{
    REMOVE_WIDTH, SaveStatus, SaveWord, env_option_text, env_text, name, remove_button,
    revealed_remove_button, save_controls, saved_label, tone_text,
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
            if current == Scope::Global {
                let mut share = state.globals_shared;
                if ui.checkbox(&mut share, "Share values").clicked() {
                    actions.push(Action::Variables(VariablesAction::ShareGlobals(share)));
                }
                ui.separator();
            }
            if let Scope::Environment(id) = current {
                if ui.button("Edit name and color…").clicked() {
                    actions.push(Action::Variables(VariablesAction::RenameEnvironment(id)));
                }
                if ui.button("Delete environment…").clicked() {
                    actions.push(Action::Variables(VariablesAction::DeleteEnvironment(id)));
                }
                ui.separator();
            }
            // Asks first, naming how many go.
            if ui
                .add_enabled(has_rows, egui::Button::new("Clear all variables…"))
                .clicked()
            {
                actions.push(Action::Variables(VariablesAction::ClearAll));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                    let gap = ui.spacing().item_spacing.x;
                    let available = ui.available_width() - icons::size() - REMOVE_WIDTH - 3.0 * gap;
                    let name_width = (available * 0.35).clamp(140.0, 320.0);
                    let value_width = (available - name_width).max(160.0);
                    let widths = (name_width, value_width, height);
                    ui.horizontal(|ui| {
                        crate::widgets::heading_cell(ui, name_width, "Variable name");
                        crate::widgets::heading_cell(ui, value_width, "Value");
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
                    ui.horizontal(|ui| {
                        if ui.button("Add Variable").clicked() {
                            state.variables.rows.push(vars::Row::new("", ""));
                        }
                    });
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
        });
    if edited {
        actions.push(Action::Variables(VariablesAction::Edited));
    }

    state.actions.extend(actions);
}

fn variable_row(
    ui: &mut Ui,
    row: &mut vars::Row,
    number: usize,
    (name_width, value_width, height): (f32, f32, f32),
    highlight: Option<&str>,
) -> (bool, bool) {
    ui.vertical(|ui| {
        let (value, mut changed, removed) = ui
            .horizontal(|ui| {
                let field = ui.add_sized(
                    [name_width, height],
                    egui::TextEdit::singleline(&mut row.name)
                        .id_salt(("variable", row.key, "name"))
                        .hint_text("name"),
                );
                name(&field, &format!("Variable {number} name"));
                if highlight == Some(row.name.as_str()) {
                    field.scroll_to_me(Some(Align::Center));
                    field.request_focus();
                }
                let value = ui.add_sized(
                    [value_width, height],
                    egui::TextEdit::singleline(&mut row.value)
                        .id_salt(("variable", row.key, "value"))
                        .hint_text("value"),
                );
                name(&value, &format!("Variable {number} value"));
                let mut changed = field.changed() || value.changed();
                changed |= pin_current(ui, row, number, &value);
                let id = egui::Id::unique(("remove variable", row.key));
                let removed = remove_button(ui, id, &format!("variable {number}"), height);
                (value, changed, removed)
            })
            .inner;

        if !row.saved.is_empty() {
            ui.spacing_mut().indent = value.rect.left() - ui.max_rect().left();
            ui.visuals_mut().indent_has_left_vline = false;
            changed |= saved_strip(ui, row, number, &value);
        }
        (changed, removed)
    })
    .inner
}

fn pin_current(ui: &mut Ui, row: &mut vars::Row, number: usize, value: &Response) -> bool {
    let (enabled, hint) = if row.value.is_empty() {
        (false, "Nothing to pin")
    } else if row.saved.contains(&row.value) {
        (false, "Already pinned")
    } else {
        (true, "Keep this value in the saved list (Enter)")
    };
    let pin = ui
        .add_enabled_ui(enabled, |ui| {
            icons::button_with_hint(
                ui,
                Icon::Plus,
                &format!("Pin variable {number} value"),
                hint,
                false,
            )
        })
        .inner;
    let entered = value.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    (entered || pin.clicked()) && row.save_current()
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
    if !rows.is_empty() {
        ui.horizontal(|ui| {
            crate::widgets::heading_cell(ui, name_width, "Secret name");
            crate::widgets::heading_cell(ui, value_width, "Value");
        });
    }
    let mut remove = None;
    for (index, row) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let field = ui.add_sized(
                [name_width, height],
                egui::TextEdit::singleline(&mut row.name)
                    .id_salt(("secret", row.key, "name"))
                    .hint_text("name"),
            );
            name(&field, &format!("Secret {} name", index + 1));
            if highlight.and_then(imposter_app::core::variables::secret_name)
                == Some(row.name.as_str())
            {
                field.scroll_to_me(Some(Align::Center));
                field.request_focus();
            }
            // Shown or not is remembered for the session only, never stored.
            let shown_id = ui.make_persistent_id(("secret shown", row.key));
            let mut shown: bool = ui.data(|d| d.get_temp(shown_id)).unwrap_or(false);
            let value = ui.add_sized(
                [value_width, height],
                egui::TextEdit::singleline(&mut row.value)
                    .id_salt(("secret", row.key, "value"))
                    .password(!shown)
                    .hint_text("value"),
            );
            name(&value, &format!("Secret {} value", index + 1));
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
    if ui.button("Add Secret").clicked() {
        rows.push(vars::Row::new("", ""));
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

fn saved_strip(ui: &mut Ui, row: &mut vars::Row, number: usize, field: &Response) -> bool {
    let mut changed = false;
    let mut remove = None;
    ui.indent(("saved chips", row.key), |ui| {
        let line_width = ui.available_width();
        ui.horizontal_wrapped(|ui| {
            if !row.saved.contains(&row.value) {
                unpinned_chip(ui, number, &row.value, field);
            }
            for (position, saved) in row.saved.iter().enumerate() {
                let id = egui::Id::unique(("saved chip", row.key, saved));
                let title = format!("{number} saved value {}", position + 1);
                match saved_chip(ui, id, &title, saved, *saved == row.value, line_width) {
                    Some(ChipAction::Choose) => {
                        row.value.clone_from(saved);
                        changed = true;
                    }
                    Some(ChipAction::Remove) => remove = Some(position),
                    None => {}
                }
            }
        });
    });
    if let Some(position) = remove {
        row.saved.remove(position);
        changed = true;
    }
    changed
}

fn unpinned_chip(ui: &mut Ui, number: usize, value: &str, field: &Response) {
    let label = saved_label(value, false);
    let chip = ui.add(
        egui::Button::new(tone_text(&label, Tone::Muted).monospace())
            .small()
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .min_size(vec2(REMOVE_WIDTH, icons::size()))
            .truncate(),
    );
    name(
        &chip,
        &format!("Variable {number} value in use, not pinned: {label}"),
    );
    let rect = chip.rect.shrink(0.5);
    ui.painter().extend(egui::Shape::dashed_line(
        &[
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ],
        egui::Stroke::new(1.0, palette().field_border),
        3.0,
        2.0,
    ));
    chip_focus(ui, &chip);
    let chip = chip.on_hover_text("In use, not pinned — Enter or the + button pins it");
    if chip.clicked() {
        field.request_focus();
    }
}

enum ChipAction {
    Choose,
    Remove,
}

fn saved_chip(
    ui: &mut Ui,
    id: egui::Id,
    title: &str,
    value: &str,
    current: bool,
    line_width: f32,
) -> Option<ChipAction> {
    let label = saved_label(value, false);
    let height = icons::size();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(label.clone(), font, palette().text)
            .size()
            .x
    });
    let remove_space = REMOVE_WIDTH + ui.spacing().item_spacing.x;
    let width = (text_width + 2.0 * ui.spacing().button_padding.x)
        .ceil()
        .clamp(REMOVE_WIDTH, (line_width - remove_space).max(REMOVE_WIDTH));
    // Wrap the pair together so revealing the remove target never moves either control.
    let (_, cell) = ui.allocate_space(vec2(width + remove_space, height));
    let mut ui = ui.new_child(
        UiBuilder::new()
            .scope_id(id)
            .max_rect(cell)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let button = egui::Button::selectable(current, egui::RichText::new(&label).monospace())
        .small()
        .frame_when_inactive(true)
        .min_size(vec2(width, height))
        .truncate();
    let button = if current {
        button.stroke(egui::Stroke::NONE)
    } else {
        button
    };
    let chip = ui.add_sized([width, height], button);
    name(&chip, &format!("Variable {title}: {label}"));
    if current {
        chip.ctx.accesskit_node_builder(chip.id, |node| {
            node.set_description("The current value".to_owned());
        });
    }
    chip_focus(&ui, &chip);

    // egui releases the chip's focus before asking the next rendered widget to take it.
    let reveal = ui.rect_contains_pointer(cell) || chip.has_focus() || chip.lost_focus();
    let removed = revealed_remove_button(&mut ui, id, &format!("variable {title}"), height, reveal);
    let mut action = (chip.clicked() && !current).then_some(ChipAction::Choose);
    if removed {
        action = Some(ChipAction::Remove);
    }
    chip.context_menu(|ui| {
        ui.set_max_width(280.0);
        ui.add(egui::Label::new(tone_text(&label, Tone::Muted).monospace()).truncate());
        if ui.button("Copy value").clicked() {
            ui.ctx().copy_text(value.to_owned());
            ui.close();
        }
        if ui.button("Remove from list").clicked() {
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
