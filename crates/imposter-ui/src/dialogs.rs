//! Modal dialogs. Each returns what the user chose; the app decides what that means.

use std::path::PathBuf;

use egui::{Id, RichText, Ui};
use imposter_app::core::ids::{CollectionId, EnvironmentId, FolderId};
use imposter_app::core::model::EnvColor;
use imposter_app::transfer::ImportPreview;

use crate::icons::{self, Icon};
use crate::theme::{self, Tone};
use crate::tree::{Node, NodeId};
use crate::vars::Scope;
use crate::widgets::{self, Variant, name, tone_text};

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
    /// Remove the cookie the Variables screen chose from `scope`'s jar; written at once.
    RemoveCookie(Scope),
    /// Remove every cookie of `scope`'s jar; written at once.
    ClearCookies(Scope),
    DiscardOnClose,
    /// Write a Postman export despite the listed limitations.
    PostmanExport,
    /// Start a load test against a red (production) environment.
    StartLoad,
    /// Start a run against a red (production) environment.
    StartRun,
}

impl ConfirmPurpose {
    const fn variant(self) -> Variant {
        match self {
            Self::PostmanExport | Self::StartLoad | Self::StartRun => Variant::Primary,
            Self::DeleteCollection(_)
            | Self::Delete(_)
            | Self::DeleteEnvironment(_)
            | Self::ClearHistory
            | Self::ClearVariables(_)
            | Self::RevertVariables(_)
            | Self::RemoveCookie(_)
            | Self::ClearCookies(_)
            | Self::DiscardOnClose => Variant::DangerPrimary,
        }
    }
}

/// Whose scripts a scripts dialog edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTarget {
    Collection(CollectionId),
    Folder(FolderId),
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
        /// Write its values to its file; off keeps them on this computer.
        share: bool,
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
    VariableValue {
        scope: Scope,
        row: u64,
        variable: String,
        original: String,
        value: String,
        available: bool,
    },
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
        notes_open: bool,
    },
    /// Paste a curl command; it opens as an unsaved draft.
    Curl {
        text: String,
    },
    /// Every keyboard shortcut, from `shortcuts::ALL`.
    Shortcuts,
    /// What scripts can use, from `imposter_script::API`.
    ScriptReference,
    /// A collection's or folder's scripts, run around every request inside it.
    Scripts {
        target: ScriptTarget,
        /// `Collection "API"`, `Folder "Users"`.
        owner: String,
        scripts: imposter_app::core::model::Scripts,
    },
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
        share: bool,
    },
    Confirmed(ConfirmPurpose),
    Guard(GuardChoice),
    VariablesGuard(GuardChoice),
    ApplyVariableValue,
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
    /// Save the scripts the dialog holds.
    Scripts,
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

/// The minimum width of the shortcut sheet's key column.
const KEYS_COLUMN: f32 = 180.0;

const FRAME_RADIUS: u8 = 8;
const PALETTE_TOP: f32 = 72.0;

struct Choice<'a> {
    label: &'a str,
    variant: Variant,
    outcome: Option<Outcome>,
}

impl<'a> Choice<'a> {
    fn new(label: &'a str, variant: Variant, outcome: Outcome) -> Self {
        Self {
            label,
            variant,
            outcome: Some(outcome),
        }
    }
}

struct Footer<'a> {
    choices: [Option<Choice<'a>>; 2],
    main: Choice<'a>,
}

impl<'a> Footer<'a> {
    fn new(label: &'a str, outcome: Option<Outcome>) -> Self {
        Self {
            choices: [
                Some(Choice::new("Cancel", Variant::Secondary, Outcome::Closed)),
                None,
            ],
            main: Choice {
                label,
                variant: Variant::Primary,
                outcome,
            },
        }
    }

    fn show(mut self, ui: &mut Ui, enter: bool) -> Outcome {
        let spacing = theme::spacing();
        let (divider, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(
            divider.x_range(),
            divider.center().y,
            egui::Stroke::new(1.0, theme::palette().border),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let choices_width: f32 = self
                .choices
                .iter()
                .flatten()
                .map(|choice| widgets::action_width(ui, choice.label) + spacing.sm)
                .sum();
            let row_width = choices_width + widgets::action_width(ui, self.main.label);
            // Reserve the leading space before drawing in Tab order. A right-to-left
            // layout in an auto-sized modal placed the buttons outside its click target.
            ui.add_space((ui.available_width() - row_width).max(0.0));
            let mut activated = None;
            for choice in self.choices.iter_mut().flatten() {
                if draw_choice(ui, choice) {
                    activated = choice.outcome.take();
                }
                ui.add_space(spacing.sm);
            }
            let main_clicked = draw_choice(ui, &self.main);
            activated
                .or_else(|| {
                    (main_clicked || enter)
                        .then_some(self.main.outcome)
                        .flatten()
                })
                .unwrap_or(Outcome::Open)
        })
        .inner
    }
}

fn draw_choice(ui: &mut Ui, choice: &Choice<'_>) -> bool {
    ui.add_enabled_ui(choice.outcome.is_some(), |ui| {
        widgets::button(ui, choice.variant, None, choice.label).clicked()
    })
    .inner
}

pub(crate) fn scroll_body<R>(ui: &mut Ui, max_height: f32, body: impl FnOnce(&mut Ui) -> R) -> R {
    // Solid bars reserve space for wrapping instead of covering the text on hover.
    ui.spacing_mut().scroll = egui::style::ScrollStyle {
        bar_inner_margin: theme::spacing().md,
        ..egui::style::ScrollStyle::solid()
    };
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, body)
        .inner
}

fn title_row(ui: &mut Ui, title: &str) -> bool {
    let spacing = theme::spacing();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing.sm;
        let width = ui.available_width() - icons::size() - spacing.sm;
        ui.add_sized(
            [width, spacing.control],
            egui::Label::new(theme::page_title(title))
                .truncate()
                .halign(egui::Align::Min),
        );
        icons::button_with_hint(ui, Icon::Close, "Close", "Close (Esc)", false).clicked()
    })
    .inner
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
            share: true,
        }
    }

    pub fn rename_environment(
        id: EnvironmentId,
        name: &str,
        color: Option<EnvColor>,
        share: bool,
    ) -> Self {
        Self::Environment {
            id: Some(id),
            name: name.to_owned(),
            color,
            chosen: true,
            share,
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

    pub fn review_import(preview: Box<ImportPreview>) -> Self {
        Self::Import {
            notes_open: preview.warnings.len() <= 3,
            preview,
            remove_unsupported: false,
        }
    }

    fn title(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Name { title, .. }
            | Self::Confirm { title, .. }
            | Self::Message { title, .. } => title.as_str().into(),
            Self::Environment { id: None, .. } => "New Environment".into(),
            Self::Environment { .. } => "Edit Environment".into(),
            Self::Guard {
                failed: Some(_), ..
            } => "Request could not be saved".into(),
            Self::Guard { draft: Some(_), .. } => "Keep this draft?".into(),
            Self::Guard { .. } => "Unsaved request changes".into(),
            Self::VariablesGuard => "Unsaved variables".into(),
            Self::VariableValue { variable, .. } => format!("Edit value for {variable}").into(),
            Self::MocksGuard => "Unsaved mocks".into(),
            Self::SettingsGuard => "Unsaved application settings".into(),
            Self::Move { .. } => "Move".into(),
            Self::Companions { .. } => "Companion files".into(),
            Self::Import { .. } => "Review collection import".into(),
            Self::Export { .. } => "Export collection".into(),
            Self::Curl { .. } => "Import from curl".into(),
            Self::Shortcuts => "Keyboard shortcuts".into(),
            Self::ScriptReference => "Script reference".into(),
            Self::Scripts { owner, .. } => format!("Scripts: {owner}").into(),
            Self::Palette(palette) => palette.title().into(),
        }
    }

    fn width(&self) -> f32 {
        match self {
            Self::Palette(_) | Self::Scripts { .. } | Self::Curl { .. } | Self::ScriptReference => {
                560.0
            }
            Self::Shortcuts | Self::Import { .. } => 520.0,
            Self::Export { .. } | Self::VariableValue { .. } => 480.0,
            Self::Guard { .. } => 440.0,
            _ => 360.0,
        }
    }

    fn cancel(&self) -> Outcome {
        match self {
            Self::Guard { .. } => Outcome::Guard(GuardChoice::Cancel),
            Self::VariablesGuard => Outcome::VariablesGuard(GuardChoice::Cancel),
            Self::SettingsGuard => Outcome::SettingsGuard(GuardChoice::Cancel),
            Self::MocksGuard => Outcome::MocksGuard(GuardChoice::Cancel),
            _ => Outcome::Closed,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        // A dialog replacing another must not inherit its size and position.
        let id = Id::unique(("dialog", std::mem::discriminant(self)));
        let spacing = theme::spacing();
        let colors = theme::palette();
        let frame = egui::Frame::new()
            .inner_margin(spacing.lg)
            .corner_radius(FRAME_RADIUS)
            .fill(colors.surface)
            .stroke(egui::Stroke::new(1.0, colors.field_border));
        let mut modal = egui::Modal::new(id)
            .frame(frame)
            .backdrop_color(colors.modal_scrim);
        let top = if matches!(self, Self::Palette(_)) {
            // Centred, it would move as the list shrinks while typing.
            modal = modal.area(
                egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, [0.0, PALETTE_TOP]),
            );
            PALETTE_TOP
        } else {
            spacing.lg
        };
        let window = ctx.content_rect().size();
        let insets = frame.total_margin().sum();
        let width = self.width().min(window.x - insets.x - 2.0 * spacing.lg);
        let has_footer = !matches!(
            self,
            Self::Palette(_) | Self::Shortcuts | Self::ScriptReference
        );
        let footer_height = if has_footer {
            spacing.control + 2.0 * spacing.md + 1.0
        } else {
            0.0
        };
        let content_height = window.y - top - spacing.lg - insets.y;
        let body_height = content_height - spacing.control - spacing.md - footer_height;
        // Read before the body: a single-line field consumes Enter.
        let enter = !matches!(
            self,
            Self::Scripts { .. } | Self::Curl { .. } | Self::VariableValue { .. }
        ) && !egui::Popup::is_any_open(ctx)
            && ctx.input(|i| i.key_pressed(egui::Key::Enter));

        let response = modal.show(ctx, |ui| {
            ui.set_width(width);
            // An Area starts with its previous size. Give a growing body the current cap.
            ui.set_max_height(content_height);
            ui.spacing_mut().item_spacing.y = spacing.md;
            // A focused control already receives Enter as a click from egui.
            let enter = enter
                && !focused_widget(ctx).is_some_and(|widget| {
                    widget.layer_id == ui.layer_id() && widget.sense == egui::Sense::click()
                });
            let close = title_row(ui, &self.title());
            let outcome = if let Self::Palette(palette) = self {
                palette.show(ui, body_height)
            } else if let Self::Scripts { owner, scripts, .. } = self {
                crate::scripts_tab::script_pair(ui, scripts, Some(owner), body_height);
                Footer::new("Save", Some(Outcome::Scripts)).show(ui, enter)
            } else {
                scroll_body(ui, body_height, |ui| self.body(ui))
                    .map_or(Outcome::Open, |footer| footer.show(ui, enter))
            };
            (close, outcome)
        });
        let outcome =
            if response.inner.0 || (response.inner.1 == Outcome::Open && response.should_close()) {
                self.cancel()
            } else {
                response.inner.1
            };
        if matches!(self, Self::VariableValue { .. })
            && outcome != Outcome::Open
            && let Some(focused) = focused_widget(ctx)
            && focused.layer_id == response.response.layer_id
        {
            // Autosave can resume as soon as the editor closes.
            focused.surrender_focus();
        }
        outcome
    }

    fn body(&mut self, ui: &mut Ui) -> Option<Footer<'_>> {
        Some(match self {
            Self::Name {
                label,
                value,
                purpose,
                ..
            } => {
                name_field(ui, label, value);
                Footer::new(
                    purpose.verb(),
                    (!value.trim().is_empty())
                        .then(|| Outcome::Name(*purpose, value.trim().to_owned())),
                )
            }
            Self::VariableValue {
                value, available, ..
            } => {
                let caption = ui.label("Value");
                let field = ui
                    .add(
                        egui::TextEdit::multiline(value)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(caption.id);
                initial_focus(ui, &field);
                if !*available {
                    ui.label(tone_text(
                        "This value changed while the editor was open.",
                        Tone::Error,
                    ));
                }
                Footer::new("Apply", available.then_some(Outcome::ApplyVariableValue))
            }
            Self::Environment {
                id,
                name: value,
                color,
                chosen,
                share,
            } => {
                if name_field(ui, "Name", value).changed() && !*chosen {
                    *color = EnvColor::suggested_for(value);
                }
                ui.vertical(|ui| {
                    if !*chosen && color.is_some() {
                        widgets::label_with_help(
                            ui,
                            "Color",
                            "Suggested for production environments",
                        );
                    } else {
                        ui.label("Color");
                    }
                    if widgets::env_swatches(ui, color) {
                        *chosen = true;
                    }
                });
                ui.horizontal(|ui| {
                    widgets::checkbox(ui, share, "Share values");
                    widgets::help(
                        ui,
                        if *share {
                            "Its values are written to its file in the collections folder."
                        } else {
                            "Its file lists the names only; the values stay on this computer."
                        },
                    );
                });
                Footer::new(
                    if id.is_some() { "Save" } else { "Create" },
                    (!value.trim().is_empty()).then(|| Outcome::Environment {
                        id: *id,
                        name: value.trim().to_owned(),
                        color: *color,
                        share: *share,
                    }),
                )
            }
            Self::Confirm {
                message,
                confirm,
                purpose,
                ..
            } => {
                ui.label(message.as_str());
                let mut footer = Footer::new(confirm, Some(Outcome::Confirmed(*purpose)));
                footer.main.variant = purpose.variant();
                footer
            }
            Self::Guard {
                request,
                draft,
                failed,
            } => {
                let save = if let Some(error) = failed {
                    ui.label(tone_text(error.as_str(), Tone::Error));
                    "Retry"
                } else if let Some(collection) = draft {
                    ui.label(format!(
                        r#"Save "{request}" as a new request in "{collection}"?"#
                    ));
                    "Save as new request"
                } else {
                    ui.label(format!(r#"Save changes to "{request}" before continuing?"#));
                    "Save"
                };
                guard_footer(
                    if draft.is_some() { "Stay" } else { "Cancel" },
                    save,
                    Outcome::Guard,
                )
            }
            Self::VariablesGuard => {
                ui.label("Save variable changes before continuing?");
                guard_footer("Cancel", "Save", Outcome::VariablesGuard)
            }
            Self::MocksGuard => {
                ui.label("Save mock configuration before continuing?");
                guard_footer("Cancel", "Save", Outcome::MocksGuard)
            }
            Self::SettingsGuard => {
                ui.label("Save application settings before exiting?");
                guard_footer("Cancel", "Save", Outcome::SettingsGuard)
            }
            Self::Move {
                node,
                options,
                choice,
            } => {
                let caption = ui.label("Destination");
                let selected = options.get(*choice).map_or("", |(label, _)| label.as_str());
                let combo = widgets::combo("move destination")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (index, (label, _)) in options.iter().enumerate() {
                            if widgets::option(ui, index == *choice, label.as_str()).clicked() {
                                *choice = index;
                                ui.close();
                            }
                        }
                    });
                combo.response.labelled_by(caption.id);
                Footer::new(
                    "Move",
                    options
                        .get(*choice)
                        .map(|(_, target)| Outcome::Move(*node, *target)),
                )
            }
            Self::Message { message, tone, .. } => {
                let text = ui.add(egui::Label::new(tone_text(message.as_str(), *tone)).wrap());
                name(&text, message);
                Footer {
                    choices: [None, None],
                    main: Choice::new("OK", Variant::Primary, Outcome::Closed),
                }
            }
            Self::Shortcuts => {
                shortcut_sheet(ui);
                return None;
            }
            Self::ScriptReference => {
                script_reference(ui);
                return None;
            }
            Self::Companions { .. } => {
                ui.label("Include Postman environment or global JSON files?");
                Footer {
                    choices: [
                        Some(Choice::new(
                            "No",
                            Variant::Secondary,
                            Outcome::Companions(false),
                        )),
                        None,
                    ],
                    main: Choice::new("Yes", Variant::Primary, Outcome::Companions(true)),
                }
            }
            Self::Import {
                preview,
                remove_unsupported,
                notes_open,
            } => import(ui, preview, remove_unsupported, notes_open),
            Self::Export {
                environments,
                globals,
                postman,
            } => export(ui, environments, globals, postman),
            Self::Curl { text } => curl(ui, text),
            Self::Palette(_) | Self::Scripts { .. } => return None,
        })
    }
}

fn name_field(ui: &mut Ui, label: &str, value: &mut String) -> egui::Response {
    let caption = ui.label(label);
    let field = ui
        .add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(caption.id);
    initial_focus(ui, &field);
    field
}

fn focused_widget(ctx: &egui::Context) -> Option<egui::Response> {
    ctx.memory(egui::Memory::focused)
        .and_then(|id| ctx.read_response(id))
}

pub(crate) fn initial_focus(ui: &Ui, field: &egui::Response) {
    if focused_widget(ui.ctx()).is_none_or(|focused| focused.layer_id != ui.layer_id()) {
        field.request_focus();
    }
}

fn guard_footer(
    stay: &'static str,
    save: &'static str,
    outcome: fn(GuardChoice) -> Outcome,
) -> Footer<'static> {
    Footer {
        choices: [
            Some(Choice::new(
                stay,
                Variant::Secondary,
                outcome(GuardChoice::Cancel),
            )),
            Some(Choice::new(
                "Discard",
                Variant::Danger,
                outcome(GuardChoice::Discard),
            )),
        ],
        main: Choice::new(save, Variant::Primary, outcome(GuardChoice::Save)),
    }
}

fn shortcut_sheet(ui: &mut Ui) {
    let spacing = theme::spacing();
    let description_width = |ui: &Ui, keys_column| ui.available_width() - keys_column - spacing.xl;
    let keys_column = crate::shortcuts::ALL
        .iter()
        .map(|s| widgets::key_caps_width(ui, s.keys))
        .fold(KEYS_COLUMN, f32::max);
    for (group, title) in crate::shortcuts::Group::ALL {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = spacing.xs;
            ui.label(theme::section(title).color(theme::palette().text_muted));
            egui::Grid::new(("shortcuts", title))
                .num_columns(2)
                .spacing([spacing.xl, spacing.sm])
                .min_col_width(keys_column)
                .max_col_width(description_width(ui, keys_column))
                .show(ui, |ui| {
                    for shortcut in crate::shortcuts::ALL.iter().filter(|s| s.group == group) {
                        widgets::key_caps(ui, shortcut.keys);
                        ui.add(egui::Label::new(shortcut.description).wrap());
                        ui.end_row();
                    }
                });
        });
    }
}

fn script_reference(ui: &mut Ui) {
    for phase in imposter_app::script::Phase::LISTED {
        ui.label(theme::section(phase.label()));
        for entry in imposter_app::script::API
            .iter()
            .filter(|e| e.phase == phase)
        {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = theme::spacing().xs;
                ui.add(egui::Label::new(RichText::new(entry.name).monospace()).wrap());
                ui.add(egui::Label::new(entry.description).wrap());
                ui.add(egui::Label::new(tone_text(entry.example, Tone::Muted).monospace()).wrap());
            });
        }
    }
}

fn warning_line(ui: &mut Ui, warning: &str) {
    ui.horizontal_top(|ui| {
        let (_, rect) = ui.allocate_space(egui::Vec2::splat(16.0));
        widgets::paint_glyph(
            ui,
            Icon::Warning,
            16.0,
            rect.center(),
            theme::palette().warning,
        );
        ui.add(egui::Label::new(theme::body(warning)).wrap());
    });
}

fn curl<'a>(ui: &mut Ui, text: &mut String) -> Footer<'a> {
    let caption = ui.label("curl command");
    let field = ui
        .add(
            egui::TextEdit::multiline(text)
                .code_editor()
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .hint_text("curl 'https://api.example.com/items' -H 'Accept: application/json'"),
        )
        .labelled_by(caption.id);
    initial_focus(ui, &field);
    let parsed = (!text.trim().is_empty()).then(|| imposter_app::core::curl::parse(text));
    match &parsed {
        Some(Ok(parsed)) => {
            for warning in &parsed.warnings {
                warning_line(ui, warning);
            }
        }
        Some(Err(error)) => {
            ui.label(tone_text(error.to_string(), Tone::Error));
        }
        None => {}
    }
    Footer::new(
        "Open as draft",
        matches!(parsed, Some(Ok(_))).then_some(Outcome::Curl),
    )
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

    pub fn start_run(environment: &str, target: &str) -> Self {
        Self::confirm(
            &format!("Run against {environment}?"),
            format!("This sends every request in \"{target}\" with the {environment} environment."),
            "Start run",
            ConfirmPurpose::StartRun,
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

    /// `requests` names whose jar it is ("requests sent with Dev").
    pub fn remove_cookie(
        scope: Scope,
        cookie: &imposter_app::http::cookies::StoredCookie,
        requests: &str,
    ) -> Self {
        Self::confirm(
            "Remove cookie",
            format!(
                "Remove the cookie \"{}\" set by {}? {} stop sending it until the server \
                 sets it again.",
                cookie.name,
                cookie.domain,
                capitalized(requests)
            ),
            "Remove cookie",
            ConfirmPurpose::RemoveCookie(scope),
        )
    }

    pub fn clear_cookies(scope: Scope, cookies: usize, requests: &str) -> Self {
        Self::confirm(
            "Clear cookies",
            format!(
                "Remove {} kept for {requests}? Servers set them again when you sign in \
                 again.",
                counted(cookies, "cookie")
            ),
            "Clear cookies",
            ConfirmPurpose::ClearCookies(scope),
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

/// What an import brings that runs code: "The collection, 2 folders and 1 request have
/// scripts; they run when sent."
fn scripted_line(document: &imposter_app::formats::native::NativeDocument) -> Option<String> {
    let total = document.scripted_count();
    if total == 0 {
        return None;
    }
    let requests = document
        .requests()
        .iter()
        .filter(|r| !r.scripts.is_empty())
        .count();
    let collection = usize::from(!document.collection.scripts.is_empty());
    let folders = total - requests - collection;
    let mut parts = Vec::new();
    if collection == 1 {
        parts.push("The collection".to_owned());
    }
    if folders > 0 {
        parts.push(counted(folders, "folder"));
    }
    if requests > 0 {
        parts.push(counted(requests, "request"));
    }
    let what = match parts.as_slice() {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => return None,
    };
    let verb = if total == 1 { "has" } else { "have" };
    Some(format!("{what} {verb} scripts; they run when sent."))
}

/// "1 request", "7 requests".
/// `text` with its first letter in capitals, to start a sentence.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn import<'a>(
    ui: &mut Ui,
    preview: &ImportPreview,
    remove: &mut bool,
    notes_open: &mut bool,
) -> Footer<'a> {
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
    if let Some(line) = scripted_line(&preview.document) {
        ui.label(line);
    }
    if !preview.warnings.is_empty() {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::spacing().xs;
            let icon = if *notes_open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            let toggle = widgets::button(
                ui,
                Variant::Ghost,
                Some(icon),
                &counted(preview.warnings.len(), "note"),
            );
            if toggle.clicked() {
                *notes_open = !*notes_open;
            }
            ui.ctx().accesskit_node_builder(toggle.id, |node| {
                node.set_role(egui::Role::DisclosureTriangle);
                node.set_expanded(*notes_open);
            });
            if *notes_open {
                for warning in &preview.warnings {
                    warning_line(ui, warning);
                }
            }
        });
    }
    if preview.requires_removal {
        widgets::checkbox(
            ui,
            remove,
            "Remove the listed unsupported execution features",
        );
    }
    Footer::new(
        "Import",
        (!preview.requires_removal || *remove).then_some(Outcome::Import),
    )
}

fn export<'a>(
    ui: &mut Ui,
    environments: &mut [(EnvironmentId, String, bool)],
    globals: &mut bool,
    postman: &mut bool,
) -> Footer<'a> {
    widgets::segmented(
        ui,
        "export format",
        postman,
        &[(false, "Native (.imposter.json)"), (true, "Postman v2.1")],
    );
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = theme::spacing().xs;
        ui.label("Include optional environments");
        if environments.is_empty() {
            ui.label(tone_text("No environments", Tone::Muted));
        }
        for (_, label, include) in environments.iter_mut() {
            widgets::checkbox(ui, include, label.as_str());
        }
    });
    widgets::checkbox(ui, globals, "Include global variables");
    warning_line(
        ui,
        "Literal credentials and file paths are included. Uploaded file contents are excluded.",
    );
    Footer::new("Export", Some(Outcome::Export))
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
}
