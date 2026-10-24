//! The Requests screen. Drawing only: it reads state, edits the editor's form fields in place
//! and records what the user asked for as actions.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use poster_app::core::ids::FolderId;
use poster_app::core::model::BodyType;

use crate::app::{Action, EditorTab, PosterApp, ResponseTab, State};
use crate::dialogs::{ConfirmPurpose, Dialog, NamePurpose};
use crate::editor::BODY_CHOICES;
use crate::layout::{
    LayoutMode, Orientation, SIDEBAR_WIDTH, WorkspaceLayout, effective_orientation,
};
use crate::theme::{PALETTE, Tone};
use crate::tree::{Node, NodeId};
use crate::viewer::{self, Language};
use crate::widgets::{self, FileRequest, name, tone_text};

const SEPARATOR: f32 = 6.0;

pub fn draw(app: &mut PosterApp, ui: &mut Ui) {
    let save = app.save_label();
    let save_text = app.save_button_text();
    let executing = app.is_executing();
    let state = &mut app.state;

    egui::Panel::bottom("status").show(ui, |ui| {
        ui.label(tone_text(state.status.as_str(), Tone::Muted));
    });
    if state.layout.sidebar_visible {
        let panel = egui::Panel::left("sidebar")
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
    egui::CentralPanel::default().show(ui, |ui| {
        context_bar(state, ui, save, save_text);
        command_bar(state, ui, executing);
        ui.add_space(4.0);
        split(state, ui);
    });
}

// ---- sidebar ------------------------------------------------------------------------------

fn sidebar(state: &mut State, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Collections");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button("New", |ui| {
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
            });
        });
    });
    if state.collections.is_empty() {
        ui.label(tone_text(
            "No collections yet. Create one with New.",
            Tone::Muted,
        ));
        return;
    }
    let current = state
        .collections
        .iter()
        .find(|c| Some(c.id) == state.collection)
        .map(|c| (c.id, c.name.clone()));
    let picker = egui::ComboBox::from_id_salt("collection")
        .width(ui.available_width() - 8.0)
        .selected_text(
            current
                .as_ref()
                .map_or("Select a collection", |(_, name)| name.as_str()),
        )
        .show_ui(ui, |ui| {
            for collection in &state.collections {
                if ui
                    .selectable_label(Some(collection.id) == state.collection, &collection.name)
                    .clicked()
                {
                    state.actions.push(Action::SelectCollection(collection.id));
                }
            }
        });
    name(&picker.response, "Collection");
    if let Some((id, collection_name)) = current {
        ui.menu_button("Collection actions", |ui| {
            if ui.button("Rename Collection").clicked() {
                state.actions.push(Action::Prompt(Dialog::name("Rename Collection", "Name:", &collection_name, NamePurpose::RenameCollection(id))));
            }
            if ui.button("Delete Collection").clicked() {
                state.actions.push(Action::Prompt(Dialog::Confirm {
                    title: "Delete collection".into(),
                    message: format!("Delete \"{collection_name}\" and its contents, including any unsaved edits?"),
                    confirm: "Delete".into(),
                    purpose: ConfirmPurpose::DeleteCollection(id),
                }));
            }
        });
    }
    ui.separator();
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
                    "This collection is empty. Add a request with New.",
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

fn selected_folder(state: &State) -> Option<FolderId> {
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

fn method_color(method: &str) -> egui::Color32 {
    match method {
        "GET" => PALETTE.ok,
        "POST" => PALETTE.warning,
        "DELETE" => PALETTE.error,
        "PUT" | "PATCH" => crate::theme::token_color(crate::highlight::Token::Key),
        _ => PALETTE.text_muted,
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
                let response = ui.add(
                    egui::Button::selectable(selected == Some(id), text)
                        .sense(Sense::click_and_drag()),
                );
                name(&response, &format!("{} {}", request.method, request.name));
                response
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
    save: Option<(String, bool, Tone)>,
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
        ui.label(RichText::new(path).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            layout_menu(&mut state.layout, ui);
            let toggle = if state.layout.sidebar_visible {
                "Hide Collections"
            } else {
                "Show Collections"
            };
            if ui.button(toggle).clicked() {
                state.layout.sidebar_visible = !state.layout.sidebar_visible;
            }
            if let Some((status, enabled, tone)) = save {
                if ui
                    .add_enabled(enabled, egui::Button::new(save_text))
                    .clicked()
                {
                    state.actions.push(Action::Save);
                }
                ui.label(tone_text(status, tone));
            }
        });
    });
}

fn layout_menu(layout: &mut WorkspaceLayout, ui: &mut Ui) {
    ui.menu_button("Layout", |ui| {
        for (mode, label) in [
            (LayoutMode::Auto, "Auto"),
            (LayoutMode::SideBySide, "Side by side"),
            (LayoutMode::Stacked, "Stacked"),
        ] {
            if ui.radio(layout.mode == mode, label).clicked() {
                layout.mode = mode;
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Reset layout").clicked() {
            *layout = WorkspaceLayout::default();
            ui.close();
        }
    });
}

fn command_bar(state: &mut State, ui: &mut Ui, executing: bool) {
    let Some(pane) = &mut state.editor else {
        return;
    };
    let mut url_edited = false;
    let mut edited = false;
    ui.horizontal(|ui| {
        edited |= widgets::method_picker(ui, &mut pane.editor.method, &mut pane.custom_method);
        let send_width = 90.0;
        let url = ui.add(
            egui::TextEdit::singleline(&mut pane.editor.url)
                .hint_text("Enter request URL")
                .desired_width(ui.available_width() - send_width - 8.0),
        );
        name(&url, "Request URL");
        url_edited = url.changed();
        let (label, fill) = if executing {
            ("Cancel", PALETTE.warning)
        } else {
            ("Send", PALETTE.accent)
        };
        let send = ui.add_sized(
            vec2(send_width, 26.0),
            egui::Button::new(RichText::new(label).color(PALETTE.on_accent).strong()).fill(fill),
        );
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter);
        if send.clicked() || ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
            state.actions.push(Action::Send);
        }
    });
    let save = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
    if ui.input_mut(|i| i.consume_shortcut(&save)) {
        state.actions.push(Action::Save);
    }
    if url_edited || edited {
        state.actions.push(Action::Edited {
            url: url_edited,
            params: false,
        });
    }
}

// ---- request / response split ---------------------------------------------------------------

fn split(state: &mut State, ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    let orientation = effective_orientation(area.width(), state.layout.mode, state.orientation);
    state.orientation = Some(orientation);
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
    ui.painter().rect_filled(handle.shrink(2.0), 2.0, color);
    ui.scope_builder(
        UiBuilder::new().max_rect(first).id_salt("request pane"),
        |ui| request_pane(state, ui),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(second.shrink2(vec2(4.0, 0.0)))
            .id_salt("response pane"),
        |ui| response_pane(state, ui),
    );
    ui.allocate_rect(area, Sense::hover());
}

fn request_pane(state: &mut State, ui: &mut Ui) {
    let defaults = state.settings.clone();
    let Some(pane) = &mut state.editor else {
        ui.centered_and_justified(|ui| {
            ui.label(tone_text(
                "Select a request in the sidebar, or create one with New.",
                Tone::Muted,
            ))
        });
        return;
    };
    ui.horizontal_wrapped(|ui| {
        for (tab, label) in EditorTab::ALL {
            ui.selectable_value(&mut pane.tab, tab, label);
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
            EditorTab::Params => {
                params = widgets::rows_table(
                    ui,
                    "Params",
                    &mut pane.editor.queries,
                    &mut pane.confirm_clear[0],
                );
            }
            EditorTab::Headers => {
                edited = widgets::rows_table(
                    ui,
                    "Headers",
                    &mut pane.editor.headers,
                    &mut pane.confirm_clear[1],
                );
            }
            EditorTab::Cookies => {
                edited = widgets::rows_table(
                    ui,
                    "Cookies",
                    &mut pane.editor.cookies,
                    &mut pane.confirm_clear[2],
                );
            }
            EditorTab::Auth => edited = widgets::auth_form(ui, &mut pane.editor.auth),
            EditorTab::Settings => {
                edited =
                    widgets::options_form(ui, &mut pane.editor.options, &defaults, &pane.invalid);
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
                        edited |= widgets::body_editor(
                            ui,
                            &mut pane.editor.body_text,
                            pane.editor.body_type == BodyType::Json,
                        );
                    }
                    BodyType::Urlencoded | BodyType::FormData => {
                        edited |= widgets::rows_table(
                            ui,
                            "Fields",
                            &mut pane.editor.form,
                            &mut pane.confirm_clear[3],
                        );
                    }
                    BodyType::Multipart => {
                        edited |= widgets::multipart_table(
                            ui,
                            &mut pane.editor.multipart,
                            &mut pane.confirm_clear[4],
                            &mut pick,
                        );
                    }
                    BodyType::Binary => {
                        ui.horizontal(|ui| {
                            let path = ui.add(
                                egui::TextEdit::singleline(&mut pane.editor.binary_path)
                                    .hint_text("File path")
                                    .desired_width(ui.available_width() - 110.0),
                            );
                            name(&path, "Binary file path");
                            edited |= path.changed();
                            if ui.button("Choose file…").clicked() {
                                pick = Some(FileRequest::Binary);
                            }
                        });
                    }
                }
            }
        });
    if let Some(target) = pick {
        state.actions.push(Action::PickFile(target));
    }
    if edited || params {
        state.actions.push(Action::Edited { url: false, params });
    }
}

fn response_pane(state: &mut State, ui: &mut Ui) {
    let response_state = &mut state.response;
    ui.horizontal(|ui| {
        ui.heading("Response");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !response_state.runs.is_empty() {
                let shown = response_state.shown_run;
                let label = response_state
                    .runs
                    .iter()
                    .find(|run| Some(run.id) == shown)
                    .map_or_else(|| "Recorded runs".to_owned(), run_label);
                let combo = egui::ComboBox::from_id_salt("runs")
                    .width(220.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for run in &response_state.runs {
                            if ui
                                .selectable_label(Some(run.id) == shown, run_label(run))
                                .clicked()
                            {
                                state.actions.push(Action::ShowRun(run.id));
                            }
                        }
                    });
                name(&combo.response, "Recorded runs");
            }
        });
    });
    if let Some(response) = &response_state.response {
        let status = viewer::status(response);
        ui.horizontal(|ui| {
            ui.label(tone_text(status.text.as_str(), status.tone))
                .on_hover_text(status.detail.as_str());
            ui.label(viewer::format_duration(response.elapsed));
            if response.outcome == poster_app::core::execution::Outcome::Response {
                ui.label(viewer::size_text(response));
                let content_type = viewer::content_type(&response.headers);
                ui.label(tone_text(
                    content_type.split(';').next().unwrap_or_default(),
                    Tone::Muted,
                ))
                .on_hover_text(content_type);
            }
        });
    }
    if let Some((notice, tone)) = &response_state.notice {
        ui.label(tone_text(notice.as_str(), *tone));
    }
    let Some((preview, headers)) = &response_state.preview else {
        return;
    };
    ui.horizontal(|ui| {
        ui.selectable_value(&mut response_state.tab, ResponseTab::Body, "Body");
        ui.selectable_value(&mut response_state.tab, ResponseTab::Headers, "Headers");
    });
    let limited = preview.limited;
    match response_state.tab {
        ResponseTab::Body => {
            let copy = if limited {
                "Copy displayed body"
            } else {
                "Copy body"
            };
            let text =
                if response_state.response.as_ref().is_some_and(|r| {
                    r.outcome == poster_app::core::execution::Outcome::TransportError
                }) {
                    response_state
                        .response
                        .as_ref()
                        .map(|r| r.reason.clone())
                        .unwrap_or_default()
                } else {
                    preview.text.clone()
                };
            widgets::code_viewer(
                ui,
                "response body",
                &mut response_state.body_view,
                &text,
                preview.language,
                copy,
            );
        }
        ResponseTab::Headers => {
            let copy = if limited {
                "Copy displayed headers"
            } else {
                "Copy headers"
            };
            widgets::code_viewer(
                ui,
                "response headers",
                &mut response_state.headers_view,
                headers,
                Language::Plain,
                copy,
            );
        }
    }
}

fn run_label(run: &poster_app::store::executions::ExecutionSummary) -> String {
    let time = run.timestamp.to_rfc3339();
    let when = time.get(..19).unwrap_or(&time).replace('T', " ");
    match run.outcome {
        poster_app::core::execution::Outcome::Response => format!("{when} · {}", run.status),
        poster_app::core::execution::Outcome::TransportError => format!("{when} · failed"),
        poster_app::core::execution::Outcome::Cancelled => format!("{when} · cancelled"),
    }
}
