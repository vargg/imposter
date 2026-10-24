//! Modal dialogs. Each returns what the user chose; the app decides what that means.

use egui::{Id, RichText};
use poster_app::core::ids::{CollectionId, FolderId};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmPurpose {
    DeleteCollection(CollectionId),
    Delete(NodeId),
    DiscardOnClose,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Name(NamePurpose, String),
    Confirmed(ConfirmPurpose),
    Guard(GuardChoice),
    Move(NodeId, Option<FolderId>),
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
        let modal = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
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
            }
        });
        if outcome == Outcome::Open && modal.should_close() {
            outcome = match self {
                Self::Guard { .. } => Outcome::Guard(GuardChoice::Cancel),
                _ => Outcome::Closed,
            };
        }
        outcome
    }
}
