//! Modal dialogs. Each returns what the user chose; the app decides what that means.

use std::path::PathBuf;

use egui::{Id, RichText};
use imposter_app::core::ids::{CollectionId, EnvironmentId, FolderId};
use imposter_app::core::model::EnvColor;
use imposter_app::transfer::ImportPreview;

use crate::theme::Tone;
use crate::tree::{Node, NodeId};
use crate::vars::Scope;
use crate::widgets::{name, tone_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamePurpose {
    NewCollection,
    NewFolder { parent: Option<FolderId> },
    NewRequest { folder: Option<FolderId> },
    RenameCollection(CollectionId),
    Rename(NodeId),
    SaveDraft(CollectionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmPurpose {
    DeleteCollection(CollectionId),
    Delete(NodeId),
    DeleteEnvironment(EnvironmentId),
    ClearHistory,
    /// Remove every row of the Variables screen's scope (an unsaved edit).
    ClearVariables(Scope),
    /// Put the Variables screen's scope back as stored, losing its unsaved edits.
    RevertVariables(Scope),
    DiscardOnClose,
    /// Write a Postman export despite the listed limitations.
    PostmanExport,
    /// Start a load test against a red (production) environment.
    StartLoad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Name {
        title: String,
        label: String,
        value: String,
        purpose: NamePurpose,
    },
    Confirm {
        title: String,
        message: String,
        confirm: String,
        purpose: ConfirmPurpose,
    },
    /// Create an environment (`id` is `None`) or rename one, and choose its color.
    Environment {
        id: Option<EnvironmentId>,
        name: String,
        color: Option<EnvColor>,
        /// Whether the user chose the color; until then a new environment's follows its name.
        chosen: bool,
    },
    /// Unsaved changes before navigating; `failed` after an autosave or save attempt failed.
    Guard {
        request: String,
        /// For an unsaved draft: the collection Save adds it to.
        draft: Option<String>,
        failed: Option<String>,
    },
    /// Unsaved rows on the Variables screen.
    VariablesGuard,
    /// Unsaved application settings while closing.
    SettingsGuard,
    MocksGuard,
    Move {
        node: NodeId,
        options: Vec<(String, Option<FolderId>)>,
        choice: usize,
    },
    Message {
        title: String,
        message: String,
        tone: Tone,
    },
    /// Whether a Postman import also reads environment or globals files.
    Companions {
        path: PathBuf,
    },
    /// Review before anything is written; losses that change requests need explicit consent.
    Import {
        preview: Box<ImportPreview>,
        remove_unsupported: bool,
    },
    /// Paste a curl command; it opens as an unsaved draft.
    Curl {
        text: String,
    },
    /// Every keyboard shortcut, from `shortcuts::ALL`.
    Shortcuts,
    /// Fuzzy search over actions, requests and environments.
    Palette(Box<crate::palette::Palette>),
    /// What an export includes. Environments and globals are opt-in.
    Export {
        environments: Vec<(EnvironmentId, String, bool)>,
        globals: bool,
        postman: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Name(NamePurpose, String),
    Environment {
        id: Option<EnvironmentId>,
        name: String,
        color: Option<EnvColor>,
    },
    Confirmed(ConfirmPurpose),
    Guard(GuardChoice),
    VariablesGuard(GuardChoice),
    SettingsGuard(GuardChoice),
    MocksGuard(GuardChoice),
    Move(NodeId, Option<FolderId>),
    /// Read companion files (`true`) or import the file alone.
    Companions(bool),
    /// Import the reviewed preview; the dialog still holds it.
    Import,
    /// Export with the dialog's choices; the dialog still holds them.
    Export,
    /// Open the pasted command as a draft; the dialog still holds it and it parses.
    Curl,
    /// Run the palette's choice; the dialog still holds it.
    Palette,
    /// Cancelled, closed or acknowledged.
    Closed,
}

impl NamePurpose {
    /// The verb on the dialog's main button.
    const fn verb(self) -> &'static str {
        match self {
            Self::NewCollection | Self::NewFolder { .. } | Self::NewRequest { .. } => "Create",
            Self::RenameCollection(_) | Self::Rename(_) => "Rename",
            Self::SaveDraft(_) => "Save",
        }
    }
}

/// How a dialog button reads: the main action, a destructive one, or the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Primary,
    Danger,
    Plain,
}

/// The width of the shortcut sheet's key column.
const KEYS_COLUMN: f32 = 180.0;

/// Space kept free at the right of a scrolling list for its scroll bar.
const SCROLL_BAR_ROOM: f32 = 16.0;

/// How a dialog button is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Accent,
    Red,
    /// Neutral, with red text.
    RedText,
    Neutral,
}

/// Only the main action, last in the row, is filled: red when destructive. A destructive
/// choice before it keeps red text, so "discard" never looks like "keep". A disabled
/// button is neutral, so its label stays readable.
fn look(kind: Kind, enabled: bool, last: bool) -> Look {
    match (kind, enabled, last) {
        (_, false, _) | (Kind::Plain, _, _) => Look::Neutral,
        (Kind::Primary, true, _) => Look::Accent,
        (Kind::Danger, true, true) => Look::Red,
        (Kind::Danger, true, false) => Look::RedText,
    }
}

/// The button row every dialog ends with, laid out left to right as given: choices and
/// Cancel first, the main or destructive action last. Enter presses the
/// last button when `enter` and it is enabled; Escape closes the dialog (the modal does that).
/// Returns the index of the button pressed.
fn buttons(ui: &mut egui::Ui, row: &[(&str, Kind, bool)], enter: bool) -> Option<usize> {
    let mut pressed = None;
    ui.add_space(12.0);
    // Left to right: a right-to-left row in an auto-sized modal can land outside it.
    ui.horizontal(|ui| {
        for (index, (label, kind, enabled)) in row.iter().enumerate() {
            let palette = crate::theme::palette();
            let button = match look(*kind, *enabled, index + 1 == row.len()) {
                Look::Red => egui::Button::new(RichText::new(*label).color(palette.on_accent))
                    .fill(palette.error),
                Look::Accent => egui::Button::new(RichText::new(*label).color(palette.on_accent))
                    .fill(palette.accent),
                Look::RedText => egui::Button::new(RichText::new(*label).color(palette.error)),
                Look::Neutral => egui::Button::new(*label),
            };
            if ui.add_enabled(*enabled, button).clicked() {
                pressed = Some(index);
            }
        }
    });
    let last = row.len().checked_sub(1);
    if pressed.is_none()
        && enter
        && last.is_some_and(|last| row[last].2)
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
    {
        pressed = last;
    }
    pressed
}

/// "Name:" and "Name" read the same; dialogs show captions without the colon.
fn caption(label: &str) -> String {
    label.trim_end().trim_end_matches(':').to_owned()
}

impl Dialog {
    pub fn name(title: &str, label: &str, value: &str, purpose: NamePurpose) -> Self {
        Self::Name {
            title: title.into(),
            label: caption(label),
            value: value.into(),
            purpose,
        }
    }

    /// A new environment: no name yet, so no color.
    pub fn new_environment() -> Self {
        Self::Environment {
            id: None,
            name: String::new(),
            color: None,
            chosen: false,
        }
    }

    pub fn rename_environment(id: EnvironmentId, name: &str, color: Option<EnvColor>) -> Self {
        Self::Environment {
            id: Some(id),
            name: name.to_owned(),
            color,
            chosen: true,
        }
    }

    pub fn error(title: &str, message: impl Into<String>) -> Self {
        Self::Message {
            title: title.into(),
            message: message.into(),
            tone: Tone::Error,
        }
    }

    pub fn info(title: &str, message: impl Into<String>) -> Self {
        Self::Message {
            title: title.into(),
            message: message.into(),
            tone: Tone::Normal,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        // One id per kind: a dialog replacing another must not inherit its size and position.
        let id = Id::unique(("dialog", std::mem::discriminant(self)));
        let mut modal = egui::Modal::new(id);
        if matches!(self, Self::Palette(_)) {
            // Pinned to the top: centred, it would move as the list shrinks while typing.
            modal = modal
                .area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, [0.0, 72.0]));
        }
        // Taller dialogs scroll inside the window instead of running off it.
        let max_height = ctx.content_rect().height() - 96.0;
        let modal = modal.show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.set_max_height(max_height);
            match self {
                Self::Name {
                    title,
                    label,
                    value,
                    purpose,
                } => {
                    ui.heading(title.as_str());
                    // Read before the field draws: a single-line field consumes Enter.
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let caption = ui.label(label.as_str());
                    let field =
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                    let field = field.labelled_by(caption.id);
                    if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                        field.request_focus();
                    }
                    let valid = !value.trim().is_empty();
                    let enter = enter_pressed && (field.lost_focus() || field.has_focus());
                    match buttons(
                        ui,
                        &[("Cancel", Kind::Plain, true), (purpose.verb(), Kind::Primary, valid)],
                        false,
                    ) {
                        Some(0) => outcome = Outcome::Closed,
                        Some(_) => outcome = Outcome::Name(*purpose, value.trim().to_owned()),
                        None if valid && enter => {
                            outcome = Outcome::Name(*purpose, value.trim().to_owned());
                        }
                        None => {}
                    }
                }
                Self::Environment {
                    id,
                    name: value,
                    color,
                    chosen,
                } => {
                    ui.heading(if id.is_some() {
                        "Edit Environment"
                    } else {
                        "New Environment"
                    });
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let caption = ui.label("Name");
                    let field =
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                    let field = field.labelled_by(caption.id);
                    if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                        field.request_focus();
                    }
                    if field.changed() && !*chosen {
                        *color = EnvColor::suggested_for(value);
                    }
                    ui.add_space(8.0);
                    ui.label("Color");
                    if crate::widgets::env_swatches(ui, color) {
                        *chosen = true;
                    }
                    if !*chosen && color.is_some() {
                        ui.label(
                            tone_text("Suggested for production environments", Tone::Muted)
                                .small(),
                        );
                    }
                    let valid = !value.trim().is_empty();
                    let enter = enter_pressed && (field.lost_focus() || field.has_focus());
                    let verb = if id.is_some() { "Save" } else { "Create" };
                    match buttons(
                        ui,
                        &[("Cancel", Kind::Plain, true), (verb, Kind::Primary, valid)],
                        false,
                    ) {
                        Some(0) => outcome = Outcome::Closed,
                        Some(_) => {
                            outcome = Outcome::Environment {
                                id: *id,
                                name: value.trim().to_owned(),
                                color: *color,
                            };
                        }
                        None if valid && enter => {
                            outcome = Outcome::Environment {
                                id: *id,
                                name: value.trim().to_owned(),
                                color: *color,
                            };
                        }
                        None => {}
                    }
                }
                Self::Confirm {
                    title,
                    message,
                    confirm,
                    purpose,
                } => {
                    ui.heading(title.as_str());
                    ui.label(message.as_str());
                    match buttons(
                        ui,
                        &[("Cancel", Kind::Plain, true), (confirm.as_str(), Kind::Danger, true)],
                        true,
                    ) {
                        Some(0) => outcome = Outcome::Closed,
                        Some(_) => outcome = Outcome::Confirmed(*purpose),
                        None => {}
                    }
                }
                Self::Guard {
                    request,
                    draft,
                    failed,
                } => {
                    if let Some(error) = failed {
                        ui.heading("Request could not be saved");
                        ui.label(
                            "Retry saving this request, discard its changes, or keep editing.",
                        );
                        ui.label(tone_text(error.as_str(), Tone::Error));
                    } else if let Some(collection) = draft {
                        // A draft has no saved version: saving creates a request.
                        ui.heading("Keep this draft?");
                        ui.label(format!(
                            "\"{request}\" is not saved yet. Save it as a new request in \"{collection}\", discard it, or stay."
                        ));
                    } else {
                        ui.heading("Unsaved request changes");
                        ui.label(format!("Save changes to \"{request}\" before continuing?"));
                    }
                    let save = match (&failed, &draft) {
                        (Some(_), _) => "Retry",
                        (None, Some(_)) => "Save as new request",
                        (None, None) => "Save",
                    };
                    let stay = if draft.is_some() { "Stay" } else { "Cancel" };
                    if let Some(choice) = guard_buttons(ui, stay, save) {
                        outcome = Outcome::Guard(choice);
                    }
                }
                Self::VariablesGuard => {
                    ui.heading("Unsaved variables");
                    ui.label("Save variable changes before continuing?");
                    if let Some(choice) = guard_buttons(ui, "Cancel", "Save") {
                        outcome = Outcome::VariablesGuard(choice);
                    }
                }
                Self::MocksGuard => {
                    ui.heading("Unsaved mocks");
                    ui.label("Save mock configuration before continuing?");
                    if let Some(choice) = guard_buttons(ui, "Cancel", "Save") {
                        outcome = Outcome::MocksGuard(choice);
                    }
                }
                Self::SettingsGuard => {
                    ui.heading("Unsaved application settings");
                    ui.label("Save application settings before exiting?");
                    if let Some(choice) = guard_buttons(ui, "Cancel", "Save") {
                        outcome = Outcome::SettingsGuard(choice);
                    }
                }
                Self::Move {
                    node,
                    options,
                    choice,
                } => {
                    ui.heading("Move");
                    let caption = ui.label("Destination");
                    let selected = options.get(*choice).map_or("", |(label, _)| label.as_str());
                    let combo = egui::ComboBox::from_id_salt("move destination")
                        .width(300.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (index, (label, _)) in options.iter().enumerate() {
                                ui.selectable_value(choice, index, label.as_str());
                            }
                        });
                    combo.response.labelled_by(caption.id);
                    match buttons(
                        ui,
                        &[("Cancel", Kind::Plain, true), ("Move", Kind::Primary, true)],
                        true,
                    ) {
                        Some(0) => outcome = Outcome::Closed,
                        Some(_) => {
                            if let Some((_, target)) = options.get(*choice) {
                                outcome = Outcome::Move(*node, *target);
                            }
                        }
                        None => {}
                    }
                }
                Self::Message {
                    title,
                    message,
                    tone,
                } => {
                    ui.heading(title.as_str());
                    let text = ui.add(egui::Label::new(tone_text(message.as_str(), *tone)).wrap());
                    name(&text, message);
                    if buttons(ui, &[("OK", Kind::Primary, true)], true).is_some() {
                        outcome = Outcome::Closed;
                    }
                }
                Self::Palette(palette) => palette.show(ui, &mut outcome),
                Self::Shortcuts => {
                    // A sheet to read, with nothing to choose: Esc or a click outside closes
                    // it, as every modal, so it has no button row.
                    ui.horizontal(|ui| {
                        ui.heading("Keyboard shortcuts");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(tone_text("Esc to close", Tone::Muted).small());
                        });
                    });
                    ui.add_space(4.0);
                    // The title stays visible; the list scrolls in what is left. Full
                    // width, so its scroll bar runs along the sheet's edge.
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height((max_height - 56.0).max(120.0))
                        .show(ui, |ui| {
                            // Clear of the scroll bar; descriptions wrap in what is left
                            // after the keys, whose column fits the widest row in every
                            // group, so all descriptions line up.
                            let width = ui.available_width() - SCROLL_BAR_ROOM;
                            ui.set_max_width(width);
                            let keys_column = crate::shortcuts::ALL
                                .iter()
                                .map(|s| crate::widgets::key_caps_width(ui, s.keys))
                                .fold(KEYS_COLUMN, f32::max);
                            for (group, title) in crate::shortcuts::Group::ALL {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(title)
                                        .font(crate::theme::semibold(13.0))
                                        .color(crate::theme::palette().text_muted),
                                );
                                egui::Grid::new(("shortcuts", title))
                                    .num_columns(2)
                                    .spacing([24.0, 6.0])
                                    .min_col_width(keys_column)
                                    .max_col_width((width - keys_column - 24.0).max(160.0))
                                    .show(ui, |ui| {
                                        for shortcut in crate::shortcuts::ALL
                                            .iter()
                                            .filter(|s| s.group == group)
                                        {
                                            crate::widgets::key_caps(ui, shortcut.keys);
                                            ui.add(egui::Label::new(shortcut.description).wrap());
                                            ui.end_row();
                                        }
                                    });
                            }
                        });
                }
                Self::Companions { .. } => {
                    ui.heading("Companion files");
                    ui.label("Include Postman environment or global JSON files?");
                    match buttons(
                        ui,
                        &[("No", Kind::Plain, true), ("Yes", Kind::Primary, true)],
                        true,
                    ) {
                        Some(0) => outcome = Outcome::Companions(false),
                        Some(_) => outcome = Outcome::Companions(true),
                        None => {}
                    }
                }
                Self::Import {
                    preview,
                    remove_unsupported,
                } => outcome = import(ui, preview, remove_unsupported),
                Self::Export {
                    environments,
                    globals,
                    postman,
                } => outcome = export(ui, environments, globals, postman),
                Self::Curl { text } => outcome = curl(ui, text),
            }
        });
        if outcome == Outcome::Open && modal.should_close() {
            outcome = match self {
                Self::Guard { .. } => Outcome::Guard(GuardChoice::Cancel),
                Self::VariablesGuard => Outcome::VariablesGuard(GuardChoice::Cancel),
                Self::SettingsGuard => Outcome::SettingsGuard(GuardChoice::Cancel),
                Self::MocksGuard => Outcome::MocksGuard(GuardChoice::Cancel),
                _ => Outcome::Closed,
            };
        }
        outcome
    }
}

/// Stay, then Discard (destructive), then the save action, which Enter presses.
fn guard_buttons(ui: &mut egui::Ui, stay: &str, save: &str) -> Option<GuardChoice> {
    buttons(
        ui,
        &[
            (stay, Kind::Plain, true),
            ("Discard", Kind::Danger, true),
            (save, Kind::Primary, true),
        ],
        true,
    )
    .map(|index| [GuardChoice::Cancel, GuardChoice::Discard, GuardChoice::Save][index])
}

fn curl(ui: &mut egui::Ui, text: &mut String) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.set_min_width(560.0);
    ui.heading("Import from curl");
    let caption = ui.label("curl command");
    let field = ui.add(
        egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(8)
            .desired_width(f32::INFINITY)
            .hint_text("curl 'https://api.example.com/items' -H 'Accept: application/json'"),
    );
    let field = field.labelled_by(caption.id);
    if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    let parsed = (!text.trim().is_empty()).then(|| imposter_app::core::curl::parse(text));
    match &parsed {
        Some(Ok(parsed)) => {
            for warning in &parsed.warnings {
                ui.label(tone_text(warning.as_str(), Tone::Warning));
            }
        }
        Some(Err(error)) => {
            ui.label(tone_text(error.to_string(), Tone::Error));
        }
        None => {}
    }
    let valid = matches!(parsed, Some(Ok(_)));
    // Enter types a new line in the command, so it does not press the button.
    match buttons(
        ui,
        &[
            ("Cancel", Kind::Plain, true),
            ("Open as draft", Kind::Primary, valid),
        ],
        false,
    ) {
        Some(0) => outcome = Outcome::Closed,
        Some(_) => outcome = Outcome::Curl,
        None => {}
    }
    outcome
}

/// What a delete takes with it: "8 requests in 2 folders", "1 request", "nothing inside".
pub fn contents(requests: usize, folders: usize) -> String {
    match (requests, folders) {
        (0, 0) => "nothing inside".to_owned(),
        (requests, 0) => counted(requests, "request"),
        (requests, folders) => format!(
            "{} in {}",
            counted(requests, "request"),
            counted(folders, "folder")
        ),
    }
}

/// Every action that removes saved data asks first, naming what goes, and the red button
/// says what it does: one stray click must never lose data.
impl Dialog {
    fn confirm(title: &str, message: String, confirm: &str, purpose: ConfirmPurpose) -> Self {
        Self::Confirm {
            title: title.to_owned(),
            message,
            confirm: confirm.to_owned(),
            purpose,
        }
    }

    /// Many requests against a red environment, which usually means production.
    pub fn start_load(environment: &str, requests: u32, request: &str) -> Self {
        Self::confirm(
            &format!("Load test {environment}?"),
            format!(
                "This sends \"{request}\" {} times with the {environment} environment.",
                crate::load_screen::count_text(u64::from(requests))
            ),
            "Start load test",
            ConfirmPurpose::StartLoad,
        )
    }

    /// `roots` are the collection's top-level nodes, when its tree is loaded.
    pub fn delete_collection(id: CollectionId, name: &str, roots: Option<&[Node]>) -> Self {
        let what = roots.map_or_else(String::new, |roots| {
            format!(
                " with {}",
                contents(
                    crate::tree::request_count(roots),
                    crate::tree::folder_count(roots)
                )
            )
        });
        Self::confirm(
            "Delete collection",
            format!(
                "Delete \"{name}\"{what}, including unsaved edits? This cannot be undone; \
                 their runs stay in History."
            ),
            "Delete collection",
            ConfirmPurpose::DeleteCollection(id),
        )
    }

    pub fn delete_node(node: &Node) -> Self {
        let (title, message) = match node {
            Node::Folder { folder, children } => (
                "Delete folder",
                format!(
                    "Delete the folder \"{}\" with {}, including unsaved edits? This cannot \
                     be undone; their runs stay in History.",
                    folder.name,
                    contents(
                        crate::tree::request_count(children),
                        crate::tree::folder_count(children)
                    )
                ),
            ),
            Node::Request(request) => (
                "Delete request",
                format!(
                    "Delete the request \"{}\", including unsaved edits? This cannot be \
                     undone; its runs stay in History.",
                    request.name
                ),
            ),
        };
        Self::confirm(title, message, title, ConfirmPurpose::Delete(node.id()))
    }

    pub fn delete_environment(id: EnvironmentId, name: &str, variables: usize) -> Self {
        Self::confirm(
            "Delete environment",
            format!(
                "Delete the environment \"{name}\" and its {}? This cannot be undone.",
                counted(variables, "variable")
            ),
            "Delete environment",
            ConfirmPurpose::DeleteEnvironment(id),
        )
    }

    /// `executions` is how many runs history holds, when known.
    pub fn clear_history(executions: Option<u64>) -> Self {
        let what = executions.map_or_else(
            || "all execution history".to_owned(),
            |n| {
                let n = usize::try_from(n).unwrap_or(usize::MAX);
                format!("{} from history", counted(n, "execution"))
            },
        );
        Self::confirm(
            "Clear all history",
            format!("Delete {what}? This cannot be undone. Saved requests are not affected."),
            "Delete history",
            ConfirmPurpose::ClearHistory,
        )
    }

    /// `name` is the scope as the picker shows it.
    pub fn clear_variables(scope: Scope, name: &str, variables: usize) -> Self {
        Self::confirm(
            "Clear all variables",
            format!(
                "Remove all {} from {name}? Nothing is written until you save; Revert brings \
                 them back before that.",
                counted(variables, "variable")
            ),
            "Remove all",
            ConfirmPurpose::ClearVariables(scope),
        )
    }

    pub fn revert_variables(scope: Scope, name: &str) -> Self {
        Self::confirm(
            "Revert variables",
            format!("Discard the unsaved changes to {name} and go back to what is saved?"),
            "Discard changes",
            ConfirmPurpose::RevertVariables(scope),
        )
    }
}

/// "1 request", "7 requests".
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn import(ui: &mut egui::Ui, preview: &ImportPreview, remove: &mut bool) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.set_min_width(520.0);
    ui.heading("Review collection import");
    // Nested folders and their requests count too: the summary is what gets written.
    ui.label(format!(
        "Creates {}: {} in {}",
        preview.collection_name,
        counted(preview.document.requests().len(), "request"),
        counted(preview.document.folder_count(), "folder"),
    ));
    let environments = if preview.environment_names.is_empty() {
        "None".to_owned()
    } else {
        preview.environment_names.join(", ")
    };
    ui.label(format!("New environments: {environments}"));
    // A script can only change its own request and read the variables that request could
    // already use, so it adds no way out a plain imported request lacks; still, say so.
    let scripted = preview
        .document
        .requests()
        .iter()
        .filter(|r| !r.scripts.is_empty())
        .count();
    if scripted > 0 {
        ui.label(format!(
            "{} {} scripts; they run when sent.",
            counted(scripted, "request"),
            if scripted == 1 { "has" } else { "have" }
        ));
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if preview.warnings.is_empty() {
                ui.label(tone_text("All supported fields validated.", Tone::Ok));
            }
            for warning in &preview.warnings {
                ui.label(tone_text(warning.as_str(), Tone::Warning));
            }
        });
    if preview.requires_removal {
        ui.checkbox(remove, "Remove the listed unsupported execution features");
    }
    let allowed = !preview.requires_removal || *remove;
    match buttons(
        ui,
        &[
            ("Cancel", Kind::Plain, true),
            ("Import", Kind::Primary, allowed),
        ],
        true,
    ) {
        Some(0) => outcome = Outcome::Closed,
        Some(_) => outcome = Outcome::Import,
        None => {}
    }
    outcome
}

fn export(
    ui: &mut egui::Ui,
    environments: &mut [(EnvironmentId, String, bool)],
    globals: &mut bool,
    postman: &mut bool,
) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.set_min_width(480.0);
    ui.heading("Export collection");
    ui.horizontal(|ui| {
        ui.radio_value(postman, false, "Native (.imposter.json)");
        ui.radio_value(postman, true, "Postman v2.1");
    });
    ui.add_space(6.0);
    ui.label("Include optional environments");
    if environments.is_empty() {
        ui.label(tone_text("No environments", Tone::Muted));
    }
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (_, label, include) in environments.iter_mut() {
                ui.checkbox(include, label.as_str());
            }
        });
    ui.checkbox(globals, "Include global variables");
    ui.label(tone_text(
        "Literal credentials and file paths are included. Uploaded file contents are excluded.",
        Tone::Muted,
    ));
    match buttons(
        ui,
        &[
            ("Cancel", Kind::Plain, true),
            ("Export", Kind::Primary, true),
        ],
        true,
    ) {
        Some(0) => outcome = Outcome::Closed,
        Some(_) => outcome = Outcome::Export,
        None => {}
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delete_names_what_goes_with_it() {
        assert_eq!(contents(0, 0), "nothing inside");
        assert_eq!(contents(1, 0), "1 request");
        assert_eq!(contents(8, 2), "8 requests in 2 folders");
        assert_eq!(contents(0, 1), "0 requests in 1 folder");
    }

    #[test]
    fn only_the_main_action_is_filled_and_disabled_buttons_are_neutral() {
        assert_eq!(look(Kind::Primary, true, true), Look::Accent);
        assert_eq!(look(Kind::Danger, true, true), Look::Red);
        assert_eq!(
            look(Kind::Danger, true, false),
            Look::RedText,
            "Discard before Save must not look like it"
        );
        assert_eq!(look(Kind::Primary, false, true), Look::Neutral);
        assert_eq!(look(Kind::Danger, false, true), Look::Neutral);
        assert_eq!(look(Kind::Plain, true, true), Look::Neutral);
    }
}
