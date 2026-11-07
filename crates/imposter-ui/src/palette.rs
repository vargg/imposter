//! The command palette (Cmd/Ctrl+K): one fuzzy-searched list of actions, the requests of
//! every collection, and environments. What it offers is captured when it opens; the request
//! index arrives from the database a moment later. Choosing returns an ordinary [`Action`].

use egui::{Key, Ui};
use imposter_app::core::ids::{CollectionId, RequestId};
use imposter_app::core::model::EnvColor;
use imposter_app::store::collections::IndexedRequest;

use crate::app::{Action, Screen, State};
use crate::dialogs::{Dialog, NamePurpose, Outcome};
use crate::shortcuts;
use crate::theme::{Tone, palette};
use crate::tree::NodeId;
use crate::widgets::{name, tone_text};

/// How many matches are drawn; the rest are reached by typing more.
const SHOWN: usize = 200;
/// How many recently opened requests are remembered.
pub const RECENT_LIMIT: usize = 20;

/// Put `id` first in the recent list, once, keeping at most [`RECENT_LIMIT`].
pub fn remember(recent: &mut Vec<RequestId>, id: RequestId) {
    recent.retain(|r| *r != id);
    recent.insert(0, id);
    recent.truncate(RECENT_LIMIT);
}

/// Where Ctrl+Tab starts: the previous request, or (with Shift) the oldest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Actions, requests and environments.
    All,
    /// Requests only, recent ones first (Cmd/Ctrl+P).
    Requests,
    /// Requests only, chosen as the Load screen's target rather than opened.
    LoadTarget,
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
    pub hint: Option<egui::KeyboardShortcut>,
    /// An environment's color, drawn as a dot before the label.
    pub color: Option<EnvColor>,
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
    /// Opened by Ctrl+Tab: Tab steps while Ctrl is held, releasing Ctrl opens the selection.
    pub switcher: bool,
    /// The first selection, made once the requests are known.
    pub start: Option<Step>,
    /// Ctrl was released before the requests arrived; open as soon as they do.
    pub release_pending: bool,
}

impl Palette {
    pub fn open(state: &State, mode: Mode) -> Self {
        Self {
            mode,
            query: String::new(),
            selected: 0,
            fixed: fixed_items(state),
            recent: state.recent.clone(),
            index: None,
            chosen: None,
            switcher: false,
            start: None,
            release_pending: false,
        }
    }

    /// The Ctrl+Tab switcher: requests only, recent first, the previous one (or with Shift
    /// the oldest) selected.
    pub fn switcher(state: &State, step: Step) -> Self {
        Self {
            switcher: true,
            start: Some(step),
            ..Self::open(state, Mode::Requests)
        }
    }

    /// Ctrl+Tab and Ctrl+Shift+Tab move the selection; releasing Ctrl chooses it.
    fn switch(&mut self, ui: &Ui, count: usize) -> bool {
        let (back, forward, held) = ui.input_mut(|i| {
            let ctrl_shift = egui::Modifiers::CTRL.plus(egui::Modifiers::SHIFT);
            let forward = i.consume_key(ctrl_shift, Key::Tab);
            (
                i.consume_key(egui::Modifiers::CTRL, Key::Tab),
                forward,
                i.modifiers.ctrl,
            )
        });
        if count > 0 {
            if let Some(step) = self.start.take() {
                self.selected = match step {
                    Step::Back => 1.min(count - 1),
                    Step::Forward => count - 1,
                };
            }
            if back {
                self.selected = (self.selected + 1) % count;
            }
            if forward {
                self.selected = (self.selected + count - 1) % count;
            }
        }
        if !held {
            self.release_pending = true;
        }
        self.release_pending && count > 0
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
        requests
            .into_iter()
            .map(|request| match self.mode {
                Mode::LoadTarget => Item {
                    action: Action::LoadTarget(crate::load_screen::Target::from_index(request)),
                    ..request_item(request)
                },
                Mode::All | Mode::Requests => request_item(request),
            })
            .collect()
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
        let released = self.switcher && self.switch(ui, count);
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
            Mode::LoadTarget => "Choose a request to load-test",
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
                    let row = row(ui, item, &self.query, position == self.selected);
                    if position == self.selected && (up || down) {
                        row.scroll_to_me(None);
                    }
                    if row.clicked() {
                        clicked = Some(position);
                    }
                }
                if items.is_empty() {
                    let text = if self.index.is_none() && self.mode != Mode::All {
                        "Loading requests…"
                    } else {
                        "Nothing matches"
                    };
                    ui.label(tone_text(text, Tone::Muted));
                }
            });
        if let Some(position) = clicked.or_else(|| (enter || released).then_some(self.selected))
            && let Some(item) = items.get(position)
        {
            self.chosen = Some(item.action.clone());
            *outcome = Outcome::Palette;
        }
    }
}

/// The gap between a request's method column and its name.
const METHOD_GAP: f32 = 8.0;

/// A row's text: the method padded to the longest one (OPTIONS) in its color, a gap, then
/// the label and its hint.
/// A row's text; the characters of the label that `query` matched are semibold.
fn row_text(
    item: &Item,
    query: &str,
    mono: egui::FontId,
    body: &egui::FontId,
) -> egui::text::LayoutJob {
    let mut text = egui::text::LayoutJob::default();
    let mut gap = 0.0;
    if let Some(method) = &item.method {
        text.append(
            &format!("{method:<7}"),
            0.0,
            egui::TextFormat::simple(mono, crate::theme::method_color(method)),
        );
        gap = METHOD_GAP;
    }
    if let Some(color) = item.color {
        text.append(
            "● ",
            0.0,
            egui::TextFormat::simple(body.clone(), crate::theme::env_color(color)),
        );
    }
    // Positions in the haystack the items are matched against: the method, a space, then
    // the label.
    let offset = item.method.as_ref().map_or(0, |m| m.chars().count() + 1);
    let haystack = match &item.method {
        Some(method) => format!("{method} {}", item.label),
        None => item.label.clone(),
    };
    let matched: Vec<usize> = fuzzy_match(query, &haystack)
        .map(|(_, positions)| positions)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.checked_sub(offset))
        .collect();
    let strong = crate::theme::semibold(body.size);
    for (i, c) in item.label.chars().enumerate() {
        let font = if matched.contains(&i) {
            strong.clone()
        } else {
            body.clone()
        };
        text.append(
            c.encode_utf8(&mut [0; 4]),
            if i == 0 { gap } else { 0.0 },
            egui::TextFormat::simple(font, palette().text),
        );
    }
    text
}

/// A row as wide as the palette; its shortcut, if any, at the right.
fn row(ui: &mut Ui, item: &Item, query: &str, selected: bool) -> egui::Response {
    let text = row_text(
        item,
        query,
        egui::TextStyle::Monospace.resolve(ui.style()),
        &egui::TextStyle::Body.resolve(ui.style()),
    );
    // Drawn as key caps, as in the shortcut sheet.
    let keys = item
        .hint
        .map(|shortcut| shortcuts::keys(ui.ctx(), &shortcut))
        .unwrap_or_default();
    let caps = egui::Atom::paint(crate::widgets::caps_size(ui, &keys), move |ui, args| {
        crate::widgets::paint_caps(ui, args.rect, &keys);
    });
    let response = ui.add_sized(
        [ui.available_width(), ui.spacing().interact_size.y],
        egui::Button::selectable(selected, (text, egui::Atom::grow(), caps))
            .stroke(egui::Stroke::NONE)
            .truncate(),
    );
    if selected {
        crate::widgets::selection_bar(ui, response.rect);
    }
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
        color: None,
        action: Action::OpenIn {
            collection: request.collection_id,
            request: request.id,
        },
    }
}

/// Actions and environments that apply now.
fn fixed_items(state: &State) -> Vec<Item> {
    let mut items = Vec::new();
    let mut action = |label: &str, shortcut: Option<&egui::KeyboardShortcut>, action: Action| {
        items.push(Item {
            group: Group::Actions,
            label: label.to_owned(),
            method: None,
            hint: shortcut.copied(),
            color: None,
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
        "Open Result File…",
        None,
        Action::History(crate::app::HistoryAction::OpenResult),
    );
    action(
        crate::workspace::PASTE_CURL,
        None,
        Action::Prompt(Dialog::Curl {
            text: String::new(),
        }),
    );
    if state.editor.is_some() {
        action("Send Request", Some(&shortcuts::SEND), Action::Send);
        action("Save Request", Some(&shortcuts::SAVE), Action::Save);
    }
    if state
        .editor
        .as_ref()
        .is_some_and(|p| p.editor.body_type == imposter_app::core::model::BodyType::Json)
    {
        action(
            "Format JSON Body",
            Some(&shortcuts::FORMAT_BODY),
            Action::FormatBody,
        );
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
        if let Some(target) = crate::load_screen::Target::from_tree(state, id) {
            action("Load Test This Request", None, Action::LoadTarget(target));
        }
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
        (Screen::Load, "Go to Load"),
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
    let mut environment = |label: String, color: Option<EnvColor>, action: Action| {
        items.push(Item {
            group: Group::Environments,
            label,
            method: None,
            hint: None,
            color,
            action,
        });
    };
    if state.active_environment.is_some() {
        environment(
            // The group's heading says these are environments.
            "Global (no environment)".to_owned(),
            None,
            Action::SetEnvironment(None),
        );
    }
    for env in &state.environments {
        if state.active_environment != Some(env.id) {
            environment(
                env.name.clone(),
                env.color,
                Action::SetEnvironment(Some(env.id)),
            );
        }
    }
    items
}

/// The least a match must score per query character; a letter alone, at no word start and
/// in no run, scores 1.
const MIN_PER_CHARACTER: i64 = 2;

/// How well `query` matches `text`, or `None`: its characters must appear in order, case
/// ignored, spaces in the query skipped. Matches at word starts, at the very start and in
/// runs of adjacent characters score higher, so "lu" prefers "List users" to "Values".
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    fuzzy_match(query, text).map(|(score, _)| score)
}

/// [`fuzzy_score`] with the positions (in characters of `text`) of its best alignment:
/// every placement is considered, not the first, so "ord" finds "Order" rather than
/// scattered letters in "Keyboard Shortcuts". Of equal placements the last wins, since a
/// label ends with its own name ("Orders › Order"). A match averaging under
/// [`MIN_PER_CHARACTER`] is no match: letters scattered without word starts or runs (the
/// "o", "r" and "d" of "Shop › Broken endpoint") listed noise beside the real one.
pub fn fuzzy_match(query: &str, text: &str) -> Option<(i64, Vec<usize>)> {
    let text: Vec<char> = text
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let worth = |i: usize| {
        let mut score = 1;
        if i == 0 || !text[i - 1].is_alphanumeric() {
            score += 8;
        }
        if i == 0 {
            score += 6;
        }
        score
    };
    // best[j][i]: the best score of query[..=j] with query[j] at text[i], and where
    // query[j - 1] was.
    let mut best: Vec<Vec<Option<(i64, usize)>>> = vec![vec![None; text.len()]; query.len()];
    for (i, c) in text.iter().enumerate() {
        if *c == query[0] {
            best[0][i] = Some((worth(i), 0));
        }
    }
    for j in 1..query.len() {
        // The best placement of query[j - 1] strictly before i - 1, kept as i grows.
        let mut earlier: Option<(i64, usize)> = None;
        for i in 1..text.len() {
            if i >= 2
                && let Some((score, _)) = best[j - 1][i - 2]
                && earlier.is_none_or(|(e, _)| score > e)
            {
                earlier = Some((score, i - 2));
            }
            if text[i] != query[j] {
                continue;
            }
            let adjacent = best[j - 1][i - 1].map(|(score, _)| (score + 5, i - 1));
            let candidate = match (adjacent, earlier) {
                (Some(a), Some(e)) => Some(if a.0 >= e.0 { a } else { e }),
                (a, e) => a.or(e),
            };
            best[j][i] = candidate.map(|(score, from)| (score + worth(i), from));
        }
    }
    let last = query.len() - 1;
    let (mut at, (score, _)) = best[last]
        .iter()
        .enumerate()
        .filter_map(|(i, cell)| cell.map(|c| (i, c)))
        .max_by_key(|(i, (score, _))| (*score, *i))?;
    if score < MIN_PER_CHARACTER * i64::try_from(query.len()).unwrap_or(i64::MAX) {
        return None;
    }
    let mut positions = vec![at];
    for j in (1..=last).rev() {
        let (_, from) = best[j][at]?;
        at = from;
        positions.push(at);
    }
    positions.reverse();
    Some((score, positions))
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
    fn the_best_alignment_wins_over_the_first_letters_found() {
        let order = fuzzy_score("ord", "GET Shop API › Orders › Order {{user_id}}").unwrap();
        let keyboard = fuzzy_score("ord", "Keyboard Shortcuts").unwrap();
        assert!(order > keyboard, "{order} vs {keyboard}");
        assert_eq!(
            fuzzy_match("ord", "Keyboard Order").unwrap().1,
            [9, 10, 11],
            "the run at a word start, not keybOaRD"
        );
        assert_eq!(fuzzy_match("lu", "List users").unwrap().1, [0, 5]);
        assert_eq!(
            fuzzy_match("ord", "Shop › Orders › Order").unwrap().1,
            [16, 17, 18],
            "a tie goes to the name at the end, not the folder"
        );
        assert!(
            fuzzy_match("ord", "GET Shop API › Broken endpoint").is_none(),
            "scattered letters are no match"
        );
    }

    #[test]
    fn request_names_start_after_the_method_column_with_a_gap() {
        let mono = egui::FontId::monospace(12.5);
        let body = egui::FontId::proportional(13.0);
        for method in ["GET", "DELETE", "OPTIONS"] {
            let mut item = request_item(&indexed(1, "API", &[], "Users"));
            item.method = Some(method.to_owned());
            let job = row_text(&item, "", mono.clone(), &body);
            assert_eq!(&job.text[..7], format!("{method:<7}"), "one column width");
            assert_eq!(job.sections[1].leading_space, METHOD_GAP, "{method}");
            assert_eq!(
                job.sections[0].format.color,
                crate::theme::method_color(method)
            );
        }
        let mut action = request_item(&indexed(1, "API", &[], "Users"));
        action.method = None;
        let action = row_text(&action, "", mono, &body);
        assert_eq!(action.sections[0].leading_space, 0.0, "no method, no gap");
    }

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
                color: None,
                action: Action::Screen(Screen::History),
            }],
            recent: vec![RequestId(3)],
            switcher: false,
            start: None,
            release_pending: false,
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
    fn recent_requests_are_unique_newest_first_and_capped() {
        let mut recent = Vec::new();
        for id in 1..=25 {
            remember(&mut recent, RequestId(id));
        }
        assert_eq!(recent.len(), RECENT_LIMIT);
        assert_eq!(recent[0], RequestId(25));
        remember(&mut recent, RequestId(20));
        assert_eq!(recent[..3], [RequestId(20), RequestId(25), RequestId(24)]);
        assert_eq!(recent.iter().filter(|r| **r == RequestId(20)).count(), 1);
    }

    #[test]
    fn the_group_with_the_best_match_comes_first() {
        let mut searched = palette(Mode::All);
        searched.fixed.push(Item {
            group: Group::Actions,
            label: "Stack Request and Response".into(),
            method: None,
            hint: None,
            color: None,
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
