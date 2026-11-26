//! The Variables screen: globals, environments and collection variables, edited one scope at a
//! time with its own unsaved-changes guard.

use egui::{Align, Layout, Response, Ui, UiBuilder, vec2};
use imposter_app::core::variables::{SecretScope, secret_name};

use crate::app::{Action, State, VariablesAction, VariablesTab};
use crate::icons::{self, Icon};
use crate::theme::{self, Tone, palette};
use crate::vars::{self, Choice, Scope};
use crate::widgets::{
    self, ListEnd, ListIcon, REMOVE_WIDTH, SaveStatus, SaveWord, TabSpec, Variant, append_row,
    blank_cell, button, heading_cell, name, remove_button, saved_label, table_text, tone_text,
};
use crate::workspace;

const ACTIVE_HINT: &str = "Active: requests use this environment";
const INACTIVE_NOTE: &str = "Inactive environment. Activate it to use these values in requests.";

struct Columns {
    name: f32,
    value: f32,
    height: f32,
}

impl Columns {
    fn new(ui: &Ui, remove_width: f32) -> Self {
        let available = ui.available_width() - 2.0 * ui.spacing().item_spacing.x;
        let name = ((available - icons::size()) * 0.35).min(480.0);
        Self {
            name,
            value: available - name - remove_width,
            height: theme::spacing().control,
        }
    }

    fn value_with_eye(&self, ui: &Ui) -> f32 {
        self.value - icons::size() - ui.spacing().item_spacing.x
    }

    fn heading_with_help(&self, ui: &mut Ui, heading: &str, help: &str) {
        heading_cell(ui, self.name, heading);
        heading_cell(ui, self.value_with_eye(ui), "Value");
        ui.allocate_ui_with_layout(
            vec2(icons::size(), self.height),
            Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| widgets::help(ui, help),
        );
    }
}

/// What the scope is for and where it stands in precedence.
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
    workspace::list_screen(
        state,
        ui,
        |state, ui| {
            let (width, list) =
                widgets::list_column(ui, "Variable scopes", state.layout.list_width);
            state.layout.list_width = width;
            list
        },
        |state, ui, list| {
            let mut actions = Vec::new();
            let highlight = scope_toolbar(state, ui, &mut actions);
            widgets::scroll_list(list, |ui| scope_list(state, ui, &mut actions));
            ui.add_space(theme::spacing().lg);
            scope_content(state, ui, &mut actions, highlight.as_deref());
            state.actions.extend(actions);
        },
    );
}

fn scope_list(state: &State, ui: &mut Ui, actions: &mut Vec<Action>) {
    let current = state.variables.scope;
    let (global, _) = widgets::list_row(
        ui,
        "Global",
        ListIcon::Glyph(Icon::Globe),
        current == Scope::Global,
        ListEnd::None,
        None,
    );
    select_scope(&global, Scope::Global, actions);

    ui.add_space(theme::spacing().sm);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icons::button(ui, Icon::Plus, "New environment", false).clicked() {
                actions.push(Action::Variables(VariablesAction::NewEnvironment));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                scope_heading(ui, "Environments");
            });
        });
    });
    for environment in &state.environments {
        let scope = Scope::Environment(environment.id);
        ui.push_id(scope, |ui| {
            let active = state.active_environment == Some(environment.id);
            let hint = if active {
                format!("{}\n{ACTIVE_HINT}", environment.name)
            } else {
                environment.name.clone()
            };
            let menu_name = format!("Actions for environment {}", environment.name);
            let (row, menu) = widgets::list_row(
                ui,
                &environment.name,
                ListIcon::Dot(environment.color.map(theme::env_color)),
                current == scope,
                ListEnd::Menu {
                    name: &menu_name,
                    checked: active,
                },
                Some(&hint),
            );
            name(&row, &format!("Environment · {}", environment.name));
            if active {
                row.ctx.accesskit_node_builder(row.id, |node| {
                    node.set_description(ACTIVE_HINT);
                });
            }
            select_scope(&row, scope, actions);
            environment_menu(ui, &row, menu.as_ref(), environment.id, actions);
        });
    }

    ui.add_space(theme::spacing().sm);
    scope_heading(ui, "Collections");
    for collection in &state.collections {
        let scope = Scope::Collection(collection.id);
        let (row, _) = ui
            .push_id(scope, |ui| {
                widgets::list_row(
                    ui,
                    &collection.name,
                    ListIcon::Glyph(Icon::Folder),
                    current == scope,
                    ListEnd::None,
                    Some(&collection.name),
                )
            })
            .inner;
        name(&row, &format!("Collection · {}", collection.name));
        select_scope(&row, scope, actions);
    }
}

fn scope_heading(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(theme::spacing().sm);
        ui.label(
            theme::meta(text)
                .font(theme::semibold(theme::META))
                .color(palette().text_muted),
        );
    });
}

fn select_scope(response: &Response, scope: Scope, actions: &mut Vec<Action>) {
    if response.clicked() {
        actions.push(Action::Variables(VariablesAction::SelectScope(scope)));
    }
}

fn environment_menu(
    ui: &Ui,
    row: &Response,
    menu: Option<&Response>,
    id: imposter_app::core::ids::EnvironmentId,
    actions: &mut Vec<Action>,
) {
    let show_menu = egui::accesskit::Action::ShowContextMenu;
    row.ctx
        .accesskit_node_builder(row.id, |node| node.add_action(show_menu));
    let keyboard =
        row.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
    let assistive = ui.input(|i| {
        i.accesskit_action_requests(row.id, show_menu)
            .next()
            .is_some()
    });
    let open = if keyboard || assistive || row.secondary_clicked() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if menu.is_some_and(Response::clicked) {
        Some(egui::SetOpenCommand::Toggle)
    } else if row.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    egui::Popup::menu(row).open_memory(open).show(|ui| {
        let edit = button(ui, Variant::Ghost, None, "Edit name and color…");
        if keyboard || assistive {
            edit.request_focus();
        }
        if edit.clicked() {
            actions.push(Action::Variables(VariablesAction::RenameEnvironment(id)));
            ui.close();
        }
        if button(ui, Variant::Ghost, None, "Delete environment…").clicked() {
            actions.push(Action::Variables(VariablesAction::DeleteEnvironment(id)));
            ui.close();
        }
    });
}

fn scope_toolbar(state: &mut State, ui: &mut Ui, actions: &mut Vec<Action>) -> Option<String> {
    let current = state.variables.scope;
    let has_local_data = SecretScope::of_scope(current).is_some();
    let highlight = if state.variables.loaded {
        state.variables.highlight.take()
    } else {
        None
    };
    if let Some(target) = highlight.as_deref() {
        state.variables.tab = if has_local_data && secret_name(target).is_some() {
            VariablesTab::Secrets
        } else {
            VariablesTab::Variables
        };
    }
    if !has_local_data {
        state.variables.tab = VariablesTab::Variables;
    }

    let (usage, precedence) = explanation(state, current);
    let guidance = format!("{usage}\n{precedence}");
    let mut tab = state.variables.tab;
    widgets::toolbar_row_at_height(
        ui,
        widgets::tab_height(),
        |ui| {
            let tabs = [
                TabSpec::new(VariablesTab::Variables, "Variables").named("Variables tab"),
                TabSpec::new(VariablesTab::Secrets, "Secrets"),
                TabSpec::new(VariablesTab::Cookies, "Cookies"),
            ];
            let count = if has_local_data { tabs.len() } else { 1 };
            widgets::tab_strip(
                ui,
                &tabs[..count],
                &mut tab,
                count,
                widgets::HELP_SIZE + theme::spacing().sm,
                "More variable tabs",
            );
            widgets::help(ui, &guidance);
        },
        |ui| scope_actions(state, ui, actions),
    );
    state.variables.tab = tab;
    highlight
}

fn scope_content(
    state: &mut State,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
    highlight: Option<&str>,
) {
    let current = state.variables.scope;
    if state.variables_changed_on_disk {
        use crate::app::DiskChoice;
        match workspace::notice_banner(
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
        return;
    }
    if matches!(current, Scope::Environment(id) if state.active_environment != Some(id)) {
        ui.add(egui::Label::new(theme::meta(INACTIVE_NOTE).color(palette().text_muted)).truncate())
            .on_hover_text(INACTIVE_NOTE);
        ui.add_space(theme::spacing().xs);
    }

    tabs_content(state, ui, actions, highlight);
}

fn scope_actions(state: &State, ui: &mut Ui, actions: &mut Vec<Action>) {
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
    if widgets::save_controls_with(ui, &status, "Save variables", |ui| {
        let revert = ui
            .add_enabled_ui(dirty, |ui| {
                icons::button(ui, Icon::Revert, "Revert unsaved changes", false)
            })
            .inner;
        if revert.clicked() {
            actions.push(Action::Variables(VariablesAction::Revert));
        }
        icons::menu(ui, Icon::More, "Variable actions", |ui| {
            if screen.scope == Scope::Global {
                let mut share = state.globals_shared;
                if widgets::checkbox(ui, &mut share, "Share values").clicked() {
                    actions.push(Action::Variables(VariablesAction::ShareGlobals(share)));
                }
                ui.separator();
            }
            if ui
                .add_enabled_ui(!screen.rows.is_empty(), |ui| {
                    button(ui, Variant::Ghost, None, "Clear all variables…")
                })
                .inner
                .clicked()
            {
                actions.push(Action::Variables(VariablesAction::ClearAll));
            }
        });
    }) {
        actions.push(Action::Variables(VariablesAction::Save));
    }
}

fn tabs_content(
    state: &mut State,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
    highlight: Option<&str>,
) {
    let screen = &mut state.variables;
    let edited = egui::ScrollArea::vertical()
        .id_salt((screen.scope, screen.tab))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::scroll_view_bounds(ui);
            let remove_width = match screen.tab {
                VariablesTab::Variables | VariablesTab::Secrets => REMOVE_WIDTH,
                VariablesTab::Cookies => icons::size(),
            };
            let columns = Columns::new(ui, remove_width);
            match screen.tab {
                VariablesTab::Variables => {
                    variables_table(ui, &mut screen.rows, &columns, highlight)
                }
                VariablesTab::Secrets => secrets_table(
                    ui,
                    &mut screen.secrets,
                    &columns,
                    highlight.and_then(secret_name),
                ),
                VariablesTab::Cookies => {
                    let requests = match screen.scope {
                        Scope::Environment(id) => {
                            state.environments.iter().find(|e| e.id == id).map_or_else(
                                || "requests sent with this environment".to_owned(),
                                |e| format!("requests sent with {}", e.name),
                            )
                        }
                        _ => "requests sent without an environment".to_owned(),
                    };
                    cookies_table(ui, &screen.cookies, &columns, &requests, actions);
                    false
                }
            }
        })
        .inner;
    if edited {
        actions.push(Action::Variables(VariablesAction::Edited));
    }
}

fn variables_table(
    ui: &mut Ui,
    rows: &mut Vec<vars::VariableRow>,
    columns: &Columns,
    highlight: Option<&str>,
) -> bool {
    ui.horizontal(|ui| {
        heading_cell(ui, columns.name, "Variable name");
        heading_cell(ui, columns.value, "Values");
    });
    let mut edited = false;
    let mut remove = None;
    for (index, row) in rows.iter_mut().enumerate() {
        let (changed, removed) = variable_row(ui, row, index + 1, columns, highlight);
        edited |= changed;
        if removed {
            remove = Some(index);
        }
    }
    if let Some(index) = remove {
        rows.remove(index);
        edited = true;
    }
    let id = ui.make_persistent_id("new variable");
    let mut row = ui.data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || vars::VariableRow::new(""))
            .clone()
    });
    ui.horizontal(|ui| {
        variable_name(ui, &mut row, "New variable name", columns.name);
        blank_cell(ui, columns.value);
        blank_cell(ui, REMOVE_WIDTH);
    });
    if !row.name.is_empty() {
        ui.data_mut(|data| data.remove::<vars::VariableRow>(id));
        edited |= append_row(ui, rows, row);
    }
    edited
}

fn variable_row(
    ui: &mut Ui,
    row: &mut vars::VariableRow,
    number: usize,
    columns: &Columns,
    highlight: Option<&str>,
) -> (bool, bool) {
    let Columns {
        name: name_width,
        value: value_width,
        height,
    } = *columns;
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
                values_editor(ui, row, number, value_width)
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

fn secrets_table(
    ui: &mut Ui,
    rows: &mut Vec<vars::Row>,
    columns: &Columns,
    highlight: Option<&str>,
) -> bool {
    let name_width = columns.name;
    let value_width = columns.value_with_eye(ui);
    let mut edited = false;
    ui.horizontal(|ui| {
        columns.heading_with_help(
            ui,
            "Secret name",
            "Used as {{secret:name}}. Kept only on this computer: never exported, and removed from History.",
        );
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
            if highlight == Some(row.name.as_str()) {
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
            if remove_button(ui, id, &what, columns.height) {
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

fn cookies_table(
    ui: &mut Ui,
    cookies: &[imposter_app::http::cookies::StoredCookie],
    columns: &Columns,
    requests: &str,
    actions: &mut Vec<Action>,
) {
    let name_width = columns.name;
    let value_width = columns.value_with_eye(ui);
    ui.horizontal(|ui| {
        columns.heading_with_help(
            ui,
            "Cookie name",
            &format!(
                "Set by servers for {requests}, and sent back to them. Kept only on this computer."
            ),
        );
        if ui
            .add_enabled_ui(!cookies.is_empty(), |ui| {
                icons::button(ui, Icon::Trash, "Clear cookies…", false)
            })
            .inner
            .clicked()
        {
            actions.push(Action::Variables(VariablesAction::ClearCookies));
        }
    });
    if cookies.is_empty() {
        ui.label(tone_text("No cookies yet.", Tone::Muted));
        return;
    }
    for (index, cookie) in cookies.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized(
                [name_width, columns.height],
                egui::Label::new(theme::body(&cookie.name)).truncate(),
            );
            let shown_id = ui.make_persistent_id(("cookie shown", index, &cookie.name));
            let mut shown: bool = ui.data(|d| d.get_temp(shown_id)).unwrap_or(false);
            let mut value = cookie.value.as_str();
            table_text(
                ui,
                &mut value,
                ui.make_persistent_id(("cookie", index, "value")),
                &format!("Cookie {} value", cookie.name),
                value_width,
                "",
                !shown,
            );
            let (icon, action) = if shown {
                (Icon::EyeOff, format!("Hide cookie {}", cookie.name))
            } else {
                (Icon::Eye, format!("Show cookie {}", cookie.name))
            };
            if icons::button(ui, icon, &action, false).clicked() {
                shown = !shown;
            }
            ui.data_mut(|d| d.insert_temp(shown_id, shown));
            if icons::button(
                ui,
                Icon::Close,
                &format!("Remove cookie {}", cookie.name),
                false,
            )
            .clicked()
            {
                actions.push(Action::Variables(VariablesAction::RemoveCookie(index)));
            }
        });
        let place = cookie_place(cookie);
        ui.add(egui::Label::new(theme::meta(&place).color(palette().text_muted)).truncate())
            .on_hover_text(place);
        ui.add_space(theme::spacing().xs);
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

fn values_editor(ui: &mut Ui, row: &mut vars::VariableRow, number: usize, line_width: f32) -> bool {
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

    let remaining = ui.available_size_before_wrap().x;
    let composer_width = if remaining < line_width.min(160.0) {
        line_width
    } else {
        remaining
    };
    let composer = table_text(
        ui,
        &mut row.draft,
        egui::Id::unique(("variable draft", row.key)),
        &format!("Add value to variable {number}"),
        composer_width,
        "Add value…",
        false,
    );
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
    let prefix = if capture { "Latest capture · " } else { "" };
    let text = widgets::value_text(&label, prefix);
    let height = icons::size();
    let text_width = widgets::text_width(ui, text.clone());
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
    let button = egui::Button::selectable(current, text)
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
    name(&chip, &label.accessible_name(title));
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
        ui.add(
            egui::Label::new(widgets::value_text(&label, "").color(palette().text_muted))
                .truncate(),
        );
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
