//! Application keyboard shortcuts, handled in one place before any widget draws. `COMMAND` is
//! Cmd on macOS and Ctrl elsewhere. Find stays with each viewer: it applies to the viewer
//! under the pointer.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::{Action, Screen, State};
use crate::dialogs::{Dialog, NamePurpose};

pub const SEND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
/// The Python app's Run shortcut.
pub const SEND_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
pub const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
pub const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);

/// `label (shortcut)` for hover texts, in the platform's notation.
pub fn hint(ctx: &egui::Context, label: &str, shortcut: &KeyboardShortcut) -> String {
    format!("{label} ({})", ctx.format_shortcut(shortcut))
}

/// Turn this frame's shortcuts into actions. Nothing applies while a dialog is open.
pub fn handle(state: &mut State, ctx: &egui::Context) {
    if state.dialog.is_some() {
        return;
    }
    let (save, send, new) = ctx.input_mut(|i| {
        (
            i.consume_shortcut(&SAVE),
            i.consume_shortcut(&SEND) | i.consume_shortcut(&SEND_ALT),
            i.consume_shortcut(&NEW),
        )
    });
    if save && state.screen != Screen::History {
        state.actions.push(Action::Save);
    }
    if state.screen != Screen::Requests {
        return;
    }
    if send && state.editor.is_some() {
        state.actions.push(Action::Send);
    }
    if new {
        let dialog = if state.collection.is_some() {
            Dialog::name(
                "New Request",
                "Name:",
                "",
                NamePurpose::NewRequest {
                    folder: crate::workspace::selected_folder(state),
                },
            )
        } else {
            Dialog::name("New Collection", "Name:", "", NamePurpose::NewCollection)
        };
        state.actions.push(Action::Prompt(dialog));
    }
}
