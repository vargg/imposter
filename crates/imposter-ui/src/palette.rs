//! The command palette (Cmd/Ctrl+K): one fuzzy-searched list of actions, the requests of
//! every collection, and environments. What it offers is captured when it opens; the request
//! index arrives from the database a moment later. Choosing returns an ordinary [`Action`].

use egui::{Key, Ui};
use imposter_app::core::ids::{CollectionId, RequestId};
use imposter_app::store::collections::IndexedRequest;

use crate::app::{Action, Screen, State};
use crate::dialogs::{Dialog, NamePurpose, Outcome};
use crate::shortcuts;
use crate::theme::{PALETTE, Tone};
use crate::tree::NodeId;
use crate::widgets::{name, tone_text};

/// How many matches are drawn; the rest are reached by typing more.
const SHOWN: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Actions, requests and environments.
    All,
    /// Requests only, recent ones first (Cmd/Ctrl+P).
    Requests,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Actions,
    Requests,
    Environments,
}

impl Group {
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Requests => "Requests",
            Self::Environments => "Environments",
        }
    }
}

/// One row: what it says, an optional method tag and shortcut, and what choosing it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub group: Group,
    pub label: String,
    pub method: Option<String>,
    pub hint: Option<String>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub mode: Mode,
    pub query: String,
    /// Index into the visible items.
    pub selected: usize,
    /// Actions and environments available when the palette opened.
    pub fixed: Vec<Item>,
    /// Request ids, most recent first.
    pub recent: Vec<RequestId>,
    /// Every request; `None` until the database answers.
    pub index: Option<Vec<IndexedRequest>>,
    /// What the user chose, taken by the app with [`Outcome::Palette`].
    pub chosen: Option<Action>,
}

impl Palette {
    pub fn open(state: &State, mode: Mode, ctx: &egui::Context) -> Self {
        Self {
            mode,
            query: String::new(),
            selected: 0,
            fixed: fixed_items(state, ctx),
            recent: state.recent.clone(),
            index: None,
            chosen: None,
        }
    }

    /// The rows matching the query, grouped in order and best match first within a group.
    pub fn items(&self) -> Vec<Item> {
        let mut scored: Vec<(Group, i64, usize, Item)> = Vec::new();
        let candidates = self
            .fixed
            .iter()
            .filter(|item| self.mode == Mode::All || item.group == Group::Requests)
            .cloned()
            .chain(self.requests());
        for (order, item) in candidates.enumerate() {
            let haystack = match &item.method {
                Some(method) => format!("{method} {}", item.label),
                None => item.label.clone(),
            };
            if let Some(score) = fuzzy_score(&self.query, &haystack) {
                scored.push((item.group, score, order, item));
            }
        }
        // While typing, the group with the best match comes first, so a strong request match
        // is not buried under a weak action match.
        let best = |group: Group| {
            scored
                .iter()
                .filter(|(g, ..)| *g == group)
                .map(|(_, score, ..)| *score)
                .max()
                .unwrap_or(i64::MIN)
        };
        let rank: Vec<(Group, i64)> = [Group::Actions, Group::Requests, Group::Environments]
            .into_iter()
            .map(|g| {
                (
                    g,
                    if self.query.trim().is_empty() {
                        0
                    } else {
                        best(g)
                    },
                )
            })
            .collect();
        let group_rank = |group: Group| {
            rank.iter()
                .find(|(g, _)| *g == group)
                .map_or(0, |(_, score)| *score)
        };
        scored.sort_by_key(|(group, score, order, _)| {
            (
                std::cmp::Reverse(group_rank(*group)),
                *group,
                std::cmp::Reverse(*score),
                *order,
            )
        });
        scored
            .into_iter()
            .map(|(.., item)| item)
            .take(SHOWN)
            .collect()
    }

    /// Requests in index order, recent ones first.
    fn requests(&self) -> Vec<Item> {
        let Some(index) = &self.index else {
            return Vec::new();
        };
        let rank = |id: RequestId| {
            self.recent
                .iter()
                .position(|r| *r == id)
                .unwrap_or(usize::MAX)
        };
        let mut requests: Vec<&IndexedRequest> = index.iter().collect();
        requests.sort_by_key(|r| rank(r.id));
        requests.into_iter().map(request_item).collect()
    }

    /// Draw the palette inside its modal and report a choice or a close.
    pub fn show(&mut self, ui: &mut Ui, outcome: &mut Outcome) {
        ui.set_width(560.0);
        let items = self.items();
        let (up, down, enter) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
            )
        });
        let count = items.len();
        if count > 0 {
            if down {
                self.selected = (self.selected + 1) % count;
            }
            if up {
                self.selected = (self.selected + count - 1) % count;
            }
        }
        self.selected = self.selected.min(count.saturating_sub(1));
        let hint = match self.mode {
            Mode::All => "Search actions, requests and environments",
            Mode::Requests => "Search requests",
        };
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        name(&field, "Command palette search");
        if !field.has_focus() {
            field.request_focus();
        }
        if field.changed() {
            self.selected = 0;
        }
        ui.add_space(6.0);
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut group = None;
                for (position, item) in items.iter().enumerate() {
                    if group != Some(item.group) {
                        group = Some(item.group);
                        ui.label(tone_text(item.group.heading(), Tone::Muted).small());
                    }
                    let row = row(ui, item, position == self.selected);
                    if position == self.selected && (up || down) {
                        row.scroll_to_me(None);
                    }
                    if row.clicked() {
                        clicked = Some(position);
                    }
                }
                if items.is_empty() {
                    let text = if self.index.is_none() && self.mode == Mode::Requests {
                        "Loading requests…"
                    } else {
                        "Nothing matches"
                    };
                    ui.label(tone_text(text, Tone::Muted));
                }
            });
        if let Some(position) = clicked.or_else(|| enter.then_some(self.selected))
            && let Some(item) = items.get(position)
        {
            self.chosen = Some(item.action.clone());
            *outcome = Outcome::Palette;
        }
    }
}

fn row(ui: &mut Ui, item: &Item, selected: bool) -> egui::Response {
    let mut text = egui::text::LayoutJob::default();
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let body = egui::TextStyle::Body.resolve(ui.style());
    if let Some(method) = &item.method {
        text.append(
            &format!("{method:<7}"),
            0.0,
            egui::TextFormat::simple(mono, PALETTE.accent),
        );
    }
    text.append(
        &item.label,
        0.0,
        egui::TextFormat::simple(body.clone(), PALETTE.text),
    );
    if let Some(hint) = &item.hint {
        text.append(
            hint,
            16.0,
            egui::TextFormat::simple(body, PALETTE.text_muted),
        );
    }
    let response = ui.add(egui::Button::selectable(selected, text).truncate());
    let label = match &item.method {
        Some(method) => format!("{method} {}", item.label),
        None => item.label.clone(),
    };
    name(&response, &label);
    response
}

fn request_item(request: &IndexedRequest) -> Item {
    let mut path = vec![request.collection.as_str()];
    path.extend(request.folders.iter().map(String::as_str));
    path.push(&request.name);
    Item {
        group: Group::Requests,
        label: path.join(" › "),
        method: Some(request.method.clone()),
        hint: None,
        action: Action::OpenIn {
            collection: request.collection_id,
            request: request.id,
        },
    }
}

/// Actions and environments that apply now.
fn fixed_items(state: &State, ctx: &egui::Context) -> Vec<Item> {
    let mut items = Vec::new();
    let mut action = |label: &str, shortcut: Option<&egui::KeyboardShortcut>, action: Action| {
        items.push(Item {
            group: Group::Actions,
            label: label.to_owned(),
            method: None,
            hint: shortcut.map(|s| ctx.format_shortcut(s)),
            action,
        });
    };
    let open = state
        .editor
        .as_ref()
        .and_then(|p| Some((p.editor.id()?, p.editor.name().to_owned())));
    if state.collection.is_some() {
        action(
            "New Request",
            Some(&shortcuts::NEW),
            Action::Prompt(shortcuts::new_dialog(state)),
        );
        action(
            "New Folder",
            None,
            Action::Prompt(Dialog::name(
                "New Folder",
                "Name:",
                "",
                NamePurpose::NewFolder {
                    parent: crate::workspace::selected_folder(state),
                },
            )),
        );
    }
    action(
        "New Collection",
        None,
        Action::Prompt(Dialog::name(
            "New Collection",
            "Name:",
            "",
            NamePurpose::NewCollection,
        )),
    );
    action("Import Collection…", None, Action::Import);
    action(
        "Paste curl Command…",
        None,
        Action::Prompt(Dialog::Curl {
            text: String::new(),
        }),
    );
    if state.editor.is_some() {
        action("Send Request", Some(&shortcuts::SEND), Action::Send);
        action("Save Request", Some(&shortcuts::SAVE), Action::Save);
    }
    if let Some((id, name)) = open {
        action(
            "Rename Request",
            Some(&shortcuts::RENAME),
            Action::Prompt(Dialog::name(
                "Rename",
                "Name:",
                &name,
                NamePurpose::Rename(NodeId::Request(id)),
            )),
        );
        action(
            "Duplicate Request",
            Some(&shortcuts::DUPLICATE),
            Action::DuplicateRequest(id),
        );
    }
    action(
        if state.layout.sidebar_visible {
            "Hide Sidebar"
        } else {
            "Show Sidebar"
        },
        Some(&shortcuts::SIDEBAR),
        Action::ToggleSidebar,
    );
    action(
        match state.layout.orientation {
            crate::layout::Orientation::SideBySide => "Stack Request and Response",
            crate::layout::Orientation::Stacked => "Put Request and Response Side by Side",
        },
        None,
        Action::ToggleLayout,
    );
    for (screen, label) in [
        (Screen::Requests, "Go to Requests"),
        (Screen::Variables, "Go to Variables"),
        (Screen::History, "Go to History"),
        (Screen::Settings, "Go to Settings"),
        (Screen::Mocks, "Go to Mocks"),
    ] {
        if state.screen != screen {
            let shortcut = (screen == Screen::Settings).then_some(&shortcuts::SETTINGS);
            action(label, shortcut, Action::Screen(screen));
        }
    }
    action(
        "Keyboard Shortcuts",
        Some(&shortcuts::SHORTCUTS),
        Action::Prompt(Dialog::Shortcuts),
    );
    let mut environment = |label: String, action: Action| {
        items.push(Item {
            group: Group::Environments,
            label,
            method: None,
            hint: None,
            action,
        });
    };
    if state.active_environment.is_some() {
        environment(
            "Environment: Global (no environment)".to_owned(),
            Action::SetEnvironment(None),
        );
    }
    for env in &state.environments {
        if state.active_environment != Some(env.id) {
            environment(
                format!("Environment: {}", env.name),
                Action::SetEnvironment(Some(env.id)),
            );
        }
    }
    items
}

/// How well `query` matches `text`, or `None`: its characters must appear in order, case
/// ignored, spaces in the query skipped. Matches at word starts and runs of adjacent
/// characters score higher, so "lu" prefers "List users" to "Values".
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0;
    let mut from = 0;
    let mut previous: Option<usize> = None;
    for wanted in query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
    {
        let found = (from..text.len()).find(|&i| text[i] == wanted)?;
        score += 1;
        if found == 0 || !text[found - 1].is_alphanumeric() {
            score += 8;
        }
        if previous.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        previous = Some(found);
        from = found + 1;
    }
    Some(score)
}

/// The collection and request to open for a palette choice; used by tests.
pub fn open_target(action: &Action) -> Option<(CollectionId, RequestId)> {
    match action {
        Action::OpenIn {
            collection,
            request,
        } => Some((*collection, *request)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_in_order_and_prefers_word_starts() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert!(fuzzy_score("xyz", "List users").is_none());
        assert!(fuzzy_score("sul", "List users").is_none(), "order matters");
        let words = fuzzy_score("lu", "List users").unwrap();
        let inside = fuzzy_score("lu", "Values").unwrap();
        assert!(words > inside, "{words} vs {inside}");
        let run = fuzzy_score("use", "List users").unwrap();
        let spread = fuzzy_score("use", "Lunch sessions").unwrap();
        assert!(run > spread, "{run} vs {spread}");
        assert_eq!(
            fuzzy_score("L U", "list users"),
            fuzzy_score("lu", "list users")
        );
    }

    fn indexed(id: i64, collection: &str, folders: &[&str], name: &str) -> IndexedRequest {
        IndexedRequest {
            id: RequestId(id),
            collection_id: CollectionId(1),
            collection: collection.into(),
            folders: folders.iter().map(|f| (*f).to_owned()).collect(),
            name: name.into(),
            method: "GET".into(),
        }
    }

    fn palette(mode: Mode) -> Palette {
        Palette {
            mode,
            query: String::new(),
            selected: 0,
            fixed: vec![Item {
                group: Group::Actions,
                label: "Go to History".into(),
                method: None,
                hint: None,
                action: Action::Screen(Screen::History),
            }],
            recent: vec![RequestId(3)],
            index: Some(vec![
                indexed(1, "API", &[], "Health"),
                indexed(2, "API", &["Users"], "List users"),
                indexed(3, "Admin", &[], "Stats"),
            ]),
            chosen: None,
        }
    }

    fn labels(palette: &Palette) -> Vec<String> {
        palette.items().into_iter().map(|i| i.label).collect()
    }

    #[test]
    fn requests_show_their_path_with_recent_ones_first() {
        let all = palette(Mode::All);
        assert_eq!(
            labels(&all),
            [
                "Go to History",
                "Admin › Stats",
                "API › Health",
                "API › Users › List users"
            ]
        );
        assert_eq!(labels(&palette(Mode::Requests))[0], "Admin › Stats");
        assert!(!labels(&palette(Mode::Requests)).contains(&"Go to History".to_owned()));
    }

    #[test]
    fn the_group_with_the_best_match_comes_first() {
        let mut searched = palette(Mode::All);
        searched.fixed.push(Item {
            group: Group::Actions,
            label: "Stack Request and Response".into(),
            method: None,
            hint: None,
            action: Action::ToggleLayout,
        });
        searched.query = "stats".into();
        assert_eq!(
            labels(&searched),
            ["Admin › Stats", "Stack Request and Response"]
        );
    }

    #[test]
    fn a_query_filters_and_ranks_within_groups() {
        let mut searched = palette(Mode::All);
        searched.query = "users".into();
        assert_eq!(labels(&searched), ["API › Users › List users"]);
        searched.query = "get he".into();
        assert_eq!(
            labels(&searched),
            ["API › Health"],
            "the method is searchable"
        );
        let target = open_target(&searched.items()[0].action);
        assert_eq!(target, Some((CollectionId(1), RequestId(1))));
        searched.index = None;
        searched.query = String::new();
        assert_eq!(
            labels(&searched),
            ["Go to History"],
            "actions before the index loads"
        );
    }
}
