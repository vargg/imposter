//! The Requests screen. Drawing only: it reads state, edits the editor's form fields in place
//! and records what the user asked for as actions.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::ids::FolderId;
use imposter_app::core::model::BodyType;

use crate::app::{Action, EditorTab, ImposterApp, ResponseTab, Screen, State};
use crate::dialogs::{Dialog, NamePurpose};
use crate::editor::BODY_CHOICES;
use crate::icons::{self, Icon};
use crate::layout::{Orientation, SIDEBAR_WIDTH, WorkspaceLayout};
use crate::theme::{Tone, method_color, palette};
use crate::tree::{Node, NodeId, Place};
use crate::viewer::{self, Language};
use crate::widgets::{self, FileRequest, VarUi, name, tone_text};

const SEPARATOR: f32 = 12.0;
const ENVIRONMENT_WIDTH: f32 = 200.0;

pub fn draw(app: &mut ImposterApp, ui: &mut Ui) {
    let save = app.save_label();
    let save_text = app.save_button_text();
    let executing = app.is_executing();
    let variables_dirty = app.variables_dirty();
    let state = &mut app.state;
    crate::shortcuts::handle(state, ui.ctx());

    egui::Panel::top("screens")
        .frame(
            egui::Frame::new()
                .fill(palette().surface)
                .inner_margin(egui::Margin::symmetric(16, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // The same drawing as the window and Dock icon.
                let logo = state.logo.get_or_insert_with(|| {
                    let size = 96;
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [size, size],
                        &icons::app_icon(96),
                    );
                    ui.ctx()
                        .load_texture("app icon", image, egui::TextureOptions::LINEAR)
                });
                ui.add(egui::Image::new(&*logo).fit_to_exact_size(vec2(34.0, 34.0)));
                let compact = ui.max_rect().width() < 900.0;
                let galley = ui.fonts_mut(|fonts| fonts.layout_job(wordmark(compact)));
                let (rect, brand) = ui.allocate_exact_size(galley.size(), Sense::hover());
                brand.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, "I’mposter")
                });
                ui.painter().galley(rect.min, galley, palette().text);
                ui.add_space(if compact { 8.0 } else { 24.0 });
                for (screen, label) in [
                    (Screen::Requests, "Requests"),
                    (Screen::Variables, "Variables"),
                    (Screen::History, "History"),
                ] {
                    // In the tone of the "Unsaved" state, so it differs from the green dot
                    // of a request tab that holds something.
                    let tab = widgets::tab_with(ui, state.screen == screen, label, None);
                    if screen == Screen::Variables && variables_dirty {
                        // Over the tab's corner, so the tabs after it do not move.
                        widgets::corner_dot(ui, &tab, Tone::Warning, "Unsaved changes");
                    }
                    if tab.clicked() {
                        state.actions.push(Action::Screen(screen));
                    }
                }
                // Draw in reading order so Tab follows the visible controls, even though
                // this group is aligned to the right edge.
                let width = ENVIRONMENT_WIDTH
                    + 8.0
                    + icons::size()
                    + widgets::menu_text_button_width(ui, tools_text(state.screen))
                    + 2.0 * ui.spacing().item_spacing.x;
                ui.add_space((ui.available_width() - width).max(0.0));
                ui.allocate_ui(
                    vec2(ENVIRONMENT_WIDTH, crate::theme::spacing().control),
                    |ui| environment_picker(state, ui),
                );
                ui.add_space(8.0);
                tools_menu(state, ui);
                let settings = icons::button(
                    ui,
                    Icon::Settings,
                    "Settings",
                    state.screen == Screen::Settings,
                );
                if state.settings_screen.dirty() {
                    widgets::corner_dot(ui, &settings, Tone::Warning, "Unsaved changes");
                }
                if settings.clicked() {
                    state.actions.push(Action::Screen(Screen::Settings));
                }
            });
        });
    egui::Panel::bottom("status")
        .frame(
            egui::Frame::new()
                .fill(palette().surface)
                .inner_margin(egui::Margin::symmetric(16, 5)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let toggle = if state.layout.sidebar_visible {
                    "Hide Collections"
                } else {
                    "Show Collections"
                };
                // An action named by what it does, never drawn selected, and set apart from
                // the layout pair: beside it, selected the same way, it read as a third layout.
                if icons::button(ui, Icon::Sidebar, toggle, false).clicked() {
                    state.layout.sidebar_visible = !state.layout.sidebar_visible;
                }
                ui.separator();
                layout_switch(&mut state.layout, ui);
                ui.separator();
                // Reserve the running tools first: a long result must not push Stop's
                // way back to its screen outside the window.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_context(state, ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let now = ui.input(|i| i.time);
                        let pane_shown = state.screen == Screen::Requests && state.editor.is_some();
                        if let Some(seconds) = state.sending_for(now).filter(|_| !pane_shown) {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.add(
                                egui::Label::new(
                                    tone_text(format!("Sending… {seconds:.1} s"), Tone::Muted)
                                        .small(),
                                )
                                .truncate(),
                            );
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(100));
                        } else if let Some(line) = state.status_line(now) {
                            ui.add(
                                egui::Label::new(tone_text(line.text.as_str(), line.tone).small())
                                    .truncate(),
                            )
                            .on_hover_text(&line.text);
                        }
                    });
                });
            });
        });
    if state.screen == Screen::Variables {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette().background)
                    .inner_margin(16),
            )
            .show(ui, |ui| crate::variables_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::History {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette().background)
                    .inner_margin(16),
            )
            .show(ui, |ui| crate::history_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::Mocks {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette().background)
                    .inner_margin(16),
            )
            .show(ui, |ui| crate::mock_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::Load {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette().background)
                    .inner_margin(16),
            )
            .show(ui, |ui| crate::load_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::Settings {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette().background)
                    .inner_margin(16),
            )
            .show(ui, |ui| crate::settings_screen::draw(state, ui));
        return;
    }
    if state.layout.sidebar_visible {
        let panel = egui::Panel::left("sidebar")
            .frame(egui::Frame::new().fill(palette().surface).inner_margin(12))
            .resizable(true)
            .default_size(state.layout.sidebar_width)
            .size_range(SIDEBAR_WIDTH.0..=SIDEBAR_WIDTH.1)
            .show(ui, |ui| sidebar(state, ui));
        state.layout.sidebar_width = panel
            .response
            .rect
            .width()
            .clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1)
            .round();
        // A grip on the edge says the sidebar can be dragged wider, as the splits' do.
        let edge = panel.response.rect;
        let near = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|p| (p.x - edge.right()).abs() < 6.0 && edge.y_range().contains(p.y));
        let handle = Rect::from_center_size(
            egui::pos2(edge.right(), edge.center().y),
            vec2(6.0, edge.height()),
        );
        widgets::split_grip(ui, handle, true, near);
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette().background)
                .inner_margin(16),
        )
        .show(ui, |ui| {
            if state.editor.is_none() {
                nothing_open(state, ui);
                return;
            }
            context_bar(state, ui, save, save_text);
            ui.add_space(8.0);
            draft_banner(state, ui);
            command_bar(state, ui, executing);
            ui.add_space(12.0);
            split(state, ui);
        });
    let name = state.inspect.as_ref().map(|i| i.name.clone());
    let blocked = state.variables.dirty().then_some(state.variables.scope);
    if let Some(action) = widgets::inspector(
        ui.ctx(),
        &mut state.inspect,
        state.snapshot.as_ref(),
        state.snapshot_error.as_deref(),
        blocked,
    ) && let Some(name) = name
    {
        state.actions.push(match action {
            widgets::InspectorAction::Open(scope) => Action::OpenDefinition(scope, name),
            widgets::InspectorAction::Choose { scope, value } => {
                Action::ChooseValue { scope, name, value }
            }
        });
    }
}

// ---- sidebar ------------------------------------------------------------------------------

fn new_menu(state: &mut State, ui: &mut Ui) {
    icons::menu(ui, Icon::Plus, "New", |ui| {
        if ui.button("New Collection").clicked() {
            state.actions.push(Action::Prompt(Dialog::name(
                "New Collection",
                "Name:",
                "",
                NamePurpose::NewCollection,
            )));
        }
        let parent = selected_folder(state);
        let why = "Create a collection first";
        if ui
            .add_enabled(state.collection.is_some(), egui::Button::new("New Folder"))
            .on_disabled_hover_text(why)
            .clicked()
        {
            state.actions.push(Action::Prompt(Dialog::name(
                "New Folder",
                "Name:",
                "",
                NamePurpose::NewFolder { parent },
            )));
        }
        if ui
            .add_enabled(state.collection.is_some(), egui::Button::new("New Request"))
            .on_disabled_hover_text(why)
            .clicked()
        {
            state.actions.push(Action::Prompt(Dialog::name(
                "New Request",
                "Name:",
                "",
                NamePurpose::NewRequest { folder: parent },
            )));
        }
        if state.collection.is_none() {
            // Said where it shows, not only on hover: greyed items alone gave no reason.
            ui.label(tone_text(why, Tone::Muted).small());
        }
        ui.separator();
        if ui.button("Import Collection…").clicked() {
            state.actions.push(Action::Import);
        }
        if ui.button(PASTE_CURL).clicked() {
            state.actions.push(Action::Prompt(Dialog::Curl {
                text: String::new(),
            }));
        }
    });
}

fn sidebar(state: &mut State, ui: &mut Ui) {
    ui.add_space(4.0);
    let current = state
        .collections
        .iter()
        .find(|c| Some(c.id) == state.collection)
        .map(|c| (c.id, c.name.clone()));
    ui.horizontal(|ui| {
        let picker_width =
            ui.available_width() - 2.0 * (icons::size() + ui.spacing().item_spacing.x);
        ui.add_enabled_ui(!state.collections.is_empty(), |ui| {
            ui.set_width(picker_width);
            let picker = egui::ComboBox::from_id_salt("collection")
                .width(picker_width)
                .truncate()
                .selected_text(
                    current
                        .as_ref()
                        .map_or("Select a collection", |(_, name)| name.as_str()),
                )
                .show_ui(ui, |ui| {
                    for collection in &state.collections {
                        if widgets::option(
                            ui,
                            Some(collection.id) == state.collection,
                            &collection.name,
                        )
                        .clicked()
                        {
                            state.actions.push(Action::SelectCollection(collection.id));
                        }
                    }
                });
            name(&picker.response, "Collection");
        });
        new_menu(state, ui);
        if let Some((id, collection_name)) = current {
            icons::menu(ui, Icon::More, "Collection actions", |ui| {
                if ui.button("Rename Collection").clicked() {
                    state.actions.push(Action::Prompt(Dialog::name(
                        "Rename Collection",
                        "Name:",
                        &collection_name,
                        NamePurpose::RenameCollection(id),
                    )));
                }
                if ui.button("Collection Scripts…").clicked()
                    && let Some(collection) = state.collections.iter().find(|c| c.id == id)
                {
                    state.actions.push(Action::Prompt(Dialog::Scripts {
                        target: crate::dialogs::ScriptTarget::Collection(id),
                        owner: format!("Collection \"{}\"", collection.name),
                        scripts: collection.scripts.clone(),
                    }));
                }
                if ui.button("Export Collection…").clicked() {
                    state.actions.push(Action::Export);
                }
                if ui.button("Delete Collection…").clicked() {
                    let roots = state
                        .tree
                        .as_ref()
                        .filter(|t| t.source.collection.id == id)
                        .map(|t| t.roots.as_slice());
                    let dialog = Dialog::delete_collection(id, &collection_name, roots);
                    state.actions.push(Action::Prompt(dialog));
                }
            });
        } else {
            ui.add_enabled_ui(false, |ui| {
                icons::button(ui, Icon::More, "Collection actions", false)
            });
        }
    });
    // An empty sidebar says nothing: the request pane offers what to do.
    if state.collections.is_empty() {
        return;
    }
    ui.add_space(6.0);
    sidebar_filter(state, ui);
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
    let query = crate::filter::Query::parse(&state.sidebar_filter);
    egui::ScrollArea::vertical()
        .id_salt("tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(tree) = &state.tree else {
                ui.label(tone_text("Loading…", Tone::Muted));
                return;
            };
            let mut actions = Vec::new();
            let filtering = !query.is_all();
            let roots = tree.filtered(&query);
            if filtering && roots.is_empty() {
                ui.label(tone_text("No requests match", Tone::Muted));
            }
            let destinations = tree.destinations();
            let view = NodesView {
                selected: state.selected,
                destinations: &destinations,
                filtering,
                dragged: egui::DragAndDrop::payload::<NodeId>(ui.ctx()).map(|node| *node),
                within_dragged: false,
            };
            // Rows as tall as the density says, with a hairline between them.
            ui.spacing_mut().interact_size.y = crate::theme::spacing().row;
            ui.spacing_mut().item_spacing.y = 2.0;
            nodes(ui, &roots, None, view, &mut actions);
            if view.dragged.is_some() {
                scroll_while_dragging(ui);
                let (_, dropped) =
                    ui.dnd_drop_zone::<NodeId, _>(egui::Frame::default().inner_margin(6.0), |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(tone_text(
                            "Drop here to move to the collection root",
                            Tone::Muted,
                        ));
                    });
                if let Some(node) = dropped {
                    actions.push(Action::Move {
                        node: *node,
                        target: None,
                        index: usize::MAX,
                    });
                }
            }
            state.actions.extend(actions);
        });
}

pub(crate) fn selected_folder(state: &State) -> Option<FolderId> {
    match state.selected? {
        NodeId::Folder(id) => Some(id),
        NodeId::Request(id) => state.tree.as_ref()?.request(id)?.folder_id,
    }
}

fn node_menu(
    ui: &mut Ui,
    node: &Node,
    place: Option<Place>,
    destinations: &[(String, Option<FolderId>)],
    actions: &mut Vec<Action>,
) {
    let id = node.id();
    if let Node::Request(request) = node {
        if ui.button("Duplicate").clicked() {
            actions.push(Action::DuplicateRequest(request.id));
        }
        if ui.button("Load test…").clicked() {
            actions.push(Action::LoadRequest(request.id));
        }
    }
    if let Node::Folder { folder, .. } = node {
        if ui.button("New Request here").clicked() {
            actions.push(Action::Prompt(Dialog::name(
                "New Request",
                "Name:",
                "",
                NamePurpose::NewRequest {
                    folder: Some(folder.id),
                },
            )));
        }
        if ui.button("New Folder here").clicked() {
            actions.push(Action::Prompt(Dialog::name(
                "New Folder",
                "Name:",
                "",
                NamePurpose::NewFolder {
                    parent: Some(folder.id),
                },
            )));
        }
        if ui.button("Scripts…").clicked() {
            actions.push(Action::Prompt(Dialog::Scripts {
                target: crate::dialogs::ScriptTarget::Folder(folder.id),
                owner: format!("Folder \"{}\"", folder.name),
                scripts: folder.scripts.clone(),
            }));
        }
        ui.separator();
    }
    if ui.button("Rename").clicked() {
        actions.push(Action::Prompt(Dialog::name(
            "Rename",
            "Name:",
            node.name(),
            NamePurpose::Rename(id),
        )));
    }
    // Not while filtering: hidden siblings would make a step look like nothing happened.
    if let Some(place) = place {
        for (label, index) in [("Move up", place.up()), ("Move down", place.down())] {
            let hint = match label {
                "Move up" => &crate::shortcuts::MOVE_UP,
                _ => &crate::shortcuts::MOVE_DOWN,
            };
            let button = ui
                .add_enabled(index.is_some(), egui::Button::new(label))
                .on_hover_text(crate::shortcuts::hint(ui.ctx(), label, hint));
            if button.clicked()
                && let Some(index) = index
            {
                actions.push(Action::Move {
                    node: id,
                    target: place.parent,
                    index,
                });
            }
        }
    }
    if ui.button("Move…").clicked() {
        let options: Vec<_> = destinations
            .iter()
            .filter(|(_, target)| *target != folder_of(node))
            .cloned()
            .collect();
        actions.push(Action::Prompt(Dialog::Move {
            node: id,
            options,
            choice: 0,
        }));
    }
    if ui.button("Delete…").clicked() {
        actions.push(Action::Prompt(Dialog::delete_node(node)));
    }
}

fn folder_of(node: &Node) -> Option<FolderId> {
    match node {
        Node::Folder { folder, .. } => Some(folder.id),
        Node::Request(_) => None,
    }
}

/// How the tree is drawn this frame.
#[derive(Clone, Copy)]
struct NodesView<'a> {
    selected: Option<NodeId>,
    destinations: &'a [(String, Option<FolderId>)],
    /// Filtered folders are drawn open under their own ids, so the user's open and closed
    /// folders are as they left them when the filter is cleared.
    filtering: bool,
    /// The node being dragged, if any.
    dragged: Option<NodeId>,
    /// Drawing the inside of the dragged folder, where nothing may be dropped.
    within_dragged: bool,
}

/// Where a dragged node would go, by the part of a row under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drop {
    Before,
    After,
    /// Into a folder: first when dropped under an open folder's header, else last.
    Into {
        first: bool,
    },
}

/// The drop for a pointer at `fraction` of a row's height (0 at the top). On a folder the top
/// and bottom quarters go before and after it (under an open folder's header: first inside
/// it), the middle into it; a request has halves. While filtering only "into" is offered,
/// since hidden siblings make "before" ambiguous.
fn drop_zone(fraction: f32, folder: bool, open: bool, filtering: bool) -> Option<Drop> {
    let into = Drop::Into { first: false };
    if filtering {
        return folder.then_some(into);
    }
    Some(if !folder {
        if fraction < 0.5 {
            Drop::Before
        } else {
            Drop::After
        }
    } else if fraction < 0.25 {
        Drop::Before
    } else if fraction <= 0.75 {
        into
    } else if open {
        Drop::Into { first: true }
    } else {
        Drop::After
    })
}

/// The drop under the pointer on `rect`, for a node of `list` at `index` under `parent`, as
/// the target folder and the index among its children counted without the dragged node.
fn drop_target(
    ui: &Ui,
    rect: Rect,
    node: &Node,
    open: bool,
    (list, index, parent): (&[Node], usize, Option<FolderId>),
    view: NodesView<'_>,
) -> Option<(Drop, Option<FolderId>, usize)> {
    let dragged = view.dragged?;
    if view.within_dragged || dragged == node.id() {
        return None;
    }
    let pointer = ui.input(|i| i.pointer.interact_pos())?;
    let fraction = (pointer.y - rect.top()) / rect.height().max(1.0);
    let drop = drop_zone(fraction, folder_of(node).is_some(), open, view.filtering)?;
    // The dragged node's own slot above this row closes up once it leaves.
    let among = |at: usize| {
        let earlier = list[..at.min(list.len())]
            .iter()
            .any(|sibling| sibling.id() == dragged);
        if earlier { at - 1 } else { at }
    };
    Some(match (drop, folder_of(node)) {
        (Drop::Before, _) => (drop, parent, among(index)),
        (Drop::After, _) => (drop, parent, among(index + 1)),
        (Drop::Into { first }, Some(folder)) => {
            (drop, Some(folder), if first { 0 } else { usize::MAX })
        }
        (Drop::Into { .. }, None) => return None,
    })
}

/// Show where a drop would land: a line between rows, or an outline around a folder.
fn paint_drop(ui: &Ui, rect: Rect, drop: Drop) {
    let stroke = egui::Stroke::new(2.0, palette().accent);
    // Rows are 2 points apart: the line sits in the gap.
    match drop {
        Drop::Before => {
            ui.painter().hline(rect.x_range(), rect.top() - 1.0, stroke);
        }
        Drop::After | Drop::Into { first: true } => {
            ui.painter()
                .hline(rect.x_range(), rect.bottom() + 1.0, stroke);
        }
        Drop::Into { first: false } => {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.5, palette().accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Scroll the tree while a node is dragged near its top or bottom edge, so a long tree can
/// be crossed in one drag.
fn scroll_while_dragging(ui: &mut Ui) {
    let Some(pointer) = ui.ctx().pointer_hover_pos() else {
        return;
    };
    let view = ui.clip_rect();
    let edge = crate::theme::spacing().row;
    if !view.x_range().contains(pointer.x) {
        return;
    }
    // Points per second, faster the closer to the edge.
    let speed = 600.0 * ui.input(|i| i.stable_dt).min(0.1);
    let delta = if (view.top() - edge..view.top() + edge).contains(&pointer.y) {
        speed * (1.0 - (pointer.y - view.top() + edge) / (2.0 * edge))
    } else if (view.bottom() - edge..view.bottom() + edge).contains(&pointer.y) {
        -speed * ((pointer.y - view.bottom() + edge) / (2.0 * edge))
    } else {
        return;
    };
    ui.scroll_with_delta(vec2(0.0, delta.max(-speed).min(speed)));
    ui.ctx().request_repaint();
}

/// The filter field above the tree. Cmd/Ctrl+F over the sidebar focuses it; Esc or × clear.
/// The × sits inside the field's right padding, so the field never changes width; while
/// filtering, "5 of 9 requests" shows under it.
fn sidebar_filter(state: &mut State, ui: &mut Ui) {
    let over_sidebar = ui.rect_contains_pointer(ui.max_rect());
    let find = over_sidebar && ui.input_mut(|i| i.consume_shortcut(&crate::shortcuts::FIND));
    let field = ui.add(
        egui::TextEdit::singleline(&mut state.sidebar_filter)
            .hint_text("Filter: text, method:POST, url:…")
            .margin(egui::Margin {
                left: 4,
                right: 26,
                top: 2,
                bottom: 2,
            })
            .desired_width(f32::INFINITY),
    );
    name(&field, "Filter requests");
    if find {
        field.request_focus();
    }
    // Esc also ends the field's focus, possibly in this same frame.
    if (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.sidebar_filter.clear();
    }
    if !state.sidebar_filter.is_empty() {
        // A square as tall as the field at its right end: a full-size target inside it.
        let side = field.rect.height();
        let spot = Rect::from_min_size(field.rect.right_top() - vec2(side, 0.0), vec2(side, side));
        let clear = ui.put(spot, egui::Button::new("×").frame(false));
        name(&clear, "Clear the filter");
        if clear.clicked() {
            state.sidebar_filter.clear();
        }
    }
    let query = crate::filter::Query::parse(&state.sidebar_filter);
    if let Some(tree) = state.tree.as_ref().filter(|_| !query.is_all()) {
        let shown = crate::tree::request_count(&tree.filtered(&query));
        let total = crate::tree::request_count(&tree.roots);
        ui.label(tone_text(format!("{shown} of {total} requests"), Tone::Muted).small());
    }
}

/// The method, URL and Send row: the main control, a little taller than the others.
/// Over a draft: it is in no collection yet, and one click starts saving it to the one
/// selected, as Save does.
fn draft_banner(state: &mut State, ui: &mut Ui) {
    if !state
        .editor
        .as_ref()
        .is_some_and(|pane| pane.editor.is_draft())
    {
        return;
    }
    let collection = state
        .collection
        .and_then(|id| state.collections.iter().find(|c| c.id == id))
        .map(|c| c.name.clone());
    egui::Frame::new()
        .fill(palette().hover)
        .stroke(egui::Stroke::new(1.0, palette().field_border))
        .corner_radius(BAR_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let button = match &collection {
                    Some(name) => format!("Save to {name}"),
                    None => "Save to a collection".to_owned(),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let save = ui.button(&button);
                    name(&save, "Save draft to collection");
                    if save.clicked() {
                        state.actions.push(Action::Save);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new("This draft is not in a collection yet.").truncate(),
                        );
                    });
                });
            });
        });
    ui.add_space(8.0);
}

/// The request bar's corner radius.
const BAR_RADIUS: u8 = 8;

/// Fields and pickers inside the request bar draw no frame of their own: the bar is their
/// edge, and it shows the focus ring.
fn frameless(ui: &mut Ui) {
    let visuals = ui.visuals_mut();
    visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
    visuals.text_edit_bg_color = Some(egui::Color32::TRANSPARENT);
    visuals.selection.stroke = egui::Stroke::NONE;
    let hover = palette().hover;
    for (state, fill) in [
        (&mut visuals.widgets.inactive, egui::Color32::TRANSPARENT),
        (&mut visuals.widgets.hovered, hover),
        (&mut visuals.widgets.active, hover),
        (&mut visuals.widgets.open, hover),
    ] {
        state.bg_stroke = egui::Stroke::NONE;
        state.weak_bg_fill = fill;
        state.bg_fill = fill;
    }
}

/// A short vertical line between two parts of the request bar.
fn divider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.shrink2(vec2(0.0, height * 0.2)).y_range(),
        egui::Stroke::new(1.0, palette().field_border),
    );
}

fn request_bar_height() -> f32 {
    crate::theme::spacing().control + 8.0
}

fn nodes(
    ui: &mut Ui,
    list: &[Node],
    parent: Option<FolderId>,
    view: NodesView<'_>,
    actions: &mut Vec<Action>,
) {
    let NodesView {
        selected,
        destinations,
        filtering,
        ..
    } = view;
    for (index, node) in list.iter().enumerate() {
        let id = node.id();
        let mut open = false;
        let response = match node {
            Node::Folder { folder, children } => {
                let key = if filtering {
                    ui.make_persistent_id(("filtered folder", folder.id.0))
                } else {
                    ui.make_persistent_id(("folder", folder.id.0))
                };
                let mut collapsing =
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        key,
                        true,
                    );
                if filtering {
                    collapsing.set_open(true);
                }
                open = collapsing.is_open();
                let inner = NodesView {
                    within_dragged: view.within_dragged || view.dragged == Some(id),
                    ..view
                };
                let (_, header, _) = collapsing
                    .show_header(ui, |ui| {
                        let button = egui::Button::selectable(
                            selected == Some(id),
                            RichText::new(&folder.name).strong(),
                        )
                        .stroke(egui::Stroke::NONE)
                        .sense(Sense::click_and_drag());
                        let response = ui.add(button);
                        name(&response, &format!("Folder {}", folder.name));
                        if !folder.scripts.is_empty() {
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_description("Has scripts");
                            });
                        }
                        response
                    })
                    .body(|ui| nodes(ui, children, Some(folder.id), inner, actions));
                header.inner
            }
            Node::Request(request) => {
                let mut text = egui::text::LayoutJob::default();
                text.append(
                    &format!("{:<7}", request.method),
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(11.0),
                        color: method_color(&request.method),
                        ..Default::default()
                    },
                );
                // `{{variables}}` in a name are marked as in the editor.
                let font = egui::FontId::proportional(13.0);
                let mut at = 0;
                let mut gap = 4.0;
                let mut part = |text: &mut egui::text::LayoutJob, piece: &str, color| {
                    if !piece.is_empty() {
                        text.append(piece, gap, egui::TextFormat::simple(font.clone(), color));
                        gap = 0.0;
                    }
                };
                for token in crate::vars::tokens(&request.name) {
                    part(
                        &mut text,
                        &request.name[at..token.range.start],
                        palette().text,
                    );
                    part(
                        &mut text,
                        &request.name[token.range.clone()],
                        palette().accent,
                    );
                    at = token.range.end;
                }
                part(&mut text, &request.name[at..], palette().text);
                let response = ui.add_sized(
                    [ui.available_width(), crate::theme::spacing().row],
                    egui::Button::selectable(selected == Some(id), (text, egui::Atom::grow()))
                        .stroke(egui::Stroke::NONE)
                        .truncate()
                        .sense(Sense::click_and_drag()),
                );
                name(&response, &format!("{} {}", request.method, request.name));
                response.on_hover_text(&request.name)
            }
        };
        if selected == Some(id) {
            widgets::selection_bar(ui, response.rect);
        }
        let slot = (list, index, parent);
        if response.dnd_hover_payload::<NodeId>().is_some()
            && let Some((drop, _, _)) = drop_target(ui, response.rect, node, open, slot, view)
        {
            paint_drop(ui, response.rect, drop);
        }
        if let Some(dropped) = response.dnd_release_payload::<NodeId>()
            && let Some((_, target, index)) = drop_target(ui, response.rect, node, open, slot, view)
        {
            actions.push(Action::Move {
                node: *dropped,
                target,
                index,
            });
        }
        response.dnd_set_drag_payload(id);
        if response.clicked() {
            actions.push(match node {
                Node::Folder { .. } => Action::Select(id),
                Node::Request(request) => Action::OpenRequest(request.id),
            });
        }
        let place = (!filtering).then_some(Place {
            parent,
            index,
            count: list.len(),
        });
        response.context_menu(|ui| node_menu(ui, node, place, destinations, actions));
    }
}

// ---- context and command bars -------------------------------------------------------------

/// "Shop API / Users / List users" as a title: the place muted, the name in semibold.
fn breadcrumb(path: &str) -> egui::text::LayoutJob {
    let (place, name) = path
        .rsplit_once(" / ")
        .map_or(("", path), |(place, name)| (place, name));
    let mut job = egui::text::LayoutJob::default();
    let format = |font, color| egui::TextFormat {
        font_id: font,
        color,
        valign: Align::Center,
        ..Default::default()
    };
    if !place.is_empty() {
        job.append(
            &format!("{place} / "),
            0.0,
            format(egui::FontId::proportional(15.0), palette().text_muted),
        );
    }
    job.append(
        name,
        0.0,
        format(crate::theme::semibold(15.0), palette().text),
    );
    job
}

fn context_bar(
    state: &mut State,
    ui: &mut Ui,
    save: Option<widgets::SaveStatus<'static>>,
    save_text: &str,
) {
    ui.horizontal(|ui| {
        let path = match &state.editor {
            // The request pane says what to do.
            None => String::new(),
            Some(pane) => match pane.editor.id() {
                None => format!(
                    "Unsaved draft / {}",
                    if pane.editor.name().is_empty() {
                        "Untitled request"
                    } else {
                        pane.editor.name()
                    }
                ),
                Some(id) => state
                    .tree
                    .as_ref()
                    .and_then(|t| t.request_path(id))
                    .unwrap_or_else(|| pane.editor.name().to_owned()),
            },
        };
        // The right end first, so the path is cut to what is left rather than run under it.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(save) = save {
                // Copy as curl sits between the icon and the word, so a passing "Saving…"
                // moves only the end of the path.
                let mut curl = false;
                let saved = widgets::save_controls_with(ui, &save, save_text, |ui| {
                    curl = icons::button(ui, Icon::Terminal, "Copy as curl", false).clicked();
                });
                if saved {
                    state.actions.push(Action::Save);
                }
                if curl {
                    state.actions.push(Action::CopyCurl);
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(breadcrumb(&path)).truncate())
                    .on_hover_text(&path);
            });
        });
    });
}

fn tools_text(screen: Screen) -> RichText {
    if matches!(screen, Screen::Mocks | Screen::Load) {
        RichText::new("Tools")
            .font(crate::theme::semibold(13.0))
            .color(palette().accent)
    } else {
        RichText::new("Tools").color(palette().text_muted)
    }
}

fn tools_menu(state: &mut State, ui: &mut Ui) {
    let selected = matches!(state.screen, Screen::Mocks | Screen::Load);
    let button = widgets::menu_text_button(ui, tools_text(state.screen), "Tools");
    button
        .ctx
        .accesskit_node_builder(button.id, |node| node.set_selected(selected));
    if state.mocks.dirty() {
        widgets::corner_dot(ui, &button, Tone::Warning, "Unsaved mocks");
    }
    egui::Popup::menu(&button).show(|ui| {
        for (screen, label, running, stopping, dirty) in [
            (
                Screen::Mocks,
                "Mocks",
                state.mocks.run.is_some(),
                state.mocks.run.as_ref().is_some_and(|run| run.stopping),
                state.mocks.dirty(),
            ),
            (
                Screen::Load,
                "Load",
                state.load.running(),
                state.load.run.as_ref().is_some_and(|run| run.stopping),
                false,
            ),
        ] {
            let mut details = Vec::new();
            if stopping {
                details.push("Stopping…");
            } else if running {
                details.push("Running");
            }
            if dirty {
                details.push("Unsaved changes");
            }
            let description = details.join(" · ");
            let mut job = egui::text::LayoutJob::default();
            job.append(
                label,
                0.0,
                egui::TextFormat::simple(
                    egui::TextStyle::Button.resolve(ui.style()),
                    palette().text,
                ),
            );
            if !description.is_empty() {
                job.append(
                    &format!("    {description}"),
                    0.0,
                    egui::TextFormat::simple(
                        egui::TextStyle::Small.resolve(ui.style()),
                        palette().text_muted,
                    ),
                );
            }
            let item = widgets::option(ui, state.screen == screen, job);
            name(&item, label);
            item.ctx.accesskit_node_builder(item.id, |node| {
                node.set_description(description);
            });
            if item.clicked() {
                state.actions.push(Action::Screen(screen));
                ui.close();
            }
        }
    });
}

/// Running tools remain reachable without opening their menu, including during shutdown.
fn status_context(state: &mut State, ui: &mut Ui) {
    if let Some(run) = &state.load.run {
        let (label, hint) = if run.stopping {
            ("Load stopping…", "Load test stopping")
        } else {
            ("Load running", "Load test running")
        };
        if activity_button(ui, label, hint).clicked() {
            state.actions.push(Action::Screen(Screen::Load));
        }
    }
    if let Some(run) = &state.mocks.run {
        let (label, hint) = if run.stopping {
            (
                "Mocks stopping…".to_owned(),
                "Mock server stopping".to_owned(),
            )
        } else if let Some(address) = &run.address {
            let port = address.rsplit(':').next().unwrap_or(address);
            (
                format!("Mocks :{port}"),
                format!("Mock server running at {address}"),
            )
        } else {
            (
                "Mocks starting…".to_owned(),
                "Mock server starting".to_owned(),
            )
        };
        if activity_button(ui, &label, &hint).clicked() {
            state.actions.push(Action::Screen(Screen::Mocks));
        }
    }
}

fn activity_button(ui: &mut Ui, label: &str, hint: &str) -> egui::Response {
    let button = ui.add(
        egui::Button::new(tone_text(format!("● {label}"), Tone::Muted).small())
            .frame(false)
            .min_size(vec2(24.0, crate::theme::spacing().control)),
    );
    name(&button, label);
    button
        .ctx
        .accesskit_node_builder(button.id, |node| node.set_description(hint));
    button.on_hover_text(format!("{hint}. Open its screen."))
}

fn active_environment(state: &State) -> Option<&imposter_app::store::environments::Environment> {
    state
        .active_environment
        .and_then(|id| state.environments.iter().find(|e| e.id == id))
}

fn environment_picker(state: &mut State, ui: &mut Ui) {
    const GLOBAL: &str = "Global (no environment)";
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = palette().text;
    let active = state.active_environment;
    let color = active_environment(state).and_then(|e| e.color);
    // A colored environment fills the picker with its tint, its name semibold in its color
    // after a stripe: the target of every send, readable at a glance and not by hue alone.
    let selected = active_environment(state).map_or_else(
        || widgets::env_text(GLOBAL, None, font.clone(), text),
        |e| match e.color {
            Some(color) => widgets::env_text(
                &e.name,
                Some(color),
                crate::theme::semibold(font.size),
                crate::theme::env_color(color),
            ),
            None => widgets::env_text(&e.name, None, font.clone(), text),
        },
    );
    let combo = ui
        .scope(|ui| {
            if let Some(color) = color {
                let tint = crate::theme::env_color(color).gamma_multiply(widgets::PILL_TINT);
                let looks = &mut ui.visuals_mut().widgets;
                for look in [&mut looks.inactive, &mut looks.hovered, &mut looks.active] {
                    look.weak_bg_fill = tint;
                    look.bg_fill = tint;
                }
                looks.open.weak_bg_fill = tint;
            }
            egui::ComboBox::from_id_salt("environment")
                .width(ENVIRONMENT_WIDTH)
                .truncate()
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    let global = widgets::option(
                        ui,
                        active.is_none(),
                        widgets::env_option_text(GLOBAL, None, font.clone(), text),
                    );
                    name(&global, GLOBAL);
                    if global.clicked() {
                        state.actions.push(Action::SetEnvironment(None));
                    }
                    for environment in &state.environments {
                        let label = widgets::env_option_text(
                            &environment.name,
                            environment.color,
                            font.clone(),
                            text,
                        );
                        let option = widgets::option(ui, active == Some(environment.id), label);
                        name(&option, &environment.name);
                        if option.clicked() {
                            state
                                .actions
                                .push(Action::SetEnvironment(Some(environment.id)));
                        }
                    }
                    ui.separator();
                    if widgets::option(ui, false, "Manage environments…").clicked() {
                        state.actions.push(Action::ManageEnvironments);
                    }
                })
        })
        .inner;
    if let Some(color) = color {
        let rect = combo.response.rect;
        let stripe = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
        ui.painter().rect_filled(
            stripe,
            egui::CornerRadius {
                nw: 2,
                sw: 2,
                ne: 0,
                se: 0,
            },
            crate::theme::env_color(color),
        );
    }
    name(&combo.response, "Environment");
    let hint = active_environment(state).map_or_else(
        || {
            "Global uses global and collection variables without an environment override."
                .to_owned()
        },
        |e| format!("Requests are sent with the {} environment", e.name),
    );
    combo.response.on_hover_text(hint);
}

/// Switch between side by side and stacked. The icon shows what a click switches to.
/// Two buttons, one per layout; the current one is selected. One button showing the other
/// layout's icon read as the current state.
fn layout_switch(layout: &mut WorkspaceLayout, ui: &mut Ui) {
    for (orientation, icon, label) in [
        (
            Orientation::SideBySide,
            Icon::Columns,
            "Put request and response side by side",
        ),
        (
            Orientation::Stacked,
            Icon::Rows,
            "Stack request and response",
        ),
    ] {
        let current = layout.orientation == orientation;
        if icons::button(ui, icon, label, current).clicked() {
            layout.orientation = orientation;
        }
    }
}

fn command_bar(state: &mut State, ui: &mut Ui, executing: bool) {
    let environment = active_environment(state).map(|e| e.name.clone());
    // A colored environment is named on Send itself ("Send to Prod"): the words on the
    // button clicked say where the request goes.
    let target = active_environment(state)
        .filter(|e| e.color.is_some())
        .map(|e| e.name.clone());
    let State {
        editor,
        snapshot,
        inspect,
        actions,
        focus_url,
        ..
    } = state;
    let Some(pane) = editor else { return };
    let mut vars = VarUi {
        snapshot: snapshot.as_ref(),
        inspect,
    };
    let mut url_edited = false;
    let mut edited = false;
    let mut url_focused = false;
    let height = request_bar_height();
    // Method, URL and Send are one control: one edge, a divider between method and URL,
    // and Send filling the right end.
    let bar = egui::Frame::new()
        .fill(palette().input)
        .stroke(egui::Stroke::new(1.0, palette().field_border))
        .corner_radius(BAR_RADIUS)
        .inner_margin(1)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().interact_size.y = height - 2.0;
                frameless(ui);
                edited |=
                    widgets::method_picker(ui, &mut pane.editor.method, &mut pane.custom_method);
                divider(ui, height - 2.0);
                let (label, fill) = match (&target, executing) {
                    (_, true) => ("Cancel".to_owned(), palette().warning),
                    (Some(name), false) => (format!("Send to {name}"), palette().accent),
                    (None, false) => ("Send".to_owned(), palette().accent),
                };
                let label_width = ui
                    .fonts_mut(|f| {
                        f.layout_no_wrap(
                            label.clone(),
                            egui::TextStyle::Button.resolve(ui.style()),
                            egui::Color32::WHITE,
                        )
                    })
                    .size()
                    .x;
                // As wide as its words up to a limit, past which the name is cut; in a
                // narrow window the URL keeps most of the bar.
                let limit = (ui.available_width() * 0.3).clamp(96.0, 168.0);
                let send_width = (label_width + 28.0).clamp(96.0, limit);
                // Narrow only while a custom method is typed beside it at the smallest size.
                let width = (ui.available_width() - send_width).max(48.0);
                let url_id = widgets::var_edit_id(ui, "Request URL");
                if let Some(command) =
                    widgets::take_whole_paste(ui, url_id, &pane.editor.url, |text| {
                        imposter_app::core::curl::parse(text).is_ok()
                    })
                {
                    actions.push(Action::PasteCurl(command));
                }
                let url = widgets::var_edit(
                    ui,
                    &mut pane.editor.url,
                    "Request URL",
                    width,
                    false,
                    &mut vars,
                );
                if std::mem::take(focus_url) {
                    widgets::focus_and_select_all(ui.ctx(), &url, pane.editor.url.chars().count());
                }
                url_edited = url.changed();
                url_focused = url.has_focus();
                let inner = BAR_RADIUS - 1;
                let send = ui.add_sized(
                    vec2(send_width, height - 2.0),
                    egui::Button::new(RichText::new(&label).color(palette().on_accent).strong())
                        .truncate()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius {
                            nw: 0,
                            sw: 0,
                            ne: inner,
                            se: inner,
                        }),
                );
                // Where it goes, for the pointer and for screen readers.
                let action = match (&environment, executing) {
                    (Some(name), false) => format!("Send with {name}"),
                    _ => label.clone(),
                };
                if !executing && let Some(name) = &environment {
                    let description = format!("Sends with the {name} environment");
                    ui.ctx()
                        .accesskit_node_builder(send.id, |node| node.set_description(description));
                }
                let send = send.on_hover_text(crate::shortcuts::hint(
                    ui.ctx(),
                    &action,
                    &crate::shortcuts::SEND,
                ));
                if send.clicked() {
                    actions.push(Action::Send);
                }
            })
        });
    // The focus ring goes around the whole control, as the field's own would.
    if url_focused {
        ui.painter().rect_stroke(
            bar.response.rect,
            BAR_RADIUS,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    if url_edited || edited {
        actions.push(Action::Edited {
            url: url_edited,
            params: false,
        });
    }
}

// ---- request / response split ---------------------------------------------------------------

fn split(state: &mut State, ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    let orientation = state.layout.orientation;
    let ratio = state.layout.ratio(orientation);
    let (first, handle, second) = match orientation {
        Orientation::SideBySide => {
            let width = crate::layout::split(
                area.width() - SEPARATOR,
                ratio,
                crate::layout::MIN_PANE_WIDE,
                crate::layout::MIN_PANE_WIDE,
            );
            let first = Rect::from_min_size(area.min, vec2(width, area.height()));
            let handle = Rect::from_min_size(first.right_top(), vec2(SEPARATOR, area.height()));
            (
                first,
                handle,
                Rect::from_min_max(handle.right_top(), area.max),
            )
        }
        Orientation::Stacked => {
            let height = crate::layout::split(
                area.height() - SEPARATOR,
                ratio,
                crate::layout::MIN_PANE_TALL,
                crate::layout::MIN_PANE_TALL,
            );
            let first = Rect::from_min_size(area.min, vec2(area.width(), height));
            let handle = Rect::from_min_size(first.left_bottom(), vec2(area.width(), SEPARATOR));
            (
                first,
                handle,
                Rect::from_min_max(handle.left_bottom(), area.max),
            )
        }
    };
    let drag = ui.interact(handle, ui.scope_id().with("split"), Sense::drag());
    let cursor = if orientation == Orientation::SideBySide {
        egui::CursorIcon::ResizeHorizontal
    } else {
        egui::CursorIcon::ResizeVertical
    };
    let drag = drag.on_hover_cursor(cursor);
    if drag.dragged() {
        let delta = drag.drag_delta();
        let (moved, total) = match orientation {
            Orientation::SideBySide => (delta.x, area.width() - SEPARATOR),
            Orientation::Stacked => (delta.y, area.height() - SEPARATOR),
        };
        // From the pane's size as drawn, which a minimum may hold away from the ratio, so
        // dragging back responds at once.
        let size = match orientation {
            Orientation::SideBySide => first.width(),
            Orientation::Stacked => first.height(),
        };
        if total > 0.0 {
            state.layout.set_ratio(orientation, (size + moved) / total);
        }
    }
    // Hover is neutral; orange is for primary, selected and focused things.
    widgets::split_grip(
        ui,
        handle,
        orientation == Orientation::SideBySide,
        drag.hovered() || drag.dragged(),
    );
    for rect in [first, second] {
        pane_frame(ui, rect);
    }
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(first.shrink(14.0))
            .id_salt("request pane"),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(first));
            request_pane(state, ui);
        },
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(second.shrink(14.0))
            .id_salt("response pane"),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(second));
            response_pane(state, ui);
        },
    );
    ui.allocate_rect(area, Sense::hover());
}

/// "3" in `tone`, or nothing for none.
fn count_badge(count: usize, tone: Tone) -> Option<widgets::TabBadge> {
    (count > 0).then(|| widgets::TabBadge {
        mark: widgets::Mark::Count(count),
        tone,
        description: format!("{count} in use"),
    })
}

/// A request tab's badge: rows in use, or a dot when it holds something. It describes the
/// request, not its last response, so the Tests count is never red.
fn editor_badge(editor: &crate::editor::Editor, tab: EditorTab) -> Option<widgets::TabBadge> {
    match editor.badge(tab)? {
        crate::editor::Badge::Count(count) => count_badge(count, Tone::Muted),
        // Green, as API clients mark a tab that holds something.
        crate::editor::Badge::Dot => Some(widgets::TabBadge {
            mark: widgets::Mark::Dot,
            tone: Tone::Ok,
            description: "Set".to_owned(),
        }),
    }
}

/// The request pane with no request open: what to do next, as buttons. On first run
/// that is a collection to create, import or start from curl.
fn empty_request_pane(state: &mut State, ui: &mut Ui) {
    ui.add_space((ui.available_height() * 0.2).min(80.0));
    let first_run = state.collections.is_empty();
    let empty = state
        .tree
        .as_ref()
        .is_some_and(|tree| tree.roots.is_empty());
    let (title, text) = if first_run {
        (
            "Start with a collection",
            "Create one, import a Postman, OpenAPI or Imposter file, or paste a curl command.",
        )
    } else if empty {
        (
            "This collection is empty",
            "Add a request, or paste a curl command.",
        )
    } else {
        (
            "Select a request",
            "Choose one in the sidebar, or start a new one.",
        )
    };
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        ui.add(egui::Label::new(tone_text(text, Tone::Muted)).wrap());
        ui.add_space(10.0);
    });
    let mut offers = Vec::new();
    if first_run {
        offers.push((
            "New Collection",
            Action::Prompt(Dialog::name(
                "New Collection",
                "Name:",
                "",
                NamePurpose::NewCollection,
            )),
        ));
        offers.push(("Import Collection…", Action::Import));
    } else if state.collection.is_some() {
        offers.push((
            "New Request",
            Action::Prompt(crate::shortcuts::new_dialog(state)),
        ));
    }
    offers.push((
        PASTE_CURL,
        Action::Prompt(Dialog::Curl {
            text: String::new(),
        }),
    ));
    if let Some(action) = offer_buttons(ui, offers) {
        state.actions.push(action);
    }
}

/// Centred under the text that introduces them, the first filled as the next step; one
/// above the other when they do not fit side by side.
fn offer_buttons(ui: &mut Ui, offers: Vec<(&str, Action)>) -> Option<Action> {
    let gap = ui.spacing().item_spacing.x;
    let widths: Vec<f32> = offers
        .iter()
        .map(|(label, _)| widgets::button_width(ui, RichText::new(*label)))
        .collect();
    let total = widths.iter().map(|width| width + gap).sum::<f32>() - gap;
    let button = |ui: &mut Ui, index: usize, label: &str| {
        let button = if index == 0 {
            egui::Button::new(RichText::new(label).color(palette().on_accent))
                .fill(palette().accent)
        } else {
            egui::Button::new(label)
        };
        ui.add(button).clicked()
    };
    let mut chosen = None;
    if total <= ui.available_width() {
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - total) / 2.0);
            for (index, (label, action)) in offers.into_iter().enumerate() {
                if button(ui, index, label) {
                    chosen = Some(action);
                }
            }
        });
    } else {
        ui.vertical_centered(|ui| {
            for (index, (label, action)) in offers.into_iter().enumerate() {
                if button(ui, index, label) {
                    chosen = Some(action);
                }
            }
        });
    }
    chosen
}

/// The one name of starting a request from a curl command.
pub const PASTE_CURL: &str = "Paste curl…";

/// With no request open, one pane over the whole area says what to do next: an empty
/// request bar and response pane beside it read as a failed render.
fn nothing_open(state: &mut State, ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    pane_frame(ui, area);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(area.shrink(14.0))
            .id_salt("request pane"),
        |ui| empty_request_pane(state, ui),
    );
    ui.allocate_rect(area, Sense::hover());
}

fn pane_frame(ui: &Ui, rect: Rect) {
    ui.painter().rect(
        rect,
        PANE_RADIUS,
        palette().surface,
        egui::Stroke::new(1.0, palette().border),
        egui::StrokeKind::Inside,
    );
}

const PANE_RADIUS: f32 = 12.0;

/// The collection's and enclosing folders' scripts around the open request, outermost
/// first, as the executor runs them; only levels that have scripts.
fn outer_scripts(state: &State) -> Vec<crate::scripts_tab::Outer> {
    use crate::dialogs::ScriptTarget;
    use crate::scripts_tab::Outer;
    let Some(identity) = state.editor.as_ref().map(|p| p.editor.identity()) else {
        return Vec::new();
    };
    let Some(collection) = identity
        .collection_id
        .and_then(|id| state.collections.iter().find(|c| c.id == id))
    else {
        return Vec::new();
    };
    let mut levels = vec![Outer {
        owner: format!("Collection \"{}\"", collection.name),
        target: ScriptTarget::Collection(collection.id),
        scripts: collection.scripts.clone(),
    }];
    let mut folders = Vec::new();
    let mut next = identity.folder_id;
    let tree = state
        .tree
        .as_ref()
        .filter(|t| t.source.collection.id == collection.id);
    while let (Some(id), Some(tree)) = (next, tree) {
        let Some(folder) = tree.folder(id) else { break };
        if folders.len() > tree.source.folders.len() {
            break;
        }
        folders.push(Outer {
            owner: format!("Folder \"{}\"", folder.name),
            target: ScriptTarget::Folder(folder.id),
            scripts: folder.scripts.clone(),
        });
        next = folder.parent_id;
    }
    levels.extend(folders.into_iter().rev());
    levels.retain(|level| !level.scripts.is_empty());
    levels
}

fn request_pane(state: &mut State, ui: &mut Ui) {
    // Where captures of names nobody defines go: the active environment, else Global.
    let capture_fallback = active_environment(state).map(|e| e.name.clone());
    let outer = outer_scripts(state);
    let State {
        editor,
        snapshot,
        inspect,
        settings,
        actions,
        ..
    } = state;
    let Some(pane) = editor else {
        return;
    };
    let mut vars = VarUi {
        snapshot: snapshot.as_ref(),
        inspect,
    };
    let tabs = EditorTab::ALL.map(|(tab, label)| {
        let spec = widgets::TabSpec::new(tab, label).badge(editor_badge(&pane.editor, tab));
        if tab == EditorTab::Settings {
            // The Settings screen tab has the same text; screen readers need them apart.
            spec.named("Request settings")
        } else {
            spec
        }
    });
    ui.horizontal(|ui| widgets::tab_strip(ui, &tabs, &mut pane.tab, 0.0, "More request tabs"));
    ui.separator();
    let mut edited = false;
    let mut params = false;
    let mut pick = None;
    egui::ScrollArea::both()
        .id_salt(("editor", pane.tab as u8))
        .auto_shrink([false, false])
        .show(ui, |ui| match pane.tab {
            EditorTab::Docs => {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut pane.editor.description)
                        .desired_width(f32::INFINITY)
                        .desired_rows(12),
                );
                name(&response, "Request description");
                edited = response.changed();
            }
            EditorTab::Params => {
                params = widgets::rows_table(
                    ui,
                    "Query",
                    &mut pane.editor.queries,
                    &mut pane.confirm_clear[0],
                    &mut vars,
                );
            }
            EditorTab::Headers => {
                edited = widgets::rows_table(
                    ui,
                    "Headers",
                    &mut pane.editor.headers,
                    &mut pane.confirm_clear[1],
                    &mut vars,
                );
            }
            EditorTab::Cookies => {
                edited = widgets::rows_table(
                    ui,
                    "Cookies",
                    &mut pane.editor.cookies,
                    &mut pane.confirm_clear[2],
                    &mut vars,
                );
            }
            EditorTab::Auth => edited = widgets::auth_form(ui, &mut pane.editor.auth, &mut vars),
            EditorTab::Tests => {
                edited = crate::checks::checks_table(ui, &mut pane.editor.checks, &mut vars);
            }
            EditorTab::Scripts => {
                edited = crate::scripts_tab::draw(ui, &mut pane.editor.scripts, &outer, actions);
            }
            EditorTab::Captures => {
                edited = crate::checks::captures_table(
                    ui,
                    &mut pane.editor.captures,
                    vars.snapshot,
                    capture_fallback.as_deref(),
                );
            }
            EditorTab::Settings => {
                edited = widgets::options_form(
                    ui,
                    &mut pane.editor.options,
                    settings,
                    &pane.invalid,
                    &mut vars,
                );
            }
            EditorTab::Body => {
                ui.horizontal(|ui| {
                    let caption = ui.label("Body type");
                    let selected = BODY_CHOICES
                        .iter()
                        .find(|(t, _)| *t == pane.editor.body_type)
                        .map_or("JSON", |(_, l)| *l);
                    let combo = egui::ComboBox::from_id_salt("body type")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (body_type, label) in BODY_CHOICES {
                                edited |= ui
                                    .selectable_value(&mut pane.editor.body_type, body_type, label)
                                    .changed();
                            }
                        });
                    combo.response.labelled_by(caption.id);
                });
                match pane.editor.body_type {
                    BodyType::None => {
                        ui.label(tone_text("This request has no body", Tone::Muted));
                    }
                    BodyType::Json | BodyType::Plain | BodyType::Text => {
                        let json = pane.editor.body_type == BodyType::Json;
                        edited |=
                            widgets::body_editor(ui, &mut pane.editor.body_text, json, &mut vars);
                    }
                    BodyType::Urlencoded | BodyType::FormData => {
                        edited |= widgets::rows_table(
                            ui,
                            "Fields",
                            &mut pane.editor.form,
                            &mut pane.confirm_clear[3],
                            &mut vars,
                        );
                    }
                    BodyType::Multipart => {
                        edited |= widgets::multipart_table(
                            ui,
                            &mut pane.editor.multipart,
                            &mut pane.confirm_clear[4],
                            &mut pick,
                            &mut vars,
                        );
                    }
                    BodyType::Binary => {
                        ui.horizontal(|ui| {
                            let width = ui.available_width() - 110.0;
                            edited |= widgets::var_edit(
                                ui,
                                &mut pane.editor.binary_path,
                                "Binary file path",
                                width,
                                false,
                                &mut vars,
                            )
                            .changed();
                            if ui.button("Choose file…").clicked() {
                                pick = Some(FileRequest::Binary);
                            }
                        });
                    }
                }
            }
        });
    if let Some(target) = pick {
        actions.push(Action::PickFile(target));
    }
    if edited || params {
        actions.push(Action::Edited { url: false, params });
    }
}

fn response_pane(state: &mut State, ui: &mut Ui) {
    // Without a request there is nothing to send: the request pane says what to do.
    if state.editor.is_none() {
        return;
    }
    let now = ui.input(|i| i.time);
    if let Some(seconds) = state.sending_for(now) {
        let sending = format!("Sending… {seconds:.1} s");
        if state.response.preview.is_none() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(20.0));
                ui.label(RichText::new(sending).strong());
                ui.label(tone_text("Cancel stops it.", Tone::Muted));
            });
            return;
        }
        state.response.notice =
            Some((format!("{sending} · showing the previous run"), Tone::Muted));
    }
    if let Some(blocked) = state.response.blocked.clone() {
        // Only why nothing went out: an earlier response here would read as this send's.
        let target = state
            .active_environment
            .and_then(|id| state.environments.iter().find(|e| e.id == id))
            .map_or((crate::vars::Scope::Global, "Global".to_owned()), |e| {
                (crate::vars::Scope::Environment(e.id), e.name.clone())
            });
        if !state.response.runs.is_empty() {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    runs_menu(ui, &state.response, &mut state.actions);
                });
            });
        }
        blocked_card(ui, &blocked, &target.1, |action| {
            state.actions.push(match action {
                BlockedAction::Define(name) => Action::DefineVariable(target.0, name),
                BlockedAction::OpenVariables => Action::Screen(Screen::Variables),
            });
        });
        return;
    }
    let response_state = &mut state.response;
    let actions = &mut state.actions;
    // The status line takes the place of a "Response" heading; the runs menu ends it.
    if response_state.response.is_some() || !response_state.runs.is_empty() {
        ui.horizontal(|ui| {
            // As tall as the runs button, so the status centres on it.
            ui.set_min_height(widgets::tab_height());
            let runs = if response_state.runs.is_empty() {
                0.0
            } else {
                widgets::menu_text_button_width(ui, RUNS)
            };
            if let Some(response) = &response_state.response
                && status_line(ui, response, runs)
            {
                response_state.tab = ResponseTab::Tests;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                runs_menu(ui, response_state, actions);
            });
        });
    }
    match response_body(ui, response_state, "response", false, true) {
        Some(PaneAction::Row(widgets::RowAction::AddCheck { path, value })) => actions.push(
            Action::AddCheck(crate::checks::check_for(&path, value.as_deref())),
        ),
        Some(PaneAction::Row(widgets::RowAction::Capture { path })) => {
            actions.push(Action::AddCapture(crate::checks::capture_for(&path)));
        }
        Some(PaneAction::Retry) => actions.push(Action::Send),
        Some(PaneAction::SaveBody) => actions.push(save_body_action(response_state)),
        None => {}
    }
}

/// Save the body of `pane`'s response.
pub fn save_body_action(pane: &crate::app::ResponsePane) -> Action {
    let response = pane.response.as_ref();
    Action::SaveBody {
        bytes: response.map(|r| r.body.clone()).unwrap_or_default(),
        name: response.map_or("response.txt", viewer::body_file_name),
        cut: response.is_some_and(|r| r.truncated),
    }
}

/// What a response pane asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneAction {
    Row(widgets::RowAction),
    /// Send the request again after a failure.
    Retry,
    /// Save the whole body to a file: it is larger than the preview, or was cut.
    SaveBody,
}

enum BlockedAction {
    Define(String),
    OpenVariables,
}

/// `{{a}}`, `{{a}} and {{b}}`, `{{a}}, {{b}} and {{c}}`.
fn placeholder_list(names: &[String]) -> String {
    let tokens: Vec<String> = names.iter().map(|name| format!("{{{{{name}}}}}")).collect();
    match tokens.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Why a send went nowhere, with a way to fix undefined variables.
fn blocked_card(
    ui: &mut Ui,
    blocked: &crate::app::NotSent,
    scope_name: &str,
    mut act: impl FnMut(BlockedAction),
) {
    problem_frame(ui, |ui| {
        let headline = match blocked.unresolved.as_slice() {
            [] => blocked.message.clone(),
            [_] => format!(
                "Not sent: {} is not defined",
                placeholder_list(&blocked.unresolved)
            ),
            _ => format!(
                "Not sent: {} are not defined",
                placeholder_list(&blocked.unresolved)
            ),
        };
        ui.add(egui::Label::new(tone_text(headline, Tone::Error).strong()).wrap());
        if let Some(first) = blocked.unresolved.first() {
            ui.add(
                egui::Label::new(tone_text(
                    "Nothing was sent. Define the variable, or fix the name in the request.",
                    Tone::Muted,
                ))
                .wrap(),
            );
            ui.horizontal_wrapped(|ui| {
                // The headline names the variable; the button says where it goes, which a
                // long name would cut off. Hover and screen readers get both.
                let full = format!("Define {{{{{first}}}}} in {scope_name}");
                let define = ui
                    .add(
                        egui::Button::new(format!("Define in {scope_name}"))
                            .wrap_mode(egui::TextWrapMode::Truncate),
                    )
                    .on_hover_text(full.as_str());
                name(&define, &full);
                if define.clicked() {
                    act(BlockedAction::Define(first.clone()));
                }
                if ui.button("Open Variables").clicked() {
                    act(BlockedAction::OpenVariables);
                }
            });
        }
    });
}

/// A bordered block for a problem, inside the pane's padding and wrapping its text; it
/// scrolls when the pane is too short for it.
fn problem_frame(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .id_salt("problem")
        .auto_shrink([false, true])
        .show(ui, |ui| problem_box(ui, contents));
}

fn problem_box(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, palette().error.gamma_multiply(0.6)))
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            contents(ui);
        });
}

/// A transport failure in plain words, the raw text on request, and Retry when `live`.
fn failure_card(
    ui: &mut Ui,
    response: &imposter_app::core::execution::Response,
    id: &str,
    live: bool,
) -> bool {
    let mut retry = false;
    problem_frame(ui, |ui| {
        ui.add(
            egui::Label::new(
                tone_text(viewer::failure_title(&response.error_kind), Tone::Error).strong(),
            )
            .wrap(),
        );
        if let Some(hint) = viewer::failure_hint(&response.error_kind) {
            ui.add(egui::Label::new(tone_text(hint, Tone::Muted)).wrap());
        }
        egui::CollapsingHeader::new("Details")
            .id_salt((id, "failure details"))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(response.reason.as_str()).monospace())
                        .wrap()
                        .selectable(true),
                );
            });
        if live && ui.button("Retry").clicked() {
            retry = true;
        }
    });
    retry
}

const RUNS: &str = "Runs";

/// The recorded runs of the open request, to show an earlier response.
fn runs_menu(ui: &mut Ui, response_state: &crate::app::ResponsePane, actions: &mut Vec<Action>) {
    if response_state.runs.is_empty() {
        return;
    }
    let shown = response_state.shown_run;
    let hint = response_state
        .runs
        .iter()
        .find(|run| Some(run.id) == shown)
        .map_or_else(
            || "Show an earlier run".to_owned(),
            |run| format!("Showing {}", run_label(run)),
        );
    let button = widgets::menu_text_button(ui, RUNS, "Recorded runs").on_hover_text(hint);
    egui::Popup::menu(&button).show(|ui| {
        for run in &response_state.runs {
            if widgets::option(ui, Some(run.id) == shown, run_label(run)).clicked() {
                actions.push(Action::ShowRun(run.id));
                ui.close();
            }
        }
    });
}

/// A part of a response's status line.
enum StatusPart {
    Pill(String, Tone),
    Text(RichText),
    Checks(crate::checks::Tally),
}

/// The response headers' view toggle.
const RAW: &str = "Raw";

/// One line about a response: its status as a pill, then what fits of the checks, time,
/// size and content type, dropped in reverse order of importance. The status's hover text
/// has them all. `reserve` keeps room at the end of the row. Returns whether the checks were
/// clicked, which asks for their results.
fn status_line(
    ui: &mut Ui,
    response: &imposter_app::core::execution::Response,
    reserve: f32,
) -> bool {
    let status = viewer::status(response);
    // In order of importance. A transport failure has no status for a pill: the card under
    // the line names it, and a pill would only repeat it.
    let failed = response.outcome == imposter_app::core::execution::Outcome::TransportError;
    let mut parts = Vec::new();
    if !failed {
        parts.push(StatusPart::Pill(status.text.clone(), status.tone));
    }
    let tally = crate::checks::Tally::of_response(response);
    if !tally.is_empty() {
        parts.push(StatusPart::Checks(tally));
    }
    let time = viewer::format_duration(response.elapsed);
    parts.push(StatusPart::Text(tone_text(time.as_str(), Tone::Muted)));
    let delivered = response.outcome == imposter_app::core::execution::Outcome::Response;
    let size = viewer::size_text(response);
    if delivered {
        parts.push(StatusPart::Text(tone_text(size.as_str(), Tone::Muted)));
    }
    let content_type = viewer::content_type(&response.headers);
    let kind = content_type.split(';').next().unwrap_or_default().trim();
    if delivered && !kind.is_empty() {
        parts.push(StatusPart::Text(tone_text(kind, Tone::Muted)));
    }
    let mut hover = vec![status.detail.clone()];
    if !tally.is_empty() {
        hover.push(format!("checks: {}", tally.words()));
    }
    hover.push(time.clone());
    if delivered {
        hover.push(format!("{} bytes", response.body.len()));
        hover.push(content_type.to_owned());
    }
    let gap = 10.0;
    let width = |ui: &Ui, part: &StatusPart| match part {
        StatusPart::Pill(text, _) => {
            widgets::text_width(ui, RichText::new(text).font(crate::theme::semibold(13.0)))
                + 2.0 * f32::from(widgets::PILL_PADDING)
        }
        StatusPart::Text(text) => widgets::text_width(ui, text.clone()),
        StatusPart::Checks(tally) => {
            widgets::text_width(ui, RichText::new(checks_text(*tally)))
                + 2.0 * ui.spacing().button_padding.x
        }
    };
    let mut open_checks = false;
    // Parts come in order of importance: each shows if it fits what the row has left, and
    // the first that does not ends the line. The status always shows, cut if need be.
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 && width(ui, &part) > ui.available_width() - reserve {
            break;
        }
        match part {
            StatusPart::Pill(text, tone) => {
                let room = ui.available_width() - reserve;
                widgets::pill(ui, &text, tone, room).on_hover_text(hover.join(" · "));
            }
            StatusPart::Text(text) => {
                let label = ui.add(egui::Label::new(text).truncate());
                if i == 0 {
                    label.on_hover_text(hover.join(" · "));
                }
            }
            StatusPart::Checks(tally) => {
                let mut job = egui::text::LayoutJob::default();
                for (n, (mark, tone)) in tally.marks().into_iter().enumerate() {
                    job.append(
                        &mark,
                        if n == 0 { 0.0 } else { 6.0 },
                        egui::TextFormat::simple(
                            egui::TextStyle::Body.resolve(ui.style()),
                            palette().tone(tone),
                        ),
                    );
                }
                let button = ui.add(egui::Button::new(job).frame_when_inactive(false));
                name(&button, &format!("Checks: {}", tally.words()));
                open_checks |= button.on_hover_text("Show the check results").clicked();
            }
        }
        ui.add_space(gap - ui.spacing().item_spacing.x);
    }
    open_checks
}

/// The checks' marks as one string, for measuring: "✓3 ✗1".
fn checks_text(tally: crate::checks::Tally) -> String {
    tally
        .marks()
        .into_iter()
        .map(|(mark, _)| mark)
        .collect::<Vec<_>>()
        .join("  ")
}

/// Notice, body, headers and check results of `pane`'s response, with its status line when
/// `status`; `id` names its viewers. `live` marks the open request's own pane: JSON rows
/// offer "Add check" and failures offer Retry. What the user chose is returned.
pub fn response_body(
    ui: &mut Ui,
    response_state: &mut crate::app::ResponsePane,
    id: &str,
    status: bool,
    live: bool,
) -> Option<PaneAction> {
    if status
        && let Some(response) = &response_state.response
        && ui.horizontal(|ui| status_line(ui, response, 0.0)).inner
    {
        response_state.tab = ResponseTab::Tests;
    }
    let mut save_body = false;
    // The preview holds part of the body: the whole of it can still be saved or copied.
    let partial = response_state
        .preview
        .as_ref()
        .is_some_and(|(preview, _)| preview.limited)
        && response_state
            .response
            .as_ref()
            .is_some_and(|r| r.outcome == imposter_app::core::execution::Outcome::Response);
    if let Some((notice, tone)) = &response_state.notice {
        // One line, the actions as icons at its end: wrapped words and buttons took three
        // rows from the body at the smallest size.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if partial && let Some(response) = &response_state.response {
                    let binary = response_state
                        .preview
                        .as_ref()
                        .is_some_and(|(preview, _)| preview.binary);
                    // Bytes that are not text would be damaged as text.
                    if !binary && icons::button(ui, Icon::Copy, "Copy full body", false).clicked() {
                        ui.ctx()
                            .copy_text(String::from_utf8_lossy(&response.body).into_owned());
                    }
                    save_body = icons::button(ui, Icon::Save, "Save body…", false).clicked();
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(tone_text(notice.as_str(), *tone)).truncate());
                });
            });
        });
    }
    if save_body {
        return Some(PaneAction::SaveBody);
    }
    let filter_status = filter_status(&response_state.filter);
    let crate::app::ResponsePane {
        response,
        preview,
        body_folds,
        filter,
        body_view,
        headers_view,
        headers_raw,
        tab,
        sent,
        sent_view,
        ..
    } = response_state;
    let Some((preview, headers)) = preview.as_ref() else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Your response will appear here").strong());
            ui.label(tone_text(
                "Send a request to inspect its status, headers and body.",
                Tone::Muted,
            ));
        });
        return None;
    };
    if let Some(failure) = response
        .as_ref()
        .filter(|r| r.outcome == imposter_app::core::execution::Outcome::TransportError)
    {
        return failure_card(ui, failure, id, live).then_some(PaneAction::Retry);
    }
    let filterable = response.as_ref().is_some_and(viewer::is_json);
    let filtered = (filterable && filter.open && !filter.query.trim().is_empty())
        .then_some(filter.result.as_ref())
        .flatten()
        .and_then(|(_, result)| result.as_ref().ok());
    let (body, outline, folds) = match filtered {
        Some(found) => (
            found.text.as_str(),
            found.outline.as_ref(),
            &mut filter.folds,
        ),
        None => (preview.text.as_str(), preview.outline.as_ref(), body_folds),
    };
    let copy_body = if filtered.is_some() {
        "Copy filtered result"
    } else if preview.limited {
        "Copy displayed body"
    } else {
        "Copy body"
    };
    let copy_headers = if preview.limited {
        "Copy displayed headers"
    } else {
        "Copy headers"
    };
    let mut toggle_filter = false;
    // Every body can be saved as received; a cut one offers it beside its notice instead.
    let body_saved_here = !partial
        && response
            .as_ref()
            .is_some_and(|r| r.outcome == imposter_app::core::execution::Outcome::Response);
    let mut save_from_tools = false;
    let mut tools = |ui: &mut Ui, shown: ResponseTab| match shown {
        ResponseTab::Body => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if body_saved_here {
                    save_from_tools = icons::button(ui, Icon::Save, "Save body…", false).clicked();
                }
                if !preview.binary {
                    widgets::viewer_tools_with(
                        ui,
                        body_view,
                        body,
                        copy_body,
                        outline.map(|outline| widgets::Folding {
                            outline,
                            folded: &mut *folds,
                            add_check: false,
                        }),
                        filterable.then_some((filter.open, &mut toggle_filter)),
                    );
                }
            });
        }
        // Raw sits at the right end, framed like a button: drawn after the tabs in their
        // style, it read as a fourth tab.
        ResponseTab::Headers => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RAW)
                            .selected(*headers_raw)
                            .min_size(vec2(0.0, icons::size())),
                    )
                    .on_hover_text("Show the headers as text, with Find and Wrap")
                    .clicked()
                {
                    *headers_raw = !*headers_raw;
                }
                if *headers_raw {
                    widgets::viewer_tools(ui, headers_view, headers, copy_headers);
                } else if icons::button(ui, Icon::Copy, copy_headers, false).clicked() {
                    ui.ctx().copy_text(headers.clone());
                }
            });
        }
        ResponseTab::Tests => {}
        ResponseTab::Request => {
            if let Some(text) = sent.as_ref() {
                widgets::viewer_tools(ui, sent_view, text, "Copy request");
            }
        }
    };
    let results = response.as_ref().map_or(&[][..], |r| r.checks.as_slice());
    let captured = response.as_ref().map_or(&[][..], |r| r.captures.as_slice());
    let script = response.as_ref().and_then(|r| r.script.as_ref());
    let console = response.as_ref().map_or(&[][..], |r| r.console.as_slice());
    let tally = response
        .as_ref()
        .map(crate::checks::Tally::of_response)
        .unwrap_or_default();
    let headers_count = response.as_ref().map_or(0, |r| r.headers.len());
    let mut tabs = vec![
        widgets::TabSpec::new(ResponseTab::Body, "Body"),
        widgets::TabSpec::new(ResponseTab::Headers, "Headers")
            .badge(count_badge(headers_count, Tone::Muted)),
    ];
    if !results.is_empty() || !captured.is_empty() || script.is_some() || !console.is_empty() {
        // Failures, when there are any: a red total read as that many failures.
        let failed = tally.failed;
        let badge = if failed > 0 {
            Some(widgets::TabBadge {
                mark: widgets::Mark::Failed(failed),
                tone: Tone::Error,
                description: format!("{failed} failed"),
            })
        } else {
            let tests = script.map_or(0, |s| s.tests.len());
            count_badge(results.len() + captured.len() + tests, Tone::Muted)
        };
        // The request's Tests tab has the same text.
        tabs.push(
            widgets::TabSpec::new(ResponseTab::Tests, "Tests")
                .named("Check results")
                .badge(badge),
        );
    }
    if sent.is_some() {
        // Where generated values ({{$uuid}}, {{$timestamp}}) can be read back.
        tabs.push(widgets::TabSpec::new(ResponseTab::Request, "Request").named("Request as sent"));
    } else if *tab == ResponseTab::Request {
        *tab = ResponseTab::Body;
    }
    ui.horizontal(|ui| {
        // At least the tools' menu button stays on the row, and Raw beside the headers'.
        let gap = ui.spacing().item_spacing.x;
        let mut reserve = icons::size() + gap;
        if *tab == ResponseTab::Body && body_saved_here {
            reserve += icons::size() + gap;
        }
        if *tab == ResponseTab::Headers {
            reserve += widgets::button_width(ui, RichText::new(RAW)) + gap;
        }
        widgets::tab_strip(ui, &tabs, tab, reserve, "More response tabs");
        tools(ui, *tab);
    });
    let mut close_filter = false;
    if *tab == ResponseTab::Body && filterable && filter.open {
        ui.horizontal_wrapped(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut filter.query)
                    .hint_text("$.items[*].id")
                    .desired_width(260.0_f32.min(ui.available_width() - 60.0)),
            );
            name(&field, "Filter the response with JSONPath");
            if filter.focus {
                field.request_focus();
                filter.focus = false;
            }
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close_filter = true;
            }
            if icons::button(ui, Icon::Close, "Close", false).clicked() {
                close_filter = true;
            }
            if let Some((text, tone)) = &filter_status {
                ui.label(tone_text(text.as_str(), *tone));
            }
        });
    }
    if save_from_tools {
        return Some(PaneAction::SaveBody);
    }
    let action = match *tab {
        ResponseTab::Body if preview.binary => {
            binary_body(ui, response.as_ref());
            None
        }
        ResponseTab::Body => widgets::code_viewer(
            ui,
            &format!("{id} body"),
            body_view,
            body,
            if filtered.is_some() {
                Language::Json
            } else {
                preview.language
            },
            outline.map(|outline| widgets::Folding {
                outline,
                folded: folds,
                add_check: live,
            }),
        ),
        ResponseTab::Headers if !*headers_raw => {
            let list = response.as_ref().map_or(&[][..], |r| r.headers.as_slice());
            widgets::headers_table(ui, id, list);
            None
        }
        ResponseTab::Headers => widgets::code_viewer(
            ui,
            &format!("{id} headers"),
            headers_view,
            headers,
            Language::Plain,
            None,
        ),
        ResponseTab::Tests => {
            crate::checks::results(
                ui,
                &crate::checks::Results {
                    checks: results,
                    captured,
                    script,
                    console,
                },
            );
            None
        }
        ResponseTab::Request => sent.as_ref().and_then(|text| {
            widgets::code_viewer(
                ui,
                &format!("{id} request"),
                sent_view,
                text,
                Language::Plain,
                None,
            )
        }),
    };
    if close_filter {
        filter.close();
    } else if toggle_filter {
        if filter.open {
            filter.close();
        } else {
            filter.open();
        }
    }
    action.map(PaneAction::Row)
}

/// A body that is not text: what it is and how big, instead of its bytes as characters.
fn binary_body(ui: &mut Ui, response: Option<&imposter_app::core::execution::Response>) {
    let Some(response) = response else {
        return;
    };
    let kind = viewer::content_type(&response.headers);
    let kind = if kind.is_empty() {
        "no content type"
    } else {
        kind
    };
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Binary body").strong());
        ui.label(tone_text(
            format!("{} · {kind}", viewer::format_size(response.body.len())),
            Tone::Muted,
        ));
        ui.label(tone_text(
            "Save body… keeps it as received, to open in another app.",
            Tone::Muted,
        ));
    });
}

/// What the filter bar says about the current query.
fn filter_status(filter: &crate::app::ResponseFilter) -> Option<(String, Tone)> {
    if filter.query.trim().is_empty() {
        return None;
    }
    if filter.stale() {
        return Some(("Filtering…".to_owned(), Tone::Muted));
    }
    Some(match &filter.result.as_ref()?.1 {
        Ok(found) => {
            let count = match found.count {
                0 => "No matches".to_owned(),
                1 => "1 match".to_owned(),
                n => format!("{n} matches"),
            };
            if found.limited {
                (
                    format!("{count} · preview limited to 256 KiB"),
                    Tone::Warning,
                )
            } else {
                (count, Tone::Muted)
            }
        }
        Err(error) => (error.clone(), Tone::Error),
    })
}

fn run_label(run: &imposter_app::store::executions::ExecutionSummary) -> String {
    let when = viewer::local_time(run.timestamp);
    match run.outcome {
        imposter_app::core::execution::Outcome::Response => format!("{when} · {}", run.status),
        imposter_app::core::execution::Outcome::TransportError => format!("{when} · failed"),
        imposter_app::core::execution::Outcome::Cancelled => format!("{when} · cancelled"),
    }
}

fn wordmark(compact: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in [
        ("I", palette().text),
        ("’m", palette().accent),
        (if compact { "p" } else { "poster" }, palette().text),
    ] {
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(20.0),
                color,
                ..Default::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod branding_tests {
    use super::*;

    #[test]
    fn only_apostrophe_m_uses_the_accent() {
        for (compact, expected) in [(false, "I’mposter"), (true, "I’mp")] {
            let job = wordmark(compact);
            assert_eq!(job.text, expected);
            assert_eq!(job.sections.len(), 3);
            assert_eq!(
                &job.text[job.sections[1].byte_range.start.0..job.sections[1].byte_range.end.0],
                "’m"
            );
            assert_eq!(job.sections[1].format.color, palette().accent);
            assert_eq!(job.sections[0].format.color, palette().text);
            assert_eq!(job.sections[2].format.color, palette().text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Drop, drop_zone};

    #[test]
    fn drop_zones_split_rows_by_height() {
        let zones = |folder, open, filtering| -> Vec<Option<Drop>> {
            [0.1, 0.4, 0.6, 0.9]
                .into_iter()
                .map(|fraction| drop_zone(fraction, folder, open, filtering))
                .collect()
        };
        let into = Some(Drop::Into { first: false });
        assert_eq!(
            zones(false, false, false),
            [
                Some(Drop::Before),
                Some(Drop::Before),
                Some(Drop::After),
                Some(Drop::After)
            ]
        );
        assert_eq!(
            zones(true, false, false),
            [Some(Drop::Before), into, into, Some(Drop::After)]
        );
        assert_eq!(
            zones(true, true, false),
            [
                Some(Drop::Before),
                into,
                into,
                Some(Drop::Into { first: true })
            ],
            "under an open folder's header is its first child"
        );
        assert_eq!(zones(true, true, true), [into; 4], "filtered: into only");
        assert_eq!(zones(false, false, true), [None; 4]);
    }
}
