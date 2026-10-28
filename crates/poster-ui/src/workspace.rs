//! The Requests screen. Drawing only: it reads state, edits the editor's form fields in place
//! and records what the user asked for as actions.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use poster_app::core::ids::FolderId;
use poster_app::core::model::BodyType;

use crate::app::{Action, EditorTab, PosterApp, ResponseTab, Screen, State};
use crate::dialogs::{ConfirmPurpose, Dialog, NamePurpose};
use crate::editor::BODY_CHOICES;
use crate::icons::{self, Icon};
use crate::layout::{Orientation, SIDEBAR_WIDTH, WorkspaceLayout};
use crate::theme::{PALETTE, Tone, method_color};
use crate::tree::{Node, NodeId};
use crate::viewer::{self, Language};
use crate::widgets::{self, FileRequest, VarUi, ViewerState, name, tone_text};

const SEPARATOR: f32 = 12.0;

pub fn draw(app: &mut PosterApp, ui: &mut Ui) {
    let save = app.save_label();
    let save_text = app.save_button_text();
    let executing = app.is_executing();
    let variables_dirty = app.variables_dirty();
    let settings_dirty = app.state.settings_screen.dirty();
    let state = &mut app.state;
    crate::shortcuts::handle(state, ui.ctx());

    egui::Panel::top("screens")
        .frame(
            egui::Frame::new()
                .fill(PALETTE.surface)
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
                ui.label(RichText::new("Poster").size(20.0).strong());
                ui.add_space(24.0);
                for (screen, label) in [
                    (Screen::Requests, "Requests"),
                    (Screen::Variables, "Variables"),
                    (Screen::History, "History"),
                    (Screen::Settings, "Settings"),
                ] {
                    let dirty = match screen {
                        Screen::Variables => variables_dirty,
                        Screen::Settings => settings_dirty,
                        Screen::Requests | Screen::History => false,
                    };
                    let text = if dirty {
                        format!("{label} •")
                    } else {
                        label.to_owned()
                    };
                    let tab = widgets::tab(ui, state.screen == screen, &text);
                    name(&tab, label);
                    if tab.clicked() {
                        state.actions.push(Action::Screen(screen));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    environment_picker(state, ui);
                });
            });
        });
    egui::Panel::bottom("status")
        .frame(
            egui::Frame::new()
                .fill(PALETTE.surface)
                .inner_margin(egui::Margin::symmetric(16, 5)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let toggle = if state.layout.sidebar_visible {
                    "Hide Collections"
                } else {
                    "Show Collections"
                };
                if icons::button(ui, Icon::Sidebar, toggle, state.layout.sidebar_visible).clicked()
                {
                    state.layout.sidebar_visible = !state.layout.sidebar_visible;
                }
                layout_switch(&mut state.layout, ui);
                ui.separator();
                ui.label(tone_text(state.status.as_str(), Tone::Muted).small());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(tone_text("HTTP / 1.1", Tone::Muted).small().monospace());
                });
            });
        });
    if state.screen == Screen::Variables {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PALETTE.background).inner_margin(16))
            .show(ui, |ui| crate::variables_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::History {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PALETTE.background).inner_margin(16))
            .show(ui, |ui| crate::history_screen::draw(state, ui));
        return;
    }
    if state.screen == Screen::Settings {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PALETTE.background).inner_margin(16))
            .show(ui, |ui| crate::settings_screen::draw(state, ui));
        return;
    }
    if state.layout.sidebar_visible {
        let panel = egui::Panel::left("sidebar")
            .frame(egui::Frame::new().fill(PALETTE.surface).inner_margin(12))
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
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(PALETTE.background).inner_margin(16))
        .show(ui, |ui| {
            context_bar(state, ui, save, save_text);
            ui.add_space(8.0);
            command_bar(state, ui, executing);
            ui.add_space(12.0);
            split(state, ui);
        });
    let name = state.inspect.as_ref().map(|i| i.name.clone());
    if let Some(scope) = widgets::inspector(
        ui.ctx(),
        &mut state.inspect,
        state.snapshot.as_ref(),
        state.snapshot_error.as_deref(),
    ) && let Some(name) = name
    {
        state.actions.push(Action::OpenDefinition(scope, name));
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
        if ui
            .add_enabled(state.collection.is_some(), egui::Button::new("New Folder"))
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
            .clicked()
        {
            state.actions.push(Action::Prompt(Dialog::name(
                "New Request",
                "Name:",
                "",
                NamePurpose::NewRequest { folder: parent },
            )));
        }
        ui.separator();
        if ui.button("Import Collection…").clicked() {
            state.actions.push(Action::Import);
        }
        if ui.button("Import from curl…").clicked() {
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
        let picker_width = ui.available_width() - 2.0 * (28.0 + ui.spacing().item_spacing.x);
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
                        if ui
                            .selectable_label(
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
                if ui.button("Export Collection…").clicked() {
                    state.actions.push(Action::Export);
                }
                if ui.button("Delete Collection").clicked() {
                    state.actions.push(Action::Prompt(Dialog::Confirm {
                        title: "Delete collection".into(),
                        message: format!(
                            "Delete \"{collection_name}\" and its contents, \
                         including any unsaved edits?"
                        ),
                        confirm: "Delete".into(),
                        purpose: ConfirmPurpose::DeleteCollection(id),
                    }));
                }
            });
        } else {
            ui.add_enabled_ui(false, |ui| {
                icons::button(ui, Icon::More, "Collection actions", false)
            });
        }
    });
    if state.collections.is_empty() {
        ui.label(tone_text(
            "No collections yet. Create or import one with +.",
            Tone::Muted,
        ));
        return;
    }
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt("tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(tree) = &state.tree else {
                ui.label(tone_text("Loading…", Tone::Muted));
                return;
            };
            if tree.roots.is_empty() {
                ui.label(tone_text(
                    "This collection is empty. Add a request with +.",
                    Tone::Muted,
                ));
            }
            let mut actions = Vec::new();
            let roots = tree.roots.clone();
            let destinations = tree.destinations();
            nodes(ui, &roots, state.selected, &destinations, &mut actions);
            if egui::DragAndDrop::has_payload_of_type::<NodeId>(ui.ctx()) {
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
    destinations: &[(String, Option<FolderId>)],
    actions: &mut Vec<Action>,
) {
    let id = node.id();
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
    if ui.button("Delete").clicked() {
        actions.push(Action::Prompt(Dialog::Confirm {
            title: "Delete".into(),
            message: format!(
                "Delete \"{}\" and its contents, including any unsaved edits?",
                node.name()
            ),
            confirm: "Delete".into(),
            purpose: ConfirmPurpose::Delete(id),
        }));
    }
}

fn folder_of(node: &Node) -> Option<FolderId> {
    match node {
        Node::Folder { folder, .. } => Some(folder.id),
        Node::Request(_) => None,
    }
}

fn nodes(
    ui: &mut Ui,
    list: &[Node],
    selected: Option<NodeId>,
    destinations: &[(String, Option<FolderId>)],
    actions: &mut Vec<Action>,
) {
    for node in list {
        let id = node.id();
        let response = match node {
            Node::Folder { folder, children } => {
                let key = ui.make_persistent_id(("folder", folder.id.0));
                let collapsing = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    key,
                    true,
                );
                let (_, header, _) = collapsing
                    .show_header(ui, |ui| {
                        let button = egui::Button::selectable(
                            selected == Some(id),
                            RichText::new(&folder.name).strong(),
                        )
                        .sense(Sense::click_and_drag());
                        let response = ui.add(button);
                        name(&response, &format!("Folder {}", folder.name));
                        response
                    })
                    .body(|ui| nodes(ui, children, selected, destinations, actions));
                let response = header.inner;
                if response.dnd_hover_payload::<NodeId>().is_some() {
                    ui.painter().rect_stroke(
                        response.rect,
                        4.0,
                        egui::Stroke::new(1.5, PALETTE.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(dropped) = response.dnd_release_payload::<NodeId>() {
                    actions.push(Action::Move {
                        node: *dropped,
                        target: Some(folder.id),
                    });
                }
                response
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
                text.append(
                    &request.name,
                    4.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(13.0),
                        color: PALETTE.text,
                        ..Default::default()
                    },
                );
                let response = ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::Button::selectable(selected == Some(id), (text, egui::Atom::grow()))
                        .truncate()
                        .sense(Sense::click_and_drag()),
                );
                name(&response, &format!("{} {}", request.method, request.name));
                response.on_hover_text(&request.name)
            }
        };
        response.dnd_set_drag_payload(id);
        if response.clicked() {
            actions.push(match node {
                Node::Folder { .. } => Action::Select(id),
                Node::Request(request) => Action::OpenRequest(request.id),
            });
        }
        response.context_menu(|ui| node_menu(ui, node, destinations, actions));
    }
}

// ---- context and command bars -------------------------------------------------------------

fn context_bar(
    state: &mut State,
    ui: &mut Ui,
    save: Option<widgets::SaveStatus<'static>>,
    save_text: &str,
) {
    ui.horizontal(|ui| {
        let path = match &state.editor {
            None => "Select a request".to_owned(),
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
        ui.add(egui::Label::new(RichText::new(&path).strong().size(14.0)).truncate())
            .on_hover_text(&path);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(save) = save {
                if widgets::save_controls(ui, &save, save_text) {
                    state.actions.push(Action::Save);
                }
                if icons::button(ui, Icon::Terminal, "Copy as curl", false).clicked() {
                    state.actions.push(Action::CopyCurl);
                }
            }
        });
    });
}

fn environment_picker(state: &mut State, ui: &mut Ui) {
    let active = state.active_environment;
    let selected = active
        .and_then(|id| state.environments.iter().find(|e| e.id == id))
        .map_or("Global (no environment)".to_owned(), |e| e.name.clone());
    let combo = egui::ComboBox::from_id_salt("environment")
        .width(170.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(active.is_none(), "Global (no environment)")
                .clicked()
            {
                state.actions.push(Action::SetEnvironment(None));
            }
            for environment in &state.environments {
                if ui
                    .selectable_label(active == Some(environment.id), &environment.name)
                    .clicked()
                {
                    state
                        .actions
                        .push(Action::SetEnvironment(Some(environment.id)));
                }
            }
        });
    name(&combo.response, "Environment");
    combo.response.on_hover_text(
        "Global uses global and collection variables without an environment override.",
    );
}

/// Switch between side by side and stacked. The icon shows what a click switches to.
fn layout_switch(layout: &mut WorkspaceLayout, ui: &mut Ui) {
    let (icon, label) = match layout.orientation {
        Orientation::SideBySide => (Icon::Rows, "Stack request and response"),
        Orientation::Stacked => (Icon::Columns, "Put request and response side by side"),
    };
    if icons::button(ui, icon, label, false).clicked() {
        layout.orientation = layout.orientation.switched();
    }
}

fn command_bar(state: &mut State, ui: &mut Ui, executing: bool) {
    let State {
        editor,
        snapshot,
        inspect,
        actions,
        ..
    } = state;
    let Some(pane) = editor else { return };
    let mut vars = VarUi {
        snapshot: snapshot.as_ref(),
        inspect,
    };
    let mut url_edited = false;
    let mut edited = false;
    egui::Frame::new()
        .fill(PALETTE.surface)
        .stroke(egui::Stroke::new(1.0, PALETTE.border))
        .corner_radius(10)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.y = 36.0;
                edited |=
                    widgets::method_picker(ui, &mut pane.editor.method, &mut pane.custom_method);
                let send_width = 96.0;
                let width =
                    (ui.available_width() - send_width - ui.spacing().item_spacing.x).max(80.0);
                url_edited = widgets::var_edit(
                    ui,
                    &mut pane.editor.url,
                    "Request URL",
                    width,
                    false,
                    &mut vars,
                )
                .changed();
                let (label, fill) = if executing {
                    ("Cancel", PALETTE.warning)
                } else {
                    ("Send", PALETTE.accent)
                };
                let send = ui.add_sized(
                    vec2(send_width, 36.0),
                    egui::Button::new(RichText::new(label).color(PALETTE.on_accent).strong())
                        .fill(fill),
                );
                let send = send.on_hover_text(crate::shortcuts::hint(
                    ui.ctx(),
                    label,
                    &crate::shortcuts::SEND,
                ));
                if send.clicked() {
                    actions.push(Action::Send);
                }
            })
        });
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
            let width = (area.width() - SEPARATOR) * ratio;
            let first = Rect::from_min_size(area.min, vec2(width, area.height()));
            let handle = Rect::from_min_size(first.right_top(), vec2(SEPARATOR, area.height()));
            (
                first,
                handle,
                Rect::from_min_max(handle.right_top(), area.max),
            )
        }
        Orientation::Stacked => {
            let height = (area.height() - SEPARATOR) * ratio;
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
        if total > 0.0 {
            state.layout.set_ratio(orientation, ratio + moved / total);
        }
    }
    let color = if drag.hovered() || drag.dragged() {
        PALETTE.accent
    } else {
        PALETTE.border
    };
    let grip = match orientation {
        Orientation::SideBySide => vec2(3.0, 32.0),
        Orientation::Stacked => vec2(32.0, 3.0),
    };
    ui.painter()
        .rect_filled(Rect::from_center_size(handle.center(), grip), 2.0, color);
    for rect in [first, second] {
        ui.painter().rect(
            rect,
            12.0,
            PALETTE.surface,
            egui::Stroke::new(1.0, PALETTE.border),
            egui::StrokeKind::Inside,
        );
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

fn request_pane(state: &mut State, ui: &mut Ui) {
    let State {
        editor,
        snapshot,
        inspect,
        settings,
        actions,
        ..
    } = state;
    let Some(pane) = editor else {
        ui.add_space((ui.available_height() * 0.25).min(100.0));
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(">_")
                    .monospace()
                    .size(32.0)
                    .color(PALETTE.accent),
            );
            ui.heading("Start with a request");
            ui.add_space(4.0);
            ui.label(tone_text(
                "Select a request in the sidebar, or create one with +.",
                Tone::Muted,
            ));
        });
        return;
    };
    let mut vars = VarUi {
        snapshot: snapshot.as_ref(),
        inspect,
    };
    ui.horizontal_wrapped(|ui| {
        for (tab, label) in EditorTab::ALL {
            let response = widgets::tab(ui, pane.tab == tab, label);
            if tab == EditorTab::Settings {
                // The Settings screen tab has the same text; screen readers need them apart.
                name(&response, "Request settings");
            }
            if response.clicked() {
                pane.tab = tab;
            }
        }
    });
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

/// Below this width the response's status line and run picker get a row each.
const RESPONSE_HEADER_ROW: f32 = 640.0;

fn response_pane(state: &mut State, ui: &mut Ui) {
    let response_state = &mut state.response;
    let actions = &mut state.actions;
    // The status line takes the place of a "Response" heading.
    if ui.available_width() >= RESPONSE_HEADER_ROW {
        ui.horizontal(|ui| {
            if let Some(response) = &response_state.response {
                status_badges(ui, response);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                runs_picker(ui, response_state, actions);
            });
        });
    } else {
        if let Some(response) = &response_state.response {
            ui.horizontal_wrapped(|ui| status_text(ui, response));
        }
        ui.horizontal(|ui| runs_picker(ui, response_state, actions));
    }
    response_body(ui, response_state, "response", false);
}

/// The recorded runs of the open request, to show an earlier response.
fn runs_picker(ui: &mut Ui, response_state: &crate::app::ResponsePane, actions: &mut Vec<Action>) {
    if response_state.runs.is_empty() {
        return;
    }
    let shown = response_state.shown_run;
    let label = response_state
        .runs
        .iter()
        .find(|run| Some(run.id) == shown)
        .map_or_else(|| "Recorded runs".to_owned(), run_label);
    let combo = egui::ComboBox::from_id_salt("runs")
        .width(220.0_f32.min(ui.available_width()))
        .truncate()
        .selected_text(label)
        .show_ui(ui, |ui| {
            for run in &response_state.runs {
                if ui
                    .selectable_label(Some(run.id) == shown, run_label(run))
                    .clicked()
                {
                    actions.push(Action::ShowRun(run.id));
                }
            }
        });
    name(&combo.response, "Recorded runs");
}

/// The status line as plain text, which wraps in a narrow pane where badges cannot.
fn status_text(ui: &mut Ui, response: &poster_app::core::execution::Response) {
    let status = viewer::status(response);
    let part = |ui: &mut Ui, text: RichText| ui.add(egui::Label::new(text).extend());
    part(ui, tone_text(status.text.as_str(), status.tone).strong())
        .on_hover_text(status.detail.as_str());
    part(
        ui,
        tone_text(viewer::format_duration(response.elapsed), Tone::Muted),
    );
    if response.outcome == poster_app::core::execution::Outcome::Response {
        part(ui, tone_text(viewer::size_text(response), Tone::Muted));
    }
}

/// Status, time, size and content type of a response, as badges.
fn status_badges(ui: &mut Ui, response: &poster_app::core::execution::Response) {
    let status = viewer::status(response);
    widgets::badge(ui, status.text.as_str(), status.tone).on_hover_text(status.detail.as_str());
    widgets::badge(ui, &viewer::format_duration(response.elapsed), Tone::Muted);
    if response.outcome == poster_app::core::execution::Outcome::Response {
        widgets::badge(ui, &viewer::size_text(response), Tone::Muted);
        let content_type = viewer::content_type(&response.headers);
        ui.label(tone_text(
            content_type.split(';').next().unwrap_or_default(),
            Tone::Muted,
        ))
        .on_hover_text(content_type);
    }
}

/// Notice, body and headers of `pane`'s response, with its status line when `status`; `id`
/// names its viewers.
pub fn response_body(
    ui: &mut Ui,
    response_state: &mut crate::app::ResponsePane,
    id: &str,
    status: bool,
) {
    if status && let Some(response) = &response_state.response {
        ui.horizontal_wrapped(|ui| status_badges(ui, response));
    }
    if let Some((notice, tone)) = &response_state.notice {
        ui.label(tone_text(notice.as_str(), *tone));
    }
    let Some((preview, headers)) = &response_state.preview else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Your response will appear here").strong());
            ui.label(tone_text(
                "Send a request to inspect its status, headers and body.",
                Tone::Muted,
            ));
        });
        return;
    };
    let body = response_state
        .response
        .as_ref()
        .filter(|response| response.outcome == poster_app::core::execution::Outcome::TransportError)
        .map_or(preview.text.as_str(), |response| response.reason.as_str());
    let copy_body = if preview.limited {
        "Copy displayed body"
    } else {
        "Copy body"
    };
    let copy_headers = if preview.limited {
        "Copy displayed headers"
    } else {
        "Copy headers"
    };
    let narrow = ui.available_width() < widgets::TAB_ROW_MIN;
    let tools = |ui: &mut Ui,
                 tab: ResponseTab,
                 body_view: &mut ViewerState,
                 headers_view: &mut ViewerState| match tab {
        ResponseTab::Body => widgets::viewer_tools(ui, body_view, body, copy_body),
        ResponseTab::Headers => widgets::viewer_tools(ui, headers_view, headers, copy_headers),
    };
    ui.horizontal(|ui| {
        for (tab, label) in [
            (ResponseTab::Body, "Body"),
            (ResponseTab::Headers, "Headers"),
        ] {
            if widgets::tab(ui, response_state.tab == tab, label).clicked() {
                response_state.tab = tab;
            }
        }
        if !narrow {
            let tab = response_state.tab;
            tools(
                ui,
                tab,
                &mut response_state.body_view,
                &mut response_state.headers_view,
            );
        }
    });
    if narrow {
        let tab = response_state.tab;
        ui.horizontal(|ui| {
            tools(
                ui,
                tab,
                &mut response_state.body_view,
                &mut response_state.headers_view,
            );
        });
    }
    match response_state.tab {
        ResponseTab::Body => {
            widgets::code_viewer(
                ui,
                &format!("{id} body"),
                &mut response_state.body_view,
                body,
                preview.language,
            );
        }
        ResponseTab::Headers => {
            widgets::code_viewer(
                ui,
                &format!("{id} headers"),
                &mut response_state.headers_view,
                headers,
                Language::Plain,
            );
        }
    }
}

fn run_label(run: &poster_app::store::executions::ExecutionSummary) -> String {
    let when = viewer::local_time(run.timestamp);
    match run.outcome {
        poster_app::core::execution::Outcome::Response => format!("{when} · {}", run.status),
        poster_app::core::execution::Outcome::TransportError => format!("{when} · failed"),
        poster_app::core::execution::Outcome::Cancelled => format!("{when} · cancelled"),
    }
}
