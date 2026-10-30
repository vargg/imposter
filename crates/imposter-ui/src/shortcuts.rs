//! Application keyboard shortcuts. [`ALL`] is the one list: [`handle`] runs the ones the app
//! owns before any widget draws, the shortcut sheet shows every entry, and a test keeps the
//! README table equal to it. `COMMAND` is Cmd on macOS and Ctrl elsewhere. Find, the
//! inspector and Esc stay with the widgets they act on.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::{Action, Screen, State};
use crate::dialogs::{Dialog, NamePurpose};
use crate::tree::NodeId;

const fn key(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

const COMMAND_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const CTRL_SHIFT: Modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT);

pub const SEND: KeyboardShortcut = key(Modifiers::COMMAND, Key::Enter);
/// The Python app's Run shortcut.
pub const SEND_ALT: KeyboardShortcut = key(Modifiers::COMMAND, Key::R);
pub const SAVE: KeyboardShortcut = key(Modifiers::COMMAND, Key::S);
pub const NEW: KeyboardShortcut = key(Modifiers::COMMAND, Key::N);
pub const FIND: KeyboardShortcut = key(Modifiers::COMMAND, Key::F);
pub const INSPECT: KeyboardShortcut = key(Modifiers::ALT, Key::Enter);
pub const ESCAPE: KeyboardShortcut = key(Modifiers::NONE, Key::Escape);
pub const FOCUS_URL: KeyboardShortcut = key(Modifiers::COMMAND, Key::L);
pub const DUPLICATE: KeyboardShortcut = key(Modifiers::COMMAND, Key::D);
pub const RENAME: KeyboardShortcut = key(Modifiers::NONE, Key::F2);
/// F2 needs Fn on most Mac keyboards; Yaak's alternative.
pub const RENAME_ALT: KeyboardShortcut = key(CTRL_SHIFT, Key::R);
pub const SETTINGS: KeyboardShortcut = key(Modifiers::COMMAND, Key::Comma);
pub const SIDEBAR: KeyboardShortcut = key(Modifiers::COMMAND, Key::B);
pub const SHORTCUTS: KeyboardShortcut = key(COMMAND_SHIFT, Key::Slash);

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Send,
    Save,
    New,
    FocusUrl,
    Duplicate,
    Rename,
    Settings,
    ToggleSidebar,
    Shortcuts,
    /// Handled by the viewer under the pointer.
    Find,
    /// Handled by the focused variable field.
    Inspect,
    /// Handled by the inspector, find bar or suggestion list that is open.
    Close,
}

impl Command {
    /// Whether [`handle`] runs it; the others belong to widgets.
    pub const fn is_global(self) -> bool {
        !matches!(self, Self::Find | Self::Inspect | Self::Close)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    pub keys: &'static [KeyboardShortcut],
    pub command: Command,
    pub description: &'static str,
}

pub const ALL: &[Shortcut] = &[
    Shortcut {
        keys: &[SEND, SEND_ALT],
        command: Command::Send,
        description: "Send (or cancel the running request)",
    },
    Shortcut {
        keys: &[SAVE],
        command: Command::Save,
        description: "Save the current screen: request, variables or settings",
    },
    Shortcut {
        keys: &[NEW],
        command: Command::New,
        description: "New request (a new collection when there is none)",
    },
    Shortcut {
        keys: &[DUPLICATE],
        command: Command::Duplicate,
        description: "Duplicate the open request",
    },
    Shortcut {
        keys: &[RENAME, RENAME_ALT],
        command: Command::Rename,
        description: "Rename the open request",
    },
    Shortcut {
        keys: &[FOCUS_URL],
        command: Command::FocusUrl,
        description: "Select the request URL",
    },
    Shortcut {
        keys: &[SIDEBAR],
        command: Command::ToggleSidebar,
        description: "Show or hide the collections sidebar",
    },
    Shortcut {
        keys: &[SETTINGS],
        command: Command::Settings,
        description: "Open Settings",
    },
    Shortcut {
        keys: &[SHORTCUTS],
        command: Command::Shortcuts,
        description: "Show keyboard shortcuts",
    },
    Shortcut {
        keys: &[FIND],
        command: Command::Find,
        description: "Find in the viewer under the pointer",
    },
    Shortcut {
        keys: &[INSPECT],
        command: Command::Inspect,
        description: "Inspect the variable at the cursor",
    },
    Shortcut {
        keys: &[ESCAPE],
        command: Command::Close,
        description: "Close the inspector, find bar or suggestions",
    },
];

/// `label (shortcut)` for hover texts, in the platform's notation.
pub fn hint(ctx: &egui::Context, label: &str, shortcut: &KeyboardShortcut) -> String {
    format!("{label} ({})", ctx.format_shortcut(shortcut))
}

/// How many modifiers a shortcut needs: egui ignores extra Shift and Alt when matching, so
/// `Ctrl+Shift+R` must be consumed before `Cmd/Ctrl+R`.
fn specificity(shortcut: KeyboardShortcut) -> usize {
    let m = shortcut.modifiers;
    [m.alt, m.shift, m.ctrl, m.command || m.mac_cmd]
        .into_iter()
        .filter(|on| *on)
        .count()
}

/// The app-owned shortcuts pressed this frame, consumed most specific first.
fn pressed(ctx: &egui::Context) -> Vec<Command> {
    let mut keys: Vec<(KeyboardShortcut, Command)> = ALL
        .iter()
        .filter(|s| s.command.is_global())
        .flat_map(|s| s.keys.iter().map(move |k| (*k, s.command)))
        .collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(specificity(*k)));
    let mut fired = Vec::new();
    ctx.input_mut(|i| {
        for (shortcut, command) in keys {
            if i.consume_shortcut(&shortcut) && !fired.contains(&command) {
                fired.push(command);
            }
        }
    });
    fired
}

/// Turn this frame's shortcuts into actions. Nothing applies while a dialog is open.
pub fn handle(state: &mut State, ctx: &egui::Context) {
    if state.dialog.is_some() {
        return;
    }
    for command in pressed(ctx) {
        run(state, command);
    }
}

fn run(state: &mut State, command: Command) {
    let requests = state.screen == Screen::Requests;
    let open = state.editor.as_ref().and_then(|p| p.editor.id());
    match command {
        Command::Save if state.screen != Screen::History => state.actions.push(Action::Save),
        Command::Send if requests && state.editor.is_some() => state.actions.push(Action::Send),
        Command::New if requests => state.actions.push(Action::Prompt(new_dialog(state))),
        Command::Duplicate if requests => {
            if let Some(id) = open {
                state.actions.push(Action::DuplicateRequest(id));
            }
        }
        Command::Rename if requests => {
            if let Some((id, name)) = state
                .editor
                .as_ref()
                .and_then(|p| Some((p.editor.id()?, p.editor.name().to_owned())))
            {
                state.actions.push(Action::Prompt(Dialog::name(
                    "Rename",
                    "Name:",
                    &name,
                    NamePurpose::Rename(NodeId::Request(id)),
                )));
            }
        }
        Command::FocusUrl if requests && state.editor.is_some() => state.focus_url = true,
        Command::ToggleSidebar if requests => {
            state.layout.sidebar_visible = !state.layout.sidebar_visible;
        }
        Command::Settings => state.actions.push(Action::Screen(Screen::Settings)),
        Command::Shortcuts => state.actions.push(Action::Prompt(Dialog::Shortcuts)),
        _ => {}
    }
}

fn new_dialog(state: &State) -> Dialog {
    if state.collection.is_some() {
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
    }
}

/// A shortcut in the README's notation, the same on every platform: `Cmd/Ctrl+Shift+/`.
pub fn notation(shortcut: &KeyboardShortcut) -> String {
    let m = shortcut.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    if m.command || m.mac_cmd {
        parts.push("Cmd/Ctrl");
    }
    if m.ctrl {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(match shortcut.logical_key {
        Key::Escape => "Esc",
        other => other.symbol_or_name(),
    });
    parts.join("+")
}

/// The README's shortcut table rows, generated from [`ALL`].
pub fn readme_rows() -> Vec<String> {
    ALL.iter()
        .map(|s| {
            let keys: Vec<String> = s.keys.iter().map(notation).collect();
            format!("| {} | {} |", keys.join(", "), s.description)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_commands_share_keys() {
        let keys: Vec<String> = ALL
            .iter()
            .flat_map(|s| s.keys.iter().map(notation))
            .collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "{keys:?}");
    }

    #[test]
    fn the_readme_lists_exactly_these_shortcuts() {
        let readme = include_str!("../../../README.md");
        let table: Vec<String> = readme
            .lines()
            .skip_while(|l| !l.starts_with("| Shortcut | Action |"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(str::to_owned)
            .collect();
        assert_eq!(table, readme_rows());
    }

    #[test]
    fn notation_is_platform_neutral() {
        assert_eq!(notation(&SHORTCUTS), "Cmd/Ctrl+Shift+/");
        assert_eq!(notation(&RENAME_ALT), "Ctrl+Shift+R");
        assert_eq!(notation(&ESCAPE), "Esc");
        assert_eq!(notation(&SETTINGS), "Cmd/Ctrl+,");
    }

    #[test]
    fn more_modifiers_are_consumed_first() {
        assert!(specificity(RENAME_ALT) > specificity(SEND_ALT));
        assert!(specificity(SHORTCUTS) > specificity(FIND));
    }
}
