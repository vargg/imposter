//! Modal dialogs. Each returns what the user chose; the app decides what that means.

use std::path::PathBuf;

use egui::{Id, RichText};
use poster_app::core::ids::{CollectionId, EnvironmentId, FolderId};
use poster_app::transfer::ImportPreview;

use crate::theme::Tone;
use crate::tree::NodeId;
use crate::widgets::{name, tone_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamePurpose {
    NewCollection,
    NewFolder { parent: Option<FolderId> },
    NewRequest { folder: Option<FolderId> },
    RenameCollection(CollectionId),
    Rename(NodeId),
    SaveDraft(CollectionId),
    NewEnvironment,
    RenameEnvironment(EnvironmentId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmPurpose {
    DeleteCollection(CollectionId),
    Delete(NodeId),
    DeleteEnvironment(EnvironmentId),
    ClearHistory,
    DiscardOnClose,
    /// Write a Postman export despite the listed limitations.
    PostmanExport,
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
    /// Unsaved changes before navigating; `failed` after an autosave or save attempt failed.
    Guard {
        request: String,
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
    /// Cancelled, closed or acknowledged.
    Closed,
}

impl Dialog {
    pub fn name(title: &str, label: &str, value: &str, purpose: NamePurpose) -> Self {
        Self::Name {
            title: title.into(),
            label: label.into(),
            value: value.into(),
            purpose,
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
        let modal = egui::Modal::new(id).show(ctx, |ui| {
            ui.set_min_width(360.0);
            match self {
                Self::Name {
                    title,
                    label,
                    value,
                    purpose,
                } => {
                    ui.heading(title.as_str());
                    let caption = ui.label(label.as_str());
                    let field =
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                    let field = field.labelled_by(caption.id);
                    if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                        field.request_focus();
                    }
                    let valid = !value.trim().is_empty();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let enter =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.add_enabled(valid, egui::Button::new("OK")).clicked()
                            || (valid && enter)
                        {
                            outcome = Outcome::Name(*purpose, value.trim().to_owned());
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Outcome::Closed;
                        }
                    });
                }
                Self::Confirm {
                    title,
                    message,
                    confirm,
                    purpose,
                } => {
                    ui.heading(title.as_str());
                    ui.label(message.as_str());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new(confirm.as_str()).color(crate::theme::PALETTE.error),
                            )
                            .clicked()
                        {
                            outcome = Outcome::Confirmed(*purpose);
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Outcome::Closed;
                        }
                    });
                }
                Self::Guard { request, failed } => {
                    if let Some(error) = failed {
                        ui.heading("Request could not be saved");
                        ui.label(
                            "Retry saving this request, discard its changes, or keep editing.",
                        );
                        ui.label(tone_text(error.as_str(), Tone::Error));
                    } else {
                        ui.heading("Unsaved request changes");
                        ui.label(format!("Save changes to \"{request}\" before continuing?"));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(if failed.is_some() { "Retry" } else { "Save" })
                            .clicked()
                        {
                            outcome = Outcome::Guard(GuardChoice::Save);
                        }
                        if ui.button("Discard").clicked() {
                            outcome = Outcome::Guard(GuardChoice::Discard);
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Outcome::Guard(GuardChoice::Cancel);
                        }
                    });
                }
                Self::VariablesGuard => {
                    ui.heading("Unsaved variables");
                    ui.label("Save variable changes before continuing?");
                    ui.add_space(8.0);
                    if let Some(choice) = guard_buttons(ui) {
                        outcome = Outcome::VariablesGuard(choice);
                    }
                }
                Self::MocksGuard => {
                    ui.heading("Unsaved mocks");
                    ui.label("Save mock configuration before continuing?");
                    if let Some(choice) = guard_buttons(ui) {
                        outcome = Outcome::MocksGuard(choice);
                    }
                }
                Self::SettingsGuard => {
                    ui.heading("Unsaved application settings");
                    ui.label("Save application settings before exiting?");
                    ui.add_space(8.0);
                    if let Some(choice) = guard_buttons(ui) {
                        outcome = Outcome::SettingsGuard(choice);
                    }
                }
                Self::Move {
                    node,
                    options,
                    choice,
                } => {
                    ui.heading("Move");
                    let caption = ui.label("Destination:");
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
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Move").clicked()
                            && let Some((_, target)) = options.get(*choice)
                        {
                            outcome = Outcome::Move(*node, *target);
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Outcome::Closed;
                        }
                    });
                }
                Self::Message {
                    title,
                    message,
                    tone,
                } => {
                    ui.heading(title.as_str());
                    let text = ui.label(tone_text(message.as_str(), *tone));
                    name(&text, message);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        outcome = Outcome::Closed;
                    }
                }
                Self::Companions { .. } => {
                    ui.heading("Companion files");
                    ui.label("Include Postman environment or global JSON files?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            outcome = Outcome::Companions(true);
                        }
                        if ui.button("No").clicked() {
                            outcome = Outcome::Companions(false);
                        }
                    });
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

fn guard_buttons(ui: &mut egui::Ui) -> Option<GuardChoice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.button("Save").clicked() {
            choice = Some(GuardChoice::Save);
        }
        if ui.button("Discard").clicked() {
            choice = Some(GuardChoice::Discard);
        }
        if ui.button("Cancel").clicked() {
            choice = Some(GuardChoice::Cancel);
        }
    });
    choice
}

fn curl(ui: &mut egui::Ui, text: &mut String) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.set_min_width(560.0);
    ui.heading("Import from curl");
    let caption = ui.label("curl command:");
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
    let parsed = (!text.trim().is_empty()).then(|| poster_app::core::curl::parse(text));
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
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let valid = matches!(parsed, Some(Ok(_)));
        if ui
            .add_enabled(valid, egui::Button::new("Open as draft"))
            .clicked()
        {
            outcome = Outcome::Curl;
        }
        if ui.button("Cancel").clicked() {
            outcome = Outcome::Closed;
        }
    });
    outcome
}

fn import(ui: &mut egui::Ui, preview: &ImportPreview, remove: &mut bool) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.set_min_width(520.0);
    ui.heading("Review collection import");
    ui.label(format!("New collection: {}", preview.collection_name));
    let environments = if preview.environment_names.is_empty() {
        "None".to_owned()
    } else {
        preview.environment_names.join(", ")
    };
    ui.label(format!("New environments: {environments}"));
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
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(allowed, egui::Button::new("Import"))
            .clicked()
        {
            outcome = Outcome::Import;
        }
        if ui.button("Cancel").clicked() {
            outcome = Outcome::Closed;
        }
    });
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
        ui.radio_value(postman, false, "Poster (.poster.json)");
        ui.radio_value(postman, true, "Postman v2.1");
    });
    ui.add_space(6.0);
    ui.label("Include optional environments:");
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
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Export").clicked() {
            outcome = Outcome::Export;
        }
        if ui.button("Cancel").clicked() {
            outcome = Outcome::Closed;
        }
    });
    outcome
}
