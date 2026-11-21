//! Workspace chrome and the Requests screen. Drawing only: it reads state, edits the editor's
//! form fields in place and records what the user asked for as actions.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use imposter_app::core::ids::FolderId;
use imposter_app::core::model::BodyType;

use crate::app::{Action, EditorTab, ImposterApp, ResponseTab, Screen, State, TokenAction};
use crate::dialogs::{Dialog, NamePurpose};
use crate::editor::BODY_CHOICES;
use crate::history_screen::HistoryViewer;
use crate::icons::{self, Icon};
use crate::layout::{Orientation, RAIL_WIDTH, SIDEBAR_WIDTH};
use crate::theme::{Tone, method_color, palette};
use crate::tree::{Node, NodeId, Place};
use crate::viewer::{self, Language};
use crate::widgets::{self, FileRequest, VarUi, Variant, ViewerTool, ViewerTools, name, tone_text};

const SEPARATOR: f32 = 12.0;
const ENVIRONMENT_WIDTH: f32 = 200.0;
const SCREEN_MARGIN: i8 = 16;

pub fn draw(app: &mut ImposterApp, ui: &mut Ui) {
    let save = app.save_label();
    let save_text = app.save_button_text();
    let executing = app.is_executing();
    let state = &mut app.state;
    crate::shortcuts::handle(state, ui.ctx());

    egui::Panel::top("screens")
        .frame(
            egui::Frame::new()
                .fill(palette().surface)
                .inner_margin(egui::Margin::symmetric(16, 10)),
        )
        .show(ui, |ui| header(state, ui));
    egui::Panel::bottom("status")
        .frame(
            egui::Frame::new()
                .fill(palette().surface)
                .inner_margin(egui::Margin::symmetric(16, 5)),
        )
        .show(ui, |ui| {
            let available = ui.available_rect_before_wrap();
            let row = Rect::from_min_size(
                available.min,
                vec2(available.width(), crate::theme::spacing().control),
            );
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::right_to_left(Align::Center))
                    .accessibility_role(egui::Role::Status)
                    .accessibility_label("Status bar"),
                |ui| {
                    ui.set_min_size(row.size());
                    widgets::region_bounds(ui, row);
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
                },
            );
        });
    navigation_rail(state, ui);
    let screen_draw: Option<fn(&mut State, &mut Ui)> = match state.screen {
        Screen::Requests => None,
        Screen::Variables | Screen::Settings => {
            let draw = if state.screen == Screen::Variables {
                crate::variables_screen::draw
            } else {
                crate::settings_screen::draw
            };
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette().background))
                .show(ui, |ui| draw(state, ui));
            return;
        }
        Screen::History => Some(crate::history_screen::draw),
        Screen::Mocks => Some(crate::mock_screen::draw),
        Screen::Load => Some(crate::load_screen::draw),
        Screen::Run => Some(crate::run_screen::draw),
    };
    if let Some(draw) = screen_draw {
        egui::CentralPanel::default()
            .frame(screen_frame())
            .show(ui, |ui| draw(state, ui));
        return;
    }
    if state.layout.sidebar_visible {
        const SIDEBAR: &str = "sidebar";
        let max_width = if state.layout.orientation == Orientation::SideBySide {
            // Leave room for both pane minimums before allocating the sidebar.
            ui.available_width()
                - 2.0 * (crate::layout::MIN_PANE_WIDE + f32::from(SCREEN_MARGIN))
                - SEPARATOR
        } else {
            SIDEBAR_WIDTH.1
        }
        .clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1);
        let panel = egui::Panel::left(SIDEBAR)
            .frame(sidebar_frame())
            .resizable(true)
            .default_size(state.layout.sidebar_width)
            .size_range(SIDEBAR_WIDTH.0..=max_width)
            .show(ui, |ui| sidebar(state, ui));
        let edge = panel.response.rect;
        let preferred = state.layout.sidebar_width;
        if preferred > max_width && edge.width().round() >= max_width.round() {
            // Keep the preferred width in the panel's cache too, so a window resize
            // only constrains the drawn width and a wider window restores the preference.
            let layout = egui::panel::PanelState {
                outer_rect: Rect::from_min_size(edge.min, vec2(preferred, edge.height())),
            };
            ui.ctx()
                .data_mut(|data| data.insert_persisted(ui.make_persistent_id(SIDEBAR), layout));
        } else {
            state.layout.sidebar_width = edge.width().clamp(SIDEBAR_WIDTH.0, max_width).round();
        }
        // A grip on the edge says the sidebar can be dragged wider, as the splits' do.
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
        .frame(screen_frame())
        .show(ui, |ui| {
            context_bar(state, ui, save, save_text);
            if state.editor.is_none() {
                let area = ui.available_rect_before_wrap();
                pane(ui, area, "Request pane", |ui| empty_request_pane(state, ui));
                ui.allocate_rect(area, Sense::hover());
                return;
            }
            disk_banner(state, ui);
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

pub(crate) fn screen_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(palette().background)
        .inner_margin(SCREEN_MARGIN)
}

pub(crate) fn sidebar_frame() -> egui::Frame {
    egui::Frame::new().fill(palette().surface).inner_margin(12)
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
            let picker = widgets::combo("collection")
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
                if ui.button("Run…").clicked() {
                    state.actions.push(Action::RunFolder(None));
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
    let query = imposter_app::filter::Query::parse(&state.sidebar_filter);
    egui::ScrollArea::vertical()
        .id_salt("tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::scroll_view_bounds(ui);
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
                name_indent: 0.0,
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
        if ui.button("Run…").clicked() {
            actions.push(Action::RunFolder(Some(folder.id)));
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
    name_indent: f32,
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
    let spacing = crate::theme::spacing();
    let field = ui.add_sized(
        [ui.available_width(), spacing.control],
        egui::TextEdit::singleline(&mut state.sidebar_filter)
            .hint_text("Filter")
            .margin(egui::epaint::MarginF32 {
                left: spacing.xs,
                right: spacing.control + 2.0,
                top: 2.0,
                bottom: 2.0,
            })
            .desired_width(f32::INFINITY),
    );
    name(&field, "Filter requests");
    let field = widgets::describe(
        field,
        "Words match the folder path, request name and URL. Use method:POST, url:…, name:…, \
         AND, OR, NOT, -word, \"phrases\" or parentheses.",
    );
    if find {
        field.request_focus();
    }
    // Esc also ends the field's focus, possibly in this same frame.
    if (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.sidebar_filter.clear();
    }
    if !state.sidebar_filter.is_empty() {
        // The icon stays inside the field's reserved padding even when it first appears.
        let side = icons::size();
        let spot = Rect::from_center_size(
            egui::pos2(field.rect.right() - side / 2.0, field.rect.center().y),
            egui::Vec2::splat(side),
        );
        let mut overlay = ui.new_child(UiBuilder::new().max_rect(spot).id_salt("clear filter"));
        let clear = icons::button(&mut overlay, Icon::Close, "Clear the filter", false);
        if clear.clicked() {
            state.sidebar_filter.clear();
        }
    }
    let query = imposter_app::filter::Query::parse(&state.sidebar_filter);
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

/// A one-line notice over a pane, with its choices as buttons at its end (the first
/// rightmost). Returns the index of the button clicked, in `buttons`' order.
pub(crate) fn notice_banner(ui: &mut Ui, text: &str, buttons: &[(&str, &str)]) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(palette().hover)
        .stroke(egui::Stroke::new(1.0, palette().field_border))
        .corner_radius(BAR_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (index, (label, accessible)) in buttons.iter().enumerate().rev() {
                        let button = ui.button(*label);
                        name(&button, accessible);
                        if button.clicked() {
                            clicked = Some(index);
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(text).truncate())
                            .on_hover_text(text);
                    });
                });
            });
        });
    ui.add_space(8.0);
    clicked
}

/// Over the open request when its file changed or went away on disk while it had unsaved
/// edits: the user chooses, and its edits wait meanwhile.
fn disk_banner(state: &mut State, ui: &mut Ui) {
    use crate::app::DiskChoice;
    let Some(change) = state.disk_change.clone() else {
        return;
    };
    if state.editor.as_ref().and_then(|p| p.editor.id()) != Some(change.id) {
        return;
    }
    let (text, buttons, choices): (String, [(&str, &str); 2], [DiskChoice; 2]) = if change.deleted {
        (
            format!(
                "{} was deleted on disk. Your unsaved edits are here.",
                change.name
            ),
            [
                ("Close", "Close the deleted request"),
                ("Keep as draft", "Keep the edits as a draft"),
            ],
            [DiskChoice::Close, DiskChoice::KeepAsDraft],
        )
    } else {
        (
            format!("{} changed on disk. Your unsaved edits wait.", change.name),
            [
                (
                    "Keep mine",
                    "Keep my edits; the next save writes over the file",
                ),
                (
                    "Take theirs",
                    "Take the file's version; keep my edits beside it",
                ),
            ],
            [DiskChoice::KeepMine, DiskChoice::TakeTheirs],
        )
    };
    if let Some(index) = notice_banner(ui, &text, &buttons) {
        state.actions.push(Action::Disk(choices[index]));
    }
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

const TREE_METHOD_CHARS: usize = 7;

fn tree_caption(
    ui: &Ui,
    badge: RichText,
    name: &str,
    name_indent: f32,
    folder: bool,
) -> egui::text::LayoutJob {
    let spacing = crate::theme::spacing();
    let gutter = widgets::text_width(
        ui,
        crate::theme::meta("M".repeat(TREE_METHOD_CHARS)).monospace(),
    );
    let badge_width = widgets::text_width(ui, badge.clone());
    // Deep trees still leave room for part of the name in the narrow sidebar.
    let indent = name_indent.min(ui.available_width() * 0.25);
    let mut gap = gutter - badge_width + spacing.xs + indent;
    let mut text = egui::text::LayoutJob::default();
    badge.append_to(
        &mut text,
        ui.style(),
        egui::FontSelection::Default,
        Align::Center,
    );
    let font = if folder {
        crate::theme::semibold(crate::theme::BODY)
    } else {
        egui::FontId::proportional(crate::theme::BODY)
    };
    let mut part = |piece: &str, color| {
        if !piece.is_empty() {
            let mut format = egui::TextFormat::simple(font.clone(), color);
            format.valign = Align::Center;
            text.append(piece, std::mem::take(&mut gap), format);
        }
    };
    let mut at = 0;
    for token in crate::vars::tokens(name) {
        part(&name[at..token.range.start], palette().text);
        part(&name[token.range.clone()], palette().accent);
        at = token.range.end;
    }
    part(&name[at..], palette().text);
    text
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
                    name_indent: view.name_indent + crate::theme::spacing().md,
                    ..view
                };
                let header = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().indent = icons::size();
                    ui.spacing_mut().icon_width = crate::theme::spacing().row;
                    let toggle = collapsing.show_toggle_button(ui, widgets::collapsing_icon);
                    toggle.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            ui.is_enabled(),
                            format!("Toggle folder {}", folder.name),
                        )
                    });
                    ui.ctx().accesskit_node_builder(toggle.id, |node| {
                        node.set_expanded(collapsing.is_open());
                    });
                    let icon = if collapsing.is_open() {
                        Icon::FolderOpen
                    } else {
                        Icon::Folder
                    };
                    let text = tree_caption(
                        ui,
                        icons::glyph_text(icon, 16.0).color(palette().text_muted),
                        &folder.name,
                        view.name_indent,
                        true,
                    );
                    let response = widgets::tree_option(ui, selected == Some(id), text);
                    name(&response, &format!("Folder {}", folder.name));
                    if !folder.scripts.is_empty() {
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_description("Has scripts");
                        });
                    }
                    response.on_hover_text(&folder.name)
                });
                collapsing.show_body_unindented(ui, |ui| {
                    nodes(ui, children, Some(folder.id), inner, actions);
                });
                header.inner
            }
            Node::Request(request) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(icons::size());
                    let mut method = request.method.chars();
                    let mut badge: String = method.by_ref().take(TREE_METHOD_CHARS).collect();
                    let elided = method.next().is_some();
                    if elided {
                        badge.pop();
                        badge.push('…');
                    }
                    let text = tree_caption(
                        ui,
                        crate::theme::meta(badge)
                            .monospace()
                            .color(method_color(&request.method)),
                        &request.name,
                        view.name_indent,
                        false,
                    );
                    let response = widgets::tree_option(ui, selected == Some(id), text);
                    let label = format!("{} {}", request.method, request.name);
                    name(&response, &label);
                    response.on_hover_text(if elided { &label } else { &request.name })
                })
                .inner
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
            format(
                egui::FontId::proportional(crate::theme::PAGE_TITLE),
                palette().text_muted,
            ),
        );
    }
    job.append(
        name,
        0.0,
        format(
            crate::theme::semibold(crate::theme::PAGE_TITLE),
            palette().text,
        ),
    );
    job
}

fn context_bar(
    state: &mut State,
    ui: &mut Ui,
    save: Option<widgets::SaveStatus<'static>>,
    save_text: &str,
) {
    let path = match &state.editor {
        None => "Requests".to_owned(),
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
    let has_editor = state.editor.is_some();
    let left = |ui: &mut Ui| {
        ui.add(egui::Label::new(breadcrumb(&path)).truncate())
            .on_hover_text(&path);
    };
    let right = |ui: &mut Ui| {
        if layout_switch(ui, state.layout.orientation) {
            state.actions.push(Action::ToggleLayout);
        }
        divider(ui, crate::theme::spacing().control);
        let toggle = if state.layout.sidebar_visible {
            "Hide Collections"
        } else {
            "Show Collections"
        };
        if icons::button(ui, Icon::Sidebar, toggle, false).clicked() {
            state.actions.push(Action::ToggleSidebar);
        }
        // The toggle and layout choices have distinct selected-state semantics.
        if save.is_some() {
            divider(ui, crate::theme::spacing().control);
        }
        if let Some(save) = save {
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
    };
    if has_editor {
        widgets::toolbar_row(ui, left, right);
    } else {
        widgets::screen_toolbar(ui, &path, |_| {}, right);
    }
}

fn header(state: &mut State, ui: &mut Ui) {
    let show_wordmark = ui.available_width() >= 900.0;
    ui.horizontal(|ui| {
        // The same drawing as the window and Dock icon.
        let logo = state.logo.get_or_insert_with(|| {
            let image = egui::ColorImage::from_rgba_unmultiplied([96, 96], &icons::app_icon(96));
            ui.ctx()
                .load_texture("app icon", image, egui::TextureOptions::LINEAR)
        });
        let mark = ui.add(egui::Image::new(&*logo).fit_to_exact_size(vec2(34.0, 34.0)));
        mark.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "I’mposter"));
        if show_wordmark {
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(wordmark()));
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, palette().text);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            environment_picker(state, ui);
        });
    });
}

fn navigation_rail(state: &mut State, ui: &mut Ui) {
    let spacing = crate::theme::spacing();
    let size = spacing.control + spacing.sm;
    egui::Panel::left("navigation rail")
        .resizable(false)
        .exact_size(RAIL_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(palette().surface)
                .inner_margin(egui::epaint::MarginF32::symmetric(0.0, spacing.sm)),
        )
        .show(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.y = spacing.xs;
                for screen in [Screen::Requests, Screen::Variables, Screen::History] {
                    rail_item(state, ui, screen, size);
                }
                ui.add_space(spacing.xs);
                for screen in [Screen::Mocks, Screen::Load, Screen::Run] {
                    rail_item(state, ui, screen, size);
                }
                ui.add_space((ui.available_height() - size).max(0.0));
                rail_item(state, ui, Screen::Settings, size);
            });
        });
}

fn activity(stopping: bool) -> (Tone, &'static str) {
    if stopping {
        (Tone::Muted, "Stopping…")
    } else {
        (Tone::Ok, "Running")
    }
}

fn rail_item(state: &mut State, ui: &mut Ui, screen: Screen, size: f32) {
    use crate::shortcuts::{self, Command};

    let (icon, label) = match screen {
        Screen::Requests => (Icon::Requests, "Requests"),
        Screen::Variables => (Icon::Variables, "Variables"),
        Screen::History => (Icon::History, "History"),
        Screen::Mocks => (Icon::Mocks, "Mocks"),
        Screen::Load => (Icon::Load, "Load"),
        Screen::Run => (Icon::Run, "Run"),
        Screen::Settings => (Icon::Settings, "Settings"),
    };
    let dirty = match screen {
        Screen::Variables => state.variables.dirty(),
        Screen::Mocks => state.mocks.dirty(),
        Screen::Settings => state.settings_screen.dirty(),
        _ => false,
    };
    let running = match screen {
        Screen::Mocks => state
            .mocks
            .run
            .as_ref()
            .filter(|run| run.stopping || run.address.is_some())
            .map(|run| activity(run.stopping)),
        Screen::Load => state.load.run.as_ref().map(|run| activity(run.stopping)),
        Screen::Run => state.run.run.as_ref().map(|run| activity(run.stopping)),
        _ => None,
    };

    let button = widgets::rail_button(ui, icon, label, size, state.screen == screen);
    if dirty {
        widgets::corner_dot(ui, button.rect, Align::Min, Tone::Warning);
    }
    if let Some((tone, _)) = running {
        widgets::corner_dot(ui, button.rect, Align::Max, tone);
    }
    let mut hint = if screen == Screen::Settings {
        shortcuts::ALL
            .iter()
            .find(|entry| entry.command == Command::Settings)
            .and_then(|entry| entry.keys.first())
            .map_or_else(
                || label.to_owned(),
                |key| shortcuts::hint(ui.ctx(), label, key),
            )
    } else {
        label.to_owned()
    };
    let description = [
        running.map(|(_, text)| text),
        dirty.then_some("Unsaved changes"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if !description.is_empty() {
        hint.push('\n');
        hint.push_str(&description);
        button
            .ctx
            .accesskit_node_builder(button.id, |node| node.set_description(description));
    }
    if button.on_hover_text(hint).clicked() {
        state.actions.push(Action::Screen(screen));
    }
}

/// Running tools stay reachable from the status bar, including during shutdown.
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
    if let Some(run) = &state.run.run {
        let (label, hint) = if run.stopping {
            ("Run stopping…", "Run stopping")
        } else {
            ("Run running", "Run in progress")
        };
        if activity_button(ui, label, hint).clicked() {
            state.actions.push(Action::Screen(Screen::Run));
        }
    }
    if state.screen != Screen::Mocks
        && let Some(run) = &state.mocks.run
    {
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
    let button = widgets::button(ui, Variant::Ghost, None, &format!("● {label}"));
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
            widgets::combo("environment")
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

/// Two buttons, one per layout; the current one is selected. One button showing the other
/// layout's icon read as the current state.
fn layout_switch(ui: &mut Ui, current: Orientation) -> bool {
    let mut switched = false;
    for (orientation, icon, label) in [
        (
            Orientation::Stacked,
            Icon::Rows,
            "Stack request and response",
        ),
        (
            Orientation::SideBySide,
            Icon::Columns,
            "Put request and response side by side",
        ),
    ] {
        let selected = current == orientation;
        switched |= icons::button(ui, icon, label, selected).clicked() && !selected;
    }
    switched
}

fn send_slot_width(ui: &Ui) -> f32 {
    // Reserve enough space for the usual environment names and keep most of the bar for the URL.
    let limit = (ui.available_width() * 0.3).clamp(96.0, 168.0);
    ["Send to Production", "Cancel", "Stop"]
        .into_iter()
        .map(|label| widgets::action_width(ui, label))
        .fold(96.0, f32::max)
        .min(limit)
}

fn command_bar(state: &mut State, ui: &mut Ui, executing: bool) {
    let streaming = executing && !state.response.live_events.is_empty();
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
                let label = match (&target, executing) {
                    // A stream is listened to until stopped; nothing is called off.
                    (_, true) if streaming => "Stop".to_owned(),
                    (_, true) => "Cancel".to_owned(),
                    (Some(name), false) => format!("Send to {name}"),
                    (None, false) => "Send".to_owned(),
                };
                let send_width = send_slot_width(ui);
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
                let variant = if executing {
                    Variant::Danger
                } else {
                    Variant::Primary
                };
                let send = widgets::request_bar_button(
                    ui,
                    variant,
                    &label,
                    vec2(send_width, height - 2.0),
                );
                let action = match (&environment, &target, executing) {
                    (Some(name), None, false) => format!("Send with {name}"),
                    _ => label,
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
    pane(ui, first, "Request pane", |ui| request_pane(state, ui));
    pane(ui, second, "Response pane", |ui| response_pane(state, ui));
    ui.allocate_rect(area, Sense::hover());
}

fn pane(ui: &mut Ui, area: Rect, label: &str, content: impl FnOnce(&mut Ui)) {
    let mut pane = ui.new_child(
        UiBuilder::new()
            .max_rect(area)
            .id_salt(label)
            .accessibility_role(egui::Role::Group)
            .accessibility_label(label),
    );
    widgets::region_bounds(&pane, area);
    pane.set_clip_rect(pane.clip_rect().intersect(area));
    // Give the scroll areas their own padded space while the group describes the entire pane.
    let mut content_ui =
        pane.new_child(UiBuilder::new().max_rect(area.shrink(crate::theme::spacing().sm)));
    content(&mut content_ui);
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
        crate::editor::Badge::Count(count) if tab == EditorTab::Tests => Some(widgets::TabBadge {
            mark: widgets::Mark::Count(count),
            tone: Tone::Muted,
            description: if count == 1 {
                "1 check".to_owned()
            } else {
                format!("{count} checks")
            },
        }),
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
    if let Some(action) = centered_message(ui, title, text, offers) {
        state.actions.push(action);
    }
}

/// Measure the text and actions together so the whole message is centred even when it wraps.
fn centered_message(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    offers: Vec<(&str, Action)>,
) -> Option<Action> {
    let width = ui.available_width();
    let spacing = crate::theme::spacing();
    let gap = ui.spacing().item_spacing.x;
    let rows_gap = ui.spacing().item_spacing.y;
    let galley = |text: RichText| {
        egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Wrap),
            width,
            egui::TextStyle::Body,
        )
    };
    let title = galley(crate::theme::section(title));
    let detail = galley(tone_text(detail, Tone::Muted));
    let buttons_width = offers
        .iter()
        .map(|(label, _)| widgets::action_width(ui, label) + gap)
        .sum::<f32>()
        - gap;
    let inline = buttons_width <= width;
    let button_height = if offers.is_empty() {
        0.0
    } else if inline {
        spacing.control + spacing.sm + rows_gap
    } else {
        offers
            .iter()
            .map(|_| spacing.control + rows_gap)
            .sum::<f32>()
            + spacing.sm
    };
    let height = title.size().y + detail.size().y + rows_gap + button_height;
    ui.add_space(((ui.available_height() - height) / 2.0).max(0.0));
    ui.vertical_centered(|ui| {
        ui.label(title);
        ui.label(detail);
    });
    if offers.is_empty() {
        return None;
    }
    ui.add_space(spacing.sm);
    let mut chosen = None;
    let buttons = |ui: &mut Ui| {
        for (index, (label, action)) in offers.into_iter().enumerate() {
            let variant = if index == 0 {
                Variant::Primary
            } else {
                Variant::Secondary
            };
            if widgets::button(ui, variant, None, label).clicked() {
                chosen = Some(action);
            }
        }
    };
    if inline {
        ui.horizontal(|ui| {
            ui.add_space((width - buttons_width) / 2.0);
            buttons(ui);
        });
    } else {
        ui.vertical_centered(buttons);
    }
    chosen
}

/// The one name of starting a request from a curl command.
pub const PASTE_CURL: &str = "Paste curl…";

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

/// Where the OAuth 2.0 token stands ("Token valid for 54 min"), with Get new token and,
/// when there is one, Clear token. Clearing asks nothing: the next send gets a new token.
fn token_line(
    ui: &mut Ui,
    view: &crate::app::TokenView,
    signs_in: bool,
    actions: &mut Vec<Action>,
) {
    use imposter_app::oauth::TokenState;
    ui.add_space(12.0);
    let now = imposter_app::executor::now().0;
    let (text, tone) = match &view.state {
        _ if view.signing_in.is_some() => (
            "Waiting for you to sign in in the browser…".to_owned(),
            Tone::Muted,
        ),
        None => ("Checking the token…".to_owned(), Tone::Muted),
        Some(Err(error)) => (error.clone(), Tone::Error),
        Some(Ok(TokenState::Missing)) if signs_in => (
            "Not signed in: Sign in to get a token.".to_owned(),
            Tone::Muted,
        ),
        Some(Ok(TokenState::Missing)) => ("No token yet: Send gets one.".to_owned(), Tone::Muted),
        Some(Ok(TokenState::Good { expires_at: None })) => {
            ("Token valid until cleared".to_owned(), Tone::Ok)
        }
        Some(Ok(TokenState::Good {
            expires_at: Some(at),
        })) => (
            format!("Token valid for {}", remaining(at.saturating_sub(now))),
            Tone::Ok,
        ),
        Some(Ok(TokenState::Expired { refreshable: true })) => (
            "Token expired: the next send refreshes it".to_owned(),
            Tone::Muted,
        ),
        Some(Ok(TokenState::Expired { refreshable: false })) if signs_in => {
            ("Token expired: sign in again".to_owned(), Tone::Muted)
        }
        Some(Ok(TokenState::Expired { refreshable: false })) => (
            "Token expired: the next send gets a new one".to_owned(),
            Tone::Muted,
        ),
    };
    ui.add(egui::Label::new(tone_text(text, tone)).wrap());
    let has_token = matches!(
        view.state,
        Some(Ok(TokenState::Good { .. } | TokenState::Expired { .. }))
    );
    ui.horizontal(|ui| {
        if view.signing_in.is_some() {
            if ui.button("Cancel sign-in").clicked() {
                actions.push(Action::Token(TokenAction::Cancel));
            }
            return;
        }
        let (label, hover) = if signs_in {
            (
                "Sign in",
                "Sign in in the browser and keep the token it gives",
            )
        } else {
            (
                "Get new token",
                "Ask the token endpoint for a token now and keep it",
            )
        };
        if ui
            .add_enabled(!view.busy, egui::Button::new(label))
            .on_hover_text(hover)
            .clicked()
        {
            actions.push(Action::Token(TokenAction::Get));
        }
        if has_token
            && ui
                .add_enabled(!view.busy, egui::Button::new("Clear token"))
                .on_hover_text(if signs_in {
                    "Forget this token; sending then needs a new sign-in"
                } else {
                    "Forget this token; the next send gets a new one"
                })
                .clicked()
        {
            actions.push(Action::Token(TokenAction::Clear));
        }
    });
}

/// Milliseconds left, in words: "45 s", "54 min", "2 h 5 min".
pub fn remaining(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1000;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        _ if seconds % 3600 < 60 => format!("{} h", seconds / 3600),
        _ => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
    }
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
        active_environment,
        ..
    } = state;
    let Some(pane) = editor else {
        return;
    };
    let mut vars = VarUi {
        snapshot: snapshot.as_ref(),
        inspect,
    };
    let tabs: Vec<_> = EditorTab::PRIMARY
        .into_iter()
        .chain(EditorTab::MORE)
        .map(|(tab, label)| {
            let spec = widgets::TabSpec::new(tab, label).badge(editor_badge(&pane.editor, tab));
            if tab == EditorTab::Settings {
                // Screen readers need distinct names for this tab and the Settings destination.
                spec.named("Request settings")
            } else {
                spec
            }
        })
        .collect();
    ui.horizontal(|ui| {
        const SCRIPT_REFERENCE: &str = "Script reference";
        let reference = pane.tab == EditorTab::Scripts;
        let with_caption = ui.available_width() >= 340.0;
        let reserve = if !reference {
            0.0
        } else if with_caption {
            widgets::action_width_with_icon(ui, SCRIPT_REFERENCE, Some(Icon::Info))
        } else {
            icons::size()
        };
        widgets::tab_strip(
            ui,
            &tabs,
            &mut pane.tab,
            EditorTab::PRIMARY.len(),
            reserve
                + if reference {
                    ui.spacing().item_spacing.x
                } else {
                    0.0
                },
            "More request tabs",
        );
        if reference {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = if with_caption {
                    widgets::button(ui, Variant::Ghost, Some(Icon::Info), SCRIPT_REFERENCE)
                } else {
                    icons::button(ui, Icon::Info, SCRIPT_REFERENCE, false)
                };
                if button.clicked() {
                    actions.push(Action::Prompt(Dialog::ScriptReference));
                }
            });
        }
    });
    ui.separator();
    if pane.tab == EditorTab::Body {
        body_tab(ui, pane, &mut vars, actions);
        return;
    }
    if pane.tab == EditorTab::Scripts {
        if crate::scripts_tab::draw(ui, &mut pane.editor.scripts, &outer, actions) {
            actions.push(Action::Edited {
                url: false,
                params: false,
            });
        }
        return;
    }
    let mut edited = false;
    let mut params = false;
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
            EditorTab::Auth => {
                edited = widgets::auth_form(ui, &mut pane.editor.auth, &mut vars);
                if pane.editor.auth.kind == Some(imposter_app::core::model::AuthKind::OAuth2) {
                    let signs_in = pane.editor.auth.oauth.grant
                        == imposter_app::core::model::OAuthGrant::AuthorizationCode;
                    token_line(ui, &pane.token, signs_in, actions);
                    let wanted = (pane.editor.auth.oauth.clone(), *active_environment);
                    if !pane.token.busy && pane.token.checked.as_ref() != Some(&wanted) {
                        actions.push(Action::Token(TokenAction::Check));
                    }
                }
            }
            EditorTab::Tests => {
                edited = crate::checks::checks_table(ui, &mut pane.editor.checks, &mut vars);
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
            EditorTab::Body | EditorTab::Scripts => {}
        });
    if edited || params {
        actions.push(Action::Edited { url: false, params });
    }
}

fn body_tab(
    ui: &mut Ui,
    pane: &mut crate::app::EditorPane,
    vars: &mut VarUi,
    actions: &mut Vec<Action>,
) {
    let editor = &mut pane.editor;
    let mut edited = body_header(ui, &mut editor.body_type, &editor.body_text, actions);
    let mut pick = None;
    edited |= match editor.body_type {
        BodyType::None => {
            ui.label(tone_text("This request has no body", Tone::Muted));
            false
        }
        BodyType::Json | BodyType::Plain | BodyType::Text => {
            // Only the text scrolls, so its viewport height stays independent of the scroll offset.
            widgets::body_editor(
                ui,
                &mut editor.body_text,
                editor.body_type == BodyType::Json,
                vars,
            )
        }
        BodyType::Urlencoded | BodyType::FormData | BodyType::Multipart => {
            egui::ScrollArea::both()
                .id_salt("body fields")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if editor.body_type == BodyType::Multipart {
                        widgets::multipart_table(
                            ui,
                            &mut editor.multipart,
                            &mut pane.confirm_clear[4],
                            &mut pick,
                            vars,
                        )
                    } else {
                        widgets::rows_table(
                            ui,
                            "Fields",
                            &mut editor.form,
                            &mut pane.confirm_clear[3],
                            vars,
                        )
                    }
                })
                .inner
        }
        BodyType::Binary => {
            ui.horizontal(|ui| {
                let width = ui.available_width() - icons::size() - ui.spacing().item_spacing.x;
                let edited = widgets::var_edit(
                    ui,
                    &mut editor.binary_path,
                    "Binary file path",
                    width,
                    false,
                    vars,
                )
                .changed();
                if icons::button(ui, Icon::Folder, "Choose file…", false).clicked() {
                    pick = Some(FileRequest::Binary);
                }
                edited
            })
            .inner
        }
    };
    if let Some(target) = pick {
        actions.push(Action::PickFile(target));
    }
    if edited {
        actions.push(Action::Edited {
            url: false,
            params: false,
        });
    }
}

fn body_header(
    ui: &mut Ui,
    body_type: &mut BodyType,
    text: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let json = (*body_type == BodyType::Json && !text.trim().is_empty())
        .then(|| viewer::check_json_template(text));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::theme::spacing().xs;
        // Keep the picker steady as the JSON status appears or changes during editing.
        let reserve = widgets::text_width(ui, crate::theme::meta("Valid JSON"))
            + icons::size()
            + 2.0 * ui.spacing().item_spacing.x;
        let previous = *body_type;
        if !widgets::compact_segments(ui, "Body type", body_type, &BODY_CHOICES, reserve) {
            let selected = BODY_CHOICES
                .iter()
                .find(|(value, _, _)| value == body_type)
                .map_or("JSON", |(_, _, full)| *full);
            let width = 108.0_f32
                .max(widgets::button_width(ui, crate::theme::body(selected)) + icons::size());
            let picker = widgets::toolbar_combo(ui, "body type", width, selected, |ui| {
                for (value, _, label) in BODY_CHOICES {
                    if widgets::option(ui, *body_type == value, label).clicked() {
                        *body_type = value;
                        ui.close();
                    }
                }
            });
            name(&picker, "Body type");
        }
        if *body_type == BodyType::Json
            && let Some(result) = json
        {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = result.is_ok()
                    && viewer::format_json_body(text).is_some_and(|formatted| formatted != text);
                let hint = if enabled {
                    crate::shortcuts::hint(
                        ui.ctx(),
                        "Re-indent the body, keeping every value as typed",
                        &crate::shortcuts::FORMAT_BODY,
                    )
                } else {
                    "Already formatted, or invalid JSON".to_owned()
                };
                let format = ui
                    .add_enabled_ui(enabled, |ui| {
                        icons::button_with_hint(ui, Icon::Indent, "Format body", &hint, false)
                    })
                    .inner;
                if format.clicked() {
                    actions.push(Action::FormatBody);
                }
                let (status, hover) = match result {
                    Ok(()) => (crate::theme::meta("Valid JSON").color(palette().ok), None),
                    Err(error) => {
                        let full = error.to_string();
                        let location =
                            format!(" at line {} column {}", error.line(), error.column());
                        let reason = full.strip_suffix(&location).unwrap_or(&full);
                        let status = crate::theme::meta(format!("Line {}: {reason}", error.line()))
                            .color(palette().error);
                        (status, Some(format!("Invalid JSON: {full}")))
                    }
                };
                let label = ui.add(
                    egui::Label::new(status)
                        .truncate()
                        .show_tooltip_when_elided(hover.is_none()),
                );
                if let Some(hover) = hover {
                    label.on_hover_text(hover);
                }
            });
        }
        *body_type != previous
    })
    .inner
}

fn response_pane(state: &mut State, ui: &mut Ui) {
    let now = ui.input(|i| i.time);
    if let Some(seconds) = state.sending_for(now) {
        // A stream shows its events as they come, in place of the waiting message.
        if !state.response.live_events.is_empty() {
            let response = &mut state.response;
            ui.label(tone_text(
                format!(
                    "Receiving events… {} so far · {seconds:.1} s · Stop ends the stream",
                    response.live_events.len()
                ),
                Tone::Muted,
            ));
            crate::events::list(
                ui,
                "live",
                &response.live_events,
                &mut response.event_selected,
                true,
            );
            return;
        }
        let sending = format!("Sending… {seconds:.1} s");
        if state.response.preview.is_none() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(20.0));
                ui.label(RichText::new(sending).strong())
                    .on_hover_text("Cancel stops the send");
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
                BlockedAction::SignIn => Action::Token(TokenAction::Get),
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
            let not_set = response_state.captures_not_set.as_ref();
            let label = not_set.map(|warning| format!("{} not set", warning.names.len()));
            let warning_width = label.as_ref().map_or(0.0, |label| {
                widgets::action_width_with_icon(ui, label, Some(Icon::Warning))
                    + ui.spacing().item_spacing.x
            });
            if let Some(response) = &response_state.response
                && status_line(ui, response, runs + warning_width, None)
            {
                response_state.tab = ResponseTab::Tests;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                runs_menu(ui, response_state, actions);
                if let (Some(warning), Some(label)) = (not_set, label) {
                    let badge = widgets::button(ui, Variant::Warning, Some(Icon::Warning), &label);
                    let detail = warning.detail();
                    badge.ctx.accesskit_node_builder(badge.id, |node| {
                        node.set_description(detail.as_str());
                    });
                    if badge.on_hover_text(detail).clicked() {
                        actions.push(Action::OpenVariablesScope(warning.scope));
                    }
                }
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
    SignIn,
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
        let error = ui.add(egui::Label::new(tone_text(headline, Tone::Error).strong()).wrap());
        if !blocked.unresolved.is_empty() {
            const HINT: &str =
                "Nothing was sent. Define the variable, or fix the name in the request.";
            error
                .ctx
                .accesskit_node_builder(error.id, |node| node.set_description(HINT));
            error.on_hover_text(HINT);
        }
        if blocked.sign_in {
            ui.add(
                egui::Label::new(tone_text(
                    "Nothing was sent. Sign in in the browser, then send again.",
                    Tone::Muted,
                ))
                .wrap(),
            );
            if widgets::button(ui, Variant::Primary, None, "Sign in")
                .on_hover_text("Sign in in the browser and keep the token it gives")
                .clicked()
            {
                act(BlockedAction::SignIn);
            }
        }
        if let Some(first) = blocked.unresolved.first() {
            let place = if imposter_app::core::variables::secret_name(first).is_some() {
                format!("{scope_name} secrets")
            } else {
                scope_name.to_owned()
            };
            let caption = format!("Define in {place}");
            let full = format!("Define {{{{{first}}}}} in {place}");
            let mut buttons = |ui: &mut Ui| {
                // The headline names the variable; the button says where it goes, which a
                // long name would cut off. Hover and screen readers get both.
                let define = widgets::button(ui, Variant::Primary, None, &caption)
                    .on_hover_text(full.as_str());
                name(&define, &full);
                if define.clicked() {
                    act(BlockedAction::Define(first.clone()));
                }
                if widgets::button(ui, Variant::Secondary, None, "Open Variables").clicked() {
                    act(BlockedAction::OpenVariables);
                }
            };
            let width = widgets::action_width(ui, &caption)
                + widgets::action_width(ui, "Open Variables")
                + ui.spacing().item_spacing.x;
            if width <= ui.available_width() {
                ui.horizontal(buttons);
            } else {
                buttons(ui);
            }
        }
    });
}

/// Scrolls when the pane is too short for the error and its available actions.
fn problem_frame(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    let spacing = crate::theme::spacing();
    ui.add_space(spacing.sm);
    egui::ScrollArea::vertical()
        .id_salt("problem")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = spacing.sm;
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
            .icon(widgets::collapsing_icon)
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

pub(crate) fn history_status(
    ui: &mut Ui,
    response: &imposter_app::core::execution::Response,
    request: &str,
) -> bool {
    status_line(ui, response, 0.0, Some(request))
}

/// One line about a response: its status as a pill, then what fits of the checks, time,
/// size and content type, dropped in reverse order of importance. The status's hover text
/// has them all. `reserve` keeps room at the end of the row. Returns whether the checks were
/// clicked, which asks for their results.
fn status_line(
    ui: &mut Ui,
    response: &imposter_app::core::execution::Response,
    reserve: f32,
    request: Option<&str>,
) -> bool {
    let status = viewer::status(response);
    // In order of importance. A transport failure has no status for a pill: the card under
    // the line names it, and a pill would only repeat it.
    let failed = response.outcome == imposter_app::core::execution::Outcome::TransportError;
    let mut parts = Vec::new();
    if !failed || request.is_some() {
        let caption = if request.is_some()
            && response.outcome == imposter_app::core::execution::Outcome::Response
            && widgets::text_width(ui, crate::theme::body(&status.text)) > 142.0
        {
            response.status.to_string()
        } else {
            status.text.clone()
        };
        parts.push(StatusPart::Pill(caption, status.tone));
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
    if delivered && !kind.is_empty() && request.is_none() {
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
    if let Some(request) = request {
        let (method, url) = request.split_once(' ').unwrap_or((request, ""));
        let method = RichText::new(method)
            .monospace()
            .color(method_color(method));
        let height = crate::theme::spacing().control;
        let method_width = widgets::text_width(ui, method.clone()).min(76.0);
        ui.add_sized([method_width, height], egui::Label::new(method).truncate());
        let metadata = parts.iter().map(|part| width(ui, part) + gap).sum::<f32>();
        let available = ui.available_width();
        let url_width = (available - metadata).max(available.min(64.0));
        let url_text = RichText::new(url).monospace();
        let url_width = widgets::text_width(ui, url_text.clone()).min(url_width);
        ui.add_sized(
            [url_width, height],
            egui::Label::new(url_text)
                .truncate()
                .selectable(true)
                .halign(Align::Min),
        )
        .on_hover_text(url);
    }
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
    response_content(ui, response_state, id, status, live, None)
}

pub(crate) fn history_response(
    ui: &mut Ui,
    response_state: &mut crate::app::ResponsePane,
    history: HistoryViewer<'_>,
) -> Option<PaneAction> {
    response_content(ui, response_state, "history", false, false, Some(history))
}

fn response_content(
    ui: &mut Ui,
    response_state: &mut crate::app::ResponsePane,
    id: &str,
    status: bool,
    live: bool,
    mut history: Option<HistoryViewer<'_>>,
) -> Option<PaneAction> {
    let queued = response_state.pending_tool.take();
    if status
        && let Some(response) = &response_state.response
        && ui
            .horizontal(|ui| status_line(ui, response, 0.0, None))
            .inner
    {
        response_state.tab = ResponseTab::Tests;
    }
    // The preview holds part of the body: the whole of it can still be saved or copied.
    let partial = response_state
        .preview
        .as_ref()
        .is_some_and(|(preview, _)| preview.limited)
        && response_state
            .response
            .as_ref()
            .is_some_and(|r| r.outcome == imposter_app::core::execution::Outcome::Response);
    let binary = response_state
        .preview
        .as_ref()
        .is_some_and(|(preview, _)| preview.binary);
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
        event_selected,
        notice,
        ..
    } = response_state;
    if history.is_none()
        && (response_notice(ui, notice.as_ref(), response.as_ref(), partial, binary)
            || (partial && queued == Some(ViewerTool::Save)))
    {
        return Some(PaneAction::SaveBody);
    }
    let failure = response
        .as_ref()
        .filter(|r| r.outcome == imposter_app::core::execution::Outcome::TransportError);
    if (preview.is_none() || failure.is_some())
        && let Some(history) = &mut history
    {
        history.tabs(ui, basic_history_tabs(), tab, 0.0, |_, _| {});
        if !history.shows_response() {
            history.show_request(ui);
            return None;
        }
        if let Some(text) = &history.empty_response {
            ui.add(egui::Label::new(text.clone()).wrap().selectable(true));
            return None;
        }
    }
    let Some((preview, headers)) = preview.as_ref() else {
        centered_message(
            ui,
            "Your response will appear here",
            "Send a request to inspect its status, headers and body.",
            Vec::new(),
        );
        return None;
    };
    if let Some(failure) = failure {
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
    let raw_open = *headers_raw;
    let mut tools = |ui: &mut Ui, shown: ResponseTab| match shown {
        ResponseTab::Body => {
            let tools = ViewerTools {
                copy: (!preview.binary).then_some(copy_body),
                wrap: body_view.wrap,
                filter_open: filterable.then_some(filter.open),
                fold: outline.is_some(),
                save: body_saved_here,
            };
            match tools.show(ui).or(queued) {
                Some(ViewerTool::Save) => save_from_tools = true,
                Some(ViewerTool::Filter) => toggle_filter = true,
                Some(tool) => tool.apply(
                    ui.ctx(),
                    body_view,
                    body,
                    outline.map(|outline| widgets::Folding {
                        outline,
                        folded: &mut *folds,
                        add_check: false,
                    }),
                ),
                None => {}
            }
        }
        // Raw sits at the right end, framed like a button: drawn after the tabs in their
        // style, it read as a fourth tab.
        ResponseTab::Headers => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::toggle_button(ui, headers_raw, RAW)
                    .on_hover_text("Show the headers as text, with Find and Wrap");
                if *headers_raw {
                    widgets::viewer_tools(ui, headers_view, headers, copy_headers);
                    if let Some(tool) = queued {
                        tool.apply(ui.ctx(), headers_view, headers, None);
                    }
                } else if icons::button(ui, Icon::Copy, copy_headers, false).clicked()
                    || queued == Some(ViewerTool::Copy)
                {
                    ui.ctx().copy_text(headers.clone());
                }
            });
        }
        ResponseTab::Tests | ResponseTab::Events => {}
        ResponseTab::Request => {
            if let Some(text) = sent.as_ref() {
                widgets::viewer_tools(ui, sent_view, text, "Copy request");
                if let Some(tool) = queued {
                    tool.apply(ui.ctx(), sent_view, text, None);
                }
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
    let stream = response.as_ref().and_then(|r| r.stream.as_ref());
    let mut tabs = Vec::new();
    if let Some(stream) = stream {
        let received = usize::try_from(stream.received()).unwrap_or(usize::MAX);
        tabs.push(
            widgets::TabSpec::new(ResponseTab::Events, "Events")
                .badge(count_badge(received, Tone::Muted)),
        );
    }
    tabs.extend([
        widgets::TabSpec::new(ResponseTab::Body, "Body"),
        widgets::TabSpec::new(ResponseTab::Headers, "Headers")
            .badge(count_badge(headers_count, Tone::Muted)),
    ]);
    if history.is_some()
        || !results.is_empty()
        || !captured.is_empty()
        || script.is_some()
        || !console.is_empty()
    {
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
    // Leave room for More tools, Find and Copy, plus Raw when it switches the viewer.
    let gap = ui.spacing().item_spacing.x;
    let slot = icons::size() + gap;
    let reserve = match *tab {
        ResponseTab::Body if !preview.binary => 3.0 * slot,
        ResponseTab::Body => {
            if body_saved_here {
                slot
            } else {
                0.0
            }
        }
        ResponseTab::Headers => {
            let raw = widgets::action_width(ui, RAW) + gap;
            raw + if raw_open { 2.0 * slot } else { slot }
        }
        ResponseTab::Request => 3.0 * slot,
        ResponseTab::Tests | ResponseTab::Events => 0.0,
    };
    if let Some(history) = &mut history {
        let mut save_full = false;
        let full_tools = response.as_ref().filter(|_| partial);
        let reserve = reserve + full_tools.map_or(0.0, |_| if binary { slot } else { 2.0 * slot });
        history.tabs(ui, tabs, tab, reserve, |ui, tab| {
            if let Some(response) = full_tools {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    save_full = whole_body_tools(ui, response, binary);
                    tools(ui, tab);
                });
            } else {
                tools(ui, tab);
            }
        });
        if !history.shows_response() {
            history.show_request(ui);
            return None;
        }
        if let Some((notice, tone)) = notice {
            ui.add(egui::Label::new(tone_text(notice.as_str(), *tone)).truncate());
        }
        if save_full || (partial && queued == Some(ViewerTool::Save)) {
            return Some(PaneAction::SaveBody);
        }
    } else {
        ui.horizontal(|ui| {
            widgets::tab_strip(ui, &tabs, tab, tabs.len(), reserve, "More response tabs");
            tools(ui, *tab);
        });
    }
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
        ResponseTab::Events => {
            if let Some(stream) = response.as_ref().and_then(|r| r.stream.as_ref()) {
                let more = if stream.more_events > 0 {
                    format!(" · {} more not kept", stream.more_events)
                } else {
                    String::new()
                };
                ui.label(tone_text(
                    format!(
                        "{} events · stream {}{more}",
                        stream.events.len(),
                        stream.end.describe()
                    ),
                    if stream.end.is_normal() {
                        Tone::Muted
                    } else {
                        Tone::Warning
                    },
                ));
                crate::events::list(ui, id, &stream.events, event_selected, false);
            }
            None
        }
    };
    if close_filter || toggle_filter {
        if close_filter || filter.open {
            filter.close();
        } else {
            filter.open();
        }
        ui.ctx().request_repaint();
    }
    action.map(PaneAction::Row)
}

fn basic_history_tabs() -> Vec<widgets::TabSpec<'static, ResponseTab>> {
    vec![
        widgets::TabSpec::new(ResponseTab::Body, "Body"),
        widgets::TabSpec::new(ResponseTab::Headers, "Headers"),
        widgets::TabSpec::new(ResponseTab::Tests, "Tests").named("Check results"),
    ]
}

fn response_notice(
    ui: &mut Ui,
    notice: Option<&(String, Tone)>,
    response: Option<&imposter_app::core::execution::Response>,
    partial: bool,
    binary: bool,
) -> bool {
    let Some((notice, tone)) = notice else {
        return false;
    };
    let mut save = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if partial && let Some(response) = response {
                save = whole_body_tools(ui, response, binary);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(tone_text(notice.as_str(), *tone)).truncate());
            });
        });
    });
    save
}

fn whole_body_tools(
    ui: &mut Ui,
    response: &imposter_app::core::execution::Response,
    binary: bool,
) -> bool {
    // Bytes that are not text would be damaged as text.
    if !binary && icons::button(ui, Icon::Copy, "Copy full body", false).clicked() {
        ui.ctx()
            .copy_text(String::from_utf8_lossy(&response.body).into_owned());
    }
    icons::button(ui, Icon::Save, "Save body…", false).clicked()
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

fn wordmark() -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in [
        ("I", palette().text),
        ("’m", palette().accent),
        ("poster", palette().text),
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
        let job = wordmark();
        assert_eq!(job.text, "I’mposter");
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

#[cfg(test)]
mod tests {
    use super::{Drop, drop_zone, remaining};

    #[test]
    fn time_left_reads_in_the_largest_units_that_matter() {
        assert_eq!(remaining(-5), "0 s");
        assert_eq!(remaining(45_000), "45 s");
        assert_eq!(remaining(54 * 60_000 + 59_000), "54 min");
        assert_eq!(remaining(2 * 3_600_000 + 30_000), "2 h");
        assert_eq!(remaining(2 * 3_600_000 + 5 * 60_000), "2 h 5 min");
    }

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
