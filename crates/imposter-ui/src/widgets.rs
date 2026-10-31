//! Reusable widgets. Each returns whether the user changed anything, so the caller decides
//! what an edit means (dirty state, autosave, URL sync).

use egui::text::LayoutJob;
use egui::{Color32, FontId, RichText, TextFormat, Ui};
use imposter_app::core::model::{
    AuthKind, FormItem, FormKind, ParamItem, STANDARD_METHODS, is_method_token,
};
use imposter_app::core::settings::AppSettings;

use crate::editor::{
    AUTH_CHOICES, AuthForm, OPTION_GROUPS, OptionField, OptionsForm, Override, default_hint,
};
use crate::highlight::{self, Spans};
use crate::icons::{self, Icon};
use crate::theme::{Tone, palette, token_color};
use crate::vars;
use crate::viewer::{self, Language};
use imposter_app::core::variables::Snapshot;

/// Give a widget an accessible name when it has no visible label (screen readers, UI tests).
pub fn name(response: &egui::Response, label: &str) {
    let label = label.to_owned();
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
}

pub fn tone_text(text: impl Into<String>, tone: Tone) -> RichText {
    RichText::new(text.into()).color(palette().tone(tone))
}

pub fn tab(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    let text = RichText::new(label).color(if selected {
        palette().accent
    } else {
        palette().text_muted
    });
    let response = ui.add(egui::Button::selectable(selected, text).min_size(egui::vec2(0.0, 30.0)));
    if selected {
        let rect = response.rect;
        ui.painter().line_segment(
            [
                rect.left_bottom() + egui::vec2(8.0, -1.0),
                rect.right_bottom() + egui::vec2(-8.0, -1.0),
            ],
            egui::Stroke::new(2.0, palette().accent),
        );
    }
    response
}

pub fn badge(ui: &mut Ui, text: &str, tone: Tone) -> egui::Response {
    egui::Frame::new()
        .fill(palette().input)
        .stroke(egui::Stroke::new(1.0, palette().border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            // A badge keeps its text on one line; the row around it wraps instead.
            ui.add(egui::Label::new(tone_text(text, tone).small().strong()).extend())
        })
        .inner
}

/// Variables for the editor's text fields this frame: what is defined, and the inspector.
pub struct VarUi<'a> {
    pub snapshot: Option<&'a Snapshot>,
    pub inspect: &'a mut Option<Inspection>,
}

/// An open variable inspector: explicit, so values never appear passively.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub name: String,
    /// Opened from a password field: values stay hidden until revealed.
    pub masked: bool,
    pub anchor: egui::Pos2,
}

/// Completion popup state of one field, kept in egui memory.
/// Exists only while suggestions are open: from typing inside an unfinished `{{name` until
/// it is chosen, dismissed or the cursor leaves the token.
#[derive(Debug, Clone, Copy, Default)]
struct Completion {
    selected: usize,
    /// The popup's rect last frame, so clicking it does not count as leaving the field.
    rect: Option<egui::Rect>,
    /// The token's start and the cursor (bytes), kept for the frame in which pressing the
    /// popup has already taken focus from the field.
    token: (usize, usize),
}

fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Overlay `{{tokens}}` on highlighted spans: defined names in the accent color, unknown ones
/// in the error color, both underlined. Only colors change, never the text.
pub fn variable_job(
    text: &str,
    spans: &Spans,
    font: &FontId,
    wrap: f32,
    snapshot: Option<&Snapshot>,
) -> LayoutJob {
    let mut job = spans_job(text, spans, font, wrap);
    for token in vars::tokens(text) {
        let defined = vars::resolves(snapshot, &token.name);
        let color = if defined {
            palette().accent
        } else {
            palette().error
        };
        restyle(&mut job, token.range, |format| {
            format.color = color;
            format.underline = egui::Stroke::new(1.0, color);
        });
    }
    job
}

/// A text field that marks `{{variables}}`, suggests names after `{{` and opens the inspector
/// when a token is clicked (or Alt+Enter is pressed on it).
pub fn var_edit(
    ui: &mut Ui,
    text: &mut String,
    label: &str,
    width: f32,
    password: bool,
    vars: &mut VarUi,
) -> egui::Response {
    var_edit_with(ui, text, label, width, password, vars, None)
}

/// The id of the variable field named `label` drawn next in `ui`.
pub fn var_edit_id(ui: &Ui, label: &str) -> egui::Id {
    ui.make_persistent_id(("variable field", label))
}

/// A paste into the focused field `id` that would replace all of `current` (empty, or all
/// of it selected) and that `accept` wants, taken out of this frame's input so the field
/// does not insert it.
pub fn take_whole_paste(
    ui: &Ui,
    id: egui::Id,
    current: &str,
    accept: impl Fn(&str) -> bool,
) -> Option<String> {
    let ctx = ui.ctx();
    if !ctx.memory(|m| m.has_focus(id)) {
        return None;
    }
    let all_selected = egui::TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .is_some_and(|range| {
            let [start, end] = range.sorted_cursors();
            let at = |chars: usize| egui::text::CCursor::new(chars).index;
            start.index == at(0) && end.index == at(current.chars().count())
        });
    if !current.is_empty() && !all_selected {
        return None;
    }
    ui.input_mut(|i| {
        let at = i
            .events
            .iter()
            .position(|e| matches!(e, egui::Event::Paste(text) if accept(text)))?;
        match i.events.remove(at) {
            egui::Event::Paste(text) => Some(text),
            _ => None,
        }
    })
}

fn var_edit_with(
    ui: &mut Ui,
    text: &mut String,
    label: &str,
    width: f32,
    password: bool,
    vars: &mut VarUi,
    multiline: Option<(bool, &str)>,
) -> egui::Response {
    let id = var_edit_id(ui, label);
    let ctx = ui.ctx().clone();
    let completion: Option<Completion> = ctx.data(|d| d.get_temp(id));
    // Not `rect_contains_pointer`: that is false whenever another layer (the popup) is on top.
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    let hovering_popup = completion
        .and_then(|c| c.rect)
        .zip(pointer)
        .is_some_and(|(rect, at)| rect.contains(at));
    let focused = ctx.memory(|m| m.has_focus(id));
    let (mut step, mut accept, mut dismiss) = (0_i64, false, false);
    if focused && completion.is_some() {
        ui.input_mut(|i| {
            step += i64::from(i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            step -= i64::from(i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
            accept = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            dismiss = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
    }

    let snapshot = vars.snapshot;
    let (font, body_json) = match multiline {
        Some((json, _)) => (FontId::monospace(13.0), json),
        None => (egui::TextStyle::Body.resolve(ui.style()), false),
    };
    let single_line = multiline.is_none();
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
        // A single-line field scrolls sideways; wrapping would show its text on two lines.
        let wrap = if single_line { f32::INFINITY } else { wrap };
        let content = buffer.as_str();
        let spans = if body_json && content.len() <= HIGHLIGHT_LIMIT {
            highlight::json(content)
        } else {
            vec![(0..content.len(), highlight::Token::Plain)]
        };
        let job = if content.len() <= HIGHLIGHT_LIMIT {
            variable_job(content, &spans, &font, wrap, snapshot)
        } else {
            spans_job(content, &spans, &font, wrap)
        };
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut edit = match multiline {
        Some((_, hint)) => egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(10)
            .hint_text(hint),
        None => egui::TextEdit::singleline(text),
    }
    .id(id)
    .desired_width(width)
    .password(password);
    if !password {
        edit = edit.layouter(&mut layouter);
    }
    let output = edit.show(ui);
    let mut response = output.response.response;
    name(&response, label);
    let tokens = vars::tokens(text);
    if !tokens.is_empty() {
        let description: Vec<String> = tokens
            .iter()
            .map(|t| {
                let state = if snapshot.is_some_and(|s| s.definitions.contains_key(&t.name)) {
                    "resolved"
                } else {
                    "not defined"
                };
                format!("{}: {state}", t.name)
            })
            .collect();
        let description = format!("{}. Inspect variable: Alt+Enter", description.join("; "));
        ctx.accesskit_node_builder(id, |node| node.set_description(description));
    }
    let cursor = output
        .cursor_range
        .map(|range| char_to_byte(text, range.primary.index.0));

    // Inspect on click or Alt+Enter.
    let alt_enter =
        response.has_focus() && ui.input_mut(|i| i.consume_shortcut(&crate::shortcuts::INSPECT));
    if (response.clicked() || alt_enter)
        && let Some(token) = cursor.and_then(|c| vars::token_at(text, c))
    {
        *vars.inspect = Some(Inspection {
            name: token.name,
            masked: password,
            anchor: response.rect.left_bottom(),
        });
    }

    // Suggest names while typing an unfinished `{{name`; clicking into a token only inspects.
    let live = cursor
        .filter(|_| response.has_focus())
        .and_then(|c| vars::open_token(text, c).map(|(start, _)| (start, c)));
    let token = match (completion, live) {
        _ if dismiss => None,
        (Some(_), Some(token)) => Some(token),
        (None, Some(token)) if response.changed() => Some(token),
        (Some(state), None) if hovering_popup && !response.has_focus() => {
            let (start, at) = state.token;
            (text.is_char_boundary(at)
                && vars::open_token(text, at).is_some_and(|(s, _)| s == start))
            .then_some(state.token)
        }
        _ => None,
    };
    let candidates = match (token, snapshot) {
        (Some((start, cursor)), Some(snapshot)) => {
            let query = &text[start + 2..cursor];
            Some((start, cursor, vars::suggestions(snapshot, query)))
        }
        _ => None,
    };
    match candidates {
        Some((start, cursor, items)) if !items.is_empty() => {
            let mut state = completion.unwrap_or_default();
            state.token = (start, cursor);
            #[allow(
                clippy::cast_possible_wrap,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )] // tiny list indexes
            {
                state.selected =
                    (state.selected as i64 + step).rem_euclid(items.len() as i64) as usize;
            }
            let mut chosen = accept.then(|| items[state.selected].name.clone());
            let area = egui::Area::new(id.with("completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 2.0))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(width.clamp(200.0, 360.0));
                        for (index, item) in items.iter().enumerate() {
                            let row = format!("{} · {}", item.name, item.detail);
                            if ui.selectable_label(index == state.selected, row).clicked() {
                                chosen = Some(item.name.clone());
                            }
                        }
                    });
                });
            state.rect = Some(area.response.rect);
            if let Some(name) = chosen {
                let (completed, at) = vars::complete(text, start, cursor, &name);
                *text = completed;
                let mut edit_state = output.state;
                let chars = text[..at].chars().count();
                edit_state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(chars),
                    )));
                edit_state.store(&ctx, id);
                ctx.memory_mut(|m| m.request_focus(id));
                response.mark_changed();
                ctx.data_mut(|d| d.remove_temp::<Completion>(id));
            } else {
                ctx.data_mut(|d| d.insert_temp(id, state));
            }
        }
        _ => {
            ctx.data_mut(|d| d.remove_temp::<Completion>(id));
        }
    }
    response
}

/// Focus a single-line field and select all of its text, as Cmd/Ctrl+L does for the URL.
pub fn focus_and_select_all(ctx: &egui::Context, field: &egui::Response, chars: usize) {
    field.request_focus();
    let mut state = egui::TextEdit::load_state(ctx, field.id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(chars),
        )));
    state.store(ctx, field.id);
}

/// What the user chose in the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorAction {
    /// Edit the definition in effect on the Variables screen.
    Open(vars::Scope),
    /// Make a saved value the current one.
    Choose { scope: vars::Scope, value: String },
}

/// One line of a saved value, shortened: the list is for recognizing values, not reading them.
pub fn saved_label(value: &str, masked: bool) -> String {
    const PREVIEW: usize = 60;
    if masked {
        "••••••••".to_owned()
    } else if value.is_empty() {
        "Empty value".to_owned()
    } else {
        let line = value.lines().next().unwrap_or_default();
        if line.chars().count() > PREVIEW || line.len() < value.len() {
            let cut: String = line.chars().take(PREVIEW).collect();
            format!("{cut}…")
        } else {
            line.to_owned()
        }
    }
}

/// The inspector popover for an open [`Inspection`]. `blocked` is a scope whose saved values
/// cannot be chosen now (the Variables screen has unsaved edits to it).
pub fn inspector(
    ctx: &egui::Context,
    inspection: &mut Option<Inspection>,
    snapshot: Option<&Snapshot>,
    error: Option<&str>,
    blocked: Option<vars::Scope>,
) -> Option<InspectorAction> {
    let current = inspection.as_mut()?;
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut open = None;
    let reveal_id = egui::Id::unique(("inspector reveal", current.name.as_str()));
    let mut revealed: bool = ctx.data(|d| d.get_temp(reveal_id)).unwrap_or(false);
    // One example per opening, so it does not change every frame.
    let sample_id = egui::Id::unique(("generated sample", current.name.as_str()));
    let area = egui::Area::new(egui::Id::unique("variable inspector"))
        .order(egui::Order::Foreground)
        .fixed_pos(current.anchor + egui::vec2(0.0, 4.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(320.0);
                ui.set_max_width(480.0);
                ui.heading(current.name.as_str());
                let definitions = snapshot.and_then(|s| s.definitions.get(&current.name));
                match (definitions, error) {
                    (_, Some(error)) => {
                        ui.label(tone_text(error, Tone::Error));
                    }
                    (Some(definitions), None) => {
                        egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                            for (heading, value) in vars::describe(definitions, current.masked && !revealed) {
                                ui.label(tone_text(heading, Tone::Muted).small());
                                ui.add(egui::Label::new(RichText::new(value).monospace()).selectable(true));
                                ui.add_space(6.0);
                            }
                        });
                    }
                    (None, None) => match imposter_app::core::generate::describe(&current.name) {
                        Some(Ok(description)) => {
                            ui.label(tone_text("Generated on each send", Tone::Muted).small());
                            ui.label(description);
                            let sample: String = ctx.data_mut(|d| {
                                d.get_temp_mut_or_insert_with(sample_id, || {
                                    imposter_app::core::generate::generate(
                                        &current.name,
                                        &mut imposter_app::executor::SystemEntropy::default(),
                                    )
                                    .unwrap_or_default()
                                })
                                .clone()
                            });
                            ui.label(tone_text(format!("For example: {sample}"), Tone::Muted));
                        }
                        Some(Err(reason)) => {
                            ui.label(tone_text(reason, Tone::Error));
                        }
                        None => {
                            ui.label(tone_text("Not defined", Tone::Error));
                        }
                    },
                }
                let effective = definitions.and_then(|d| d.first()).filter(|_| error.is_none());
                if let Some(effective) = effective.filter(|d| !d.saved.is_empty())
                    && let Some(scope) = vars::Scope::of(&effective.source)
                {
                    ui.separator();
                    ui.label(tone_text("Saved values", Tone::Muted).small());
                    let enabled = blocked != Some(scope);
                    let masked = current.masked && !revealed;
                    let list = ui.add_enabled_ui(enabled, |ui| {
                        egui::ScrollArea::vertical().id_salt("saved values").max_height(160.0).show(ui, |ui| {
                            for (index, value) in effective.saved.iter().enumerate() {
                                let chosen = *value == effective.value;
                                let text = saved_label(value, masked);
                                let row = ui.selectable_label(chosen, RichText::new(&text).monospace());
                                name(&row, &format!("Use saved value {}: {text}", index + 1));
                                if row.clicked() && !chosen {
                                    open = Some(InspectorAction::Choose { scope, value: value.clone() });
                                    close = true;
                                }
                            }
                        });
                    });
                    if !enabled {
                        list.response.on_disabled_hover_text(
                            "The Variables screen has unsaved changes to this scope; save or revert them first.",
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if current.masked && definitions.is_some() && ui.button(if revealed { "Hide value" } else { "Reveal" }).clicked() {
                        revealed = !revealed;
                    }
                    let target = definitions.and_then(|d| d.first()).map_or(Some(vars::Scope::Global), |d| vars::Scope::of(&d.source));
                    let button = ui.add_enabled(target.is_some(), egui::Button::new("Open definition"));
                    let button = if target.is_none() {
                        button.on_disabled_hover_text("The original collection was deleted; this value comes from execution history.")
                    } else {
                        button
                    };
                    if button.clicked() {
                        open = target.map(InspectorAction::Open);
                        close = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        });
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
        && !area.response.contains_pointer()
        && !area.response.hovered();
    ctx.data_mut(|d| d.insert_temp(reveal_id, revealed));
    if close || (clicked_outside && !ctx.input(|i| i.pointer.any_released())) {
        ctx.data_mut(|d| {
            d.remove_temp::<bool>(reveal_id);
            d.remove_temp::<String>(sample_id);
        });
        *inspection = None;
    }
    open
}

/// Above this size the body editor stops highlighting (see AGENTS.md: editor decision).
pub const HIGHLIGHT_LIMIT: usize = 64 * 1024;

pub fn spans_job(text: &str, spans: &Spans, font: &FontId, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, token) in spans {
        job.append(
            &text[range.clone()],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: token_color(*token),
                ..TextFormat::default()
            },
        );
    }
    job.wrap.max_width = wrap;
    job
}

/// A left-aligned muted heading occupying exactly `width`.
pub fn heading_cell(ui: &mut Ui, width: f32, text: &str) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(width);
            ui.label(tone_text(text, Tone::Muted));
        },
    );
}

/// A screen's save state: one word, its explanation for the hover text, and whether Save
/// can run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveStatus<'a> {
    pub text: &'a str,
    pub detail: &'a str,
    pub tone: Tone,
    pub enabled: bool,
}

/// The save controls every screen shares, at its top right: the state, then a disk icon named
/// `action` with the shortcut in its hover text. Call inside a right-to-left layout. Returns
/// whether Save was clicked.
pub fn save_controls(ui: &mut Ui, status: &SaveStatus<'_>, action: &str) -> bool {
    let hint = crate::shortcuts::hint(ui.ctx(), action, &crate::shortcuts::SAVE);
    let clicked = ui
        .add_enabled_ui(status.enabled, |ui| {
            icons::button_with_hint(ui, Icon::Save, action, &hint, false)
        })
        .inner
        .clicked();
    let label = ui.label(tone_text(status.text, status.tone));
    if !status.detail.is_empty() {
        label.on_hover_text(status.detail);
    }
    clicked
}

/// How long a remove button stays armed after its first click.
pub const REMOVE_ARMED_SECONDS: f64 = 3.0;

/// A × button that removes only on a second click: the first arms it (red, "Click again to
/// remove …"). It disarms after [`REMOVE_ARMED_SECONDS`] or once the pointer is somewhere
/// else outside it. Returns whether to remove now.
pub fn remove_button(ui: &mut Ui, id: egui::Id, what: &str, height: f32) -> bool {
    let (now, pointer) = ui.input(|i| (i.time, i.pointer.latest_pos()));
    // When it was armed, and where the pointer was then.
    let armed_at: Option<(f64, Option<egui::Pos2>)> = ui.data(|d| d.get_temp(id));
    let armed = armed_at.is_some_and(|(at, _)| now - at < REMOVE_ARMED_SECONDS);
    let button = if armed {
        egui::Button::new(RichText::new("×").strong().color(palette().error))
            .stroke(egui::Stroke::new(1.0, palette().error))
    } else {
        egui::Button::new("×")
    };
    let response = ui.add_sized([24.0, height], button);
    let label = if armed {
        format!("Click again to remove {what}")
    } else {
        format!("Remove {what}")
    };
    name(&response, &label);
    let response = response.on_hover_text(&label);
    let disarm = |ui: &mut Ui| ui.data_mut(|d| d.remove_temp::<(f64, Option<egui::Pos2>)>(id));
    if response.clicked() {
        if armed {
            disarm(ui);
            return true;
        }
        ui.data_mut(|d| d.insert_temp(id, (now, pointer)));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(REMOVE_ARMED_SECONDS));
        return false;
    }
    if let Some((at, origin)) = armed_at {
        let away = pointer.is_some_and(|p| !response.rect.contains(p) && Some(p) != origin);
        if !armed || away {
            disarm(ui);
        } else {
            let left = (REMOVE_ARMED_SECONDS - (now - at)).max(0.0);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
    }
    false
}

/// One editable row list: enabled, name, value, remove; then Add and Clear all.
pub fn rows_table(
    ui: &mut Ui,
    id: &str,
    rows: &mut Vec<ParamItem>,
    confirm_clear: &mut bool,
    vars: &mut VarUi,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    let height = ui.spacing().interact_size.y;
    let spacing = ui.spacing().item_spacing.x;
    let available = ui.available_width() - 2.0 * 24.0 - 3.0 * spacing;
    let name_width = (available * 0.35).clamp(110.0, 280.0);
    let value_width = (available - name_width).max(120.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0 + spacing);
        heading_cell(ui, name_width, "Name");
        heading_cell(ui, value_width, "Value");
    });
    for (index, row) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let enabled = ui.add_sized(
                [24.0, height],
                egui::Checkbox::without_text(&mut row.is_enabled),
            );
            name(&enabled, &format!("{id} row {} enabled", index + 1));
            changed |= enabled.changed();
            changed |= var_edit(
                ui,
                &mut row.name,
                &format!("{id} row {} name", index + 1),
                name_width,
                false,
                vars,
            )
            .changed();
            changed |= var_edit(
                ui,
                &mut row.value,
                &format!("{id} row {} value", index + 1),
                value_width,
                false,
                vars,
            )
            .changed();
            let what = format!("{id} row {}", index + 1);
            if remove_button(ui, egui::Id::unique(("remove", id, index)), &what, height) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        rows.remove(index);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui
            .button(format!("Add {}", id.to_lowercase().trim_end_matches('s')))
            .clicked()
        {
            rows.push(ParamItem::new("", ""));
        }
        changed |= clear_all(ui, rows, confirm_clear);
    });
    changed
}

/// "Clear all" asks once inline before removing every row.
fn clear_all<T>(ui: &mut Ui, rows: &mut Vec<T>, confirm: &mut bool) -> bool {
    if rows.is_empty() {
        *confirm = false;
        return false;
    }
    if *confirm {
        ui.label(tone_text("Remove all rows?", Tone::Warning));
        if ui.button("Remove all").clicked() {
            rows.clear();
            *confirm = false;
            return true;
        }
        if ui.button("Keep").clicked() {
            *confirm = false;
        }
    } else if ui.button("Clear all").clicked() {
        *confirm = true;
    }
    false
}

/// What a click on "Choose file…" asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRequest {
    Binary,
    /// Add a new multipart file field.
    MultipartNew,
}

pub fn multipart_table(
    ui: &mut Ui,
    rows: &mut Vec<FormItem>,
    confirm_clear: &mut bool,
    pick: &mut Option<FileRequest>,
    vars: &mut VarUi,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    let height = ui.spacing().interact_size.y;
    let spacing = ui.spacing().item_spacing.x;
    let fixed = [24.0, 110.0, 70.0, 120.0, 110.0, 24.0];
    let value_width = (ui.available_width() - fixed.iter().sum::<f32>() - 6.0 * spacing).max(140.0);
    let widths = [
        fixed[0],
        fixed[1],
        value_width,
        fixed[2],
        fixed[3],
        fixed[4],
        fixed[5],
    ];
    ui.horizontal(|ui| {
        for (heading, width) in [
            "",
            "Name",
            "Value / path",
            "Type",
            "Content type",
            "Filename",
            "",
        ]
        .into_iter()
        .zip(widths)
        {
            heading_cell(ui, width, heading);
        }
    });
    for (index, row) in rows.iter_mut().enumerate() {
        let label = |part: &str| format!("Multipart row {} {part}", index + 1);
        ui.horizontal(|ui| {
            let enabled = ui.add_sized(
                [widths[0], height],
                egui::Checkbox::without_text(&mut row.is_enabled),
            );
            name(&enabled, &label("enabled"));
            changed |= enabled.changed();
            changed |=
                var_edit(ui, &mut row.name, &label("name"), widths[1], false, vars).changed();
            changed |=
                var_edit(ui, &mut row.value, &label("value"), widths[2], false, vars).changed();
            egui::ComboBox::from_id_salt(("multipart kind", index))
                .width(widths[3] - 8.0)
                .selected_text(if row.kind == FormKind::File {
                    "file"
                } else {
                    "text"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut row.kind, FormKind::Text, "text")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut row.kind, FormKind::File, "file")
                        .changed();
                });
            changed |= var_edit(
                ui,
                &mut row.content_type,
                &label("content type"),
                widths[4],
                false,
                vars,
            )
            .changed();
            changed |= var_edit(
                ui,
                &mut row.filename,
                &label("filename"),
                widths[5],
                false,
                vars,
            )
            .changed();
            let what = format!("multipart field {}", index + 1);
            if remove_button(ui, egui::Id::unique(("remove field", index)), &what, height) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        rows.remove(index);
        changed = true;
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Add field").clicked() {
            rows.push(FormItem::text("", ""));
        }
        if ui.button("Add file…").clicked() {
            *pick = Some(FileRequest::MultipartNew);
        }
        changed |= clear_all(ui, rows, confirm_clear);
    });
    changed
}

pub fn auth_form(ui: &mut Ui, auth: &mut AuthForm, vars: &mut VarUi) -> bool {
    let mut changed = false;
    let selected = AUTH_CHOICES
        .iter()
        .find(|(kind, _)| *kind == auth.kind)
        .map_or("None", |(_, label)| label);
    ui.horizontal(|ui| {
        let caption = ui.label("Type");
        let combo = egui::ComboBox::from_id_salt("auth type")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (kind, label) in AUTH_CHOICES {
                    changed |= ui.selectable_value(&mut auth.kind, kind, label).changed();
                }
            });
        combo.response.labelled_by(caption.id);
    });
    let width = (ui.available_width() - 100.0).clamp(160.0, 420.0);
    match auth.kind {
        None => {
            ui.label(tone_text(
                "No authentication is configured for this request.",
                Tone::Muted,
            ));
        }
        Some(AuthKind::Basic | AuthKind::Digest) => {
            ui.horizontal(|ui| {
                ui.add_sized([80.0, 18.0], egui::Label::new("Username"));
                changed |=
                    var_edit(ui, &mut auth.username, "Username", width, false, vars).changed();
            });
            ui.horizontal(|ui| {
                ui.add_sized([80.0, 18.0], egui::Label::new("Password"));
                changed |=
                    var_edit(ui, &mut auth.password, "Password", width, true, vars).changed();
            });
        }
        Some(AuthKind::BearerToken) => {
            ui.horizontal(|ui| {
                ui.add_sized([80.0, 18.0], egui::Label::new("Token"));
                changed |= var_edit(ui, &mut auth.token, "Token", width, true, vars).changed();
            });
        }
    }
    changed
}

fn override_text(
    ui: &mut Ui,
    field: OptionField,
    value: &mut Override<String>,
    vars: &mut VarUi,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let inherit = ui.checkbox(&mut value.inherit, "Use application default");
        name(
            &inherit,
            &format!("Use application default for {}", field.label()),
        );
        changed |= inherit.changed();
        let width = ui.available_width();
        let password = field == OptionField::Password;
        ui.add_enabled_ui(!value.inherit, |ui| {
            changed |=
                var_edit(ui, &mut value.value, field.label(), width, password, vars).changed();
        });
    });
    changed
}

fn override_bool(ui: &mut Ui, field: OptionField, value: &mut Override<bool>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let inherit = ui.checkbox(&mut value.inherit, "Use application default");
        name(
            &inherit,
            &format!("Use application default for {}", field.label()),
        );
        changed |= inherit.changed();
        let response = ui.add_enabled(
            !value.inherit,
            egui::Checkbox::new(&mut value.value, "Enabled"),
        );
        name(&response, field.label());
        changed |= response.changed();
    });
    changed
}

/// The request's settings overrides, each with a hint showing what it inherits.
pub fn options_form(
    ui: &mut Ui,
    options: &mut OptionsForm,
    defaults: &AppSettings,
    errors: &[(OptionField, &'static str)],
    vars: &mut VarUi,
) -> bool {
    let mut changed = false;
    for (title, fields) in OPTION_GROUPS {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            for &field in fields {
                ui.label(field.label());
                changed |= match field {
                    OptionField::Timeout => override_text(ui, field, &mut options.timeout, vars),
                    OptionField::DownloadLimit => {
                        override_text(ui, field, &mut options.download_limit, vars)
                    }
                    OptionField::ProxyUrl => override_text(ui, field, &mut options.proxy_url, vars),
                    OptionField::CaBundle => override_text(ui, field, &mut options.ca_bundle, vars),
                    OptionField::CertificatePath => {
                        override_text(ui, field, &mut options.certificate_path, vars)
                    }
                    OptionField::KeyFile => override_text(ui, field, &mut options.key_file, vars),
                    OptionField::Password => override_text(ui, field, &mut options.password, vars),
                    OptionField::FollowRedirects => {
                        override_bool(ui, field, &mut options.follow_redirects)
                    }
                    OptionField::AttachCookies => {
                        override_bool(ui, field, &mut options.attach_cookies)
                    }
                    OptionField::VerifySsl => override_bool(ui, field, &mut options.verify_ssl),
                };
                let hint = if options.is_inherited(field) {
                    default_hint(field, defaults)
                } else {
                    "Request override".to_owned()
                };
                ui.label(tone_text(hint, Tone::Muted).small());
                if let Some((_, message)) = errors.iter().find(|(f, _)| *f == field) {
                    ui.label(tone_text(*message, Tone::Error));
                }
            }
        });
    }
    changed
}

/// Standard methods plus a custom token, which keeps its case.
pub fn method_picker(ui: &mut Ui, method: &mut String, custom: &mut Option<String>) -> bool {
    let mut changed = false;
    let response = egui::ComboBox::from_id_salt("method")
        .width(100.0)
        .height(320.0)
        .selected_text(
            RichText::new(method.as_str())
                .monospace()
                .strong()
                .color(crate::theme::method_color(method)),
        )
        .show_ui(ui, |ui| {
            for standard in STANDARD_METHODS {
                if ui.selectable_label(method == standard, standard).clicked() && method != standard
                {
                    standard.clone_into(method);
                    changed = true;
                }
            }
            if ui.selectable_label(false, "Custom method…").clicked() {
                *custom = Some(method.clone());
            }
        });
    name(&response.response, "HTTP method");
    if let Some(mut draft) = custom.take() {
        let field = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .desired_width(90.0)
                .hint_text("METHOD"),
        );
        name(&field, "Custom HTTP method");
        let valid = is_method_token(&draft);
        if ui.add_enabled(valid, egui::Button::new("Use")).clicked()
            || (valid && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            *method = draft;
            changed = true;
        } else if !ui.small_button("×").clicked() {
            if !valid && !draft.is_empty() {
                ui.label(tone_text("ASCII token, no spaces", Tone::Error));
            }
            *custom = Some(draft);
        }
    }
    changed
}

/// A multiline body editor; JSON bodies are highlighted while small enough.
pub fn body_editor(ui: &mut Ui, text: &mut String, json: bool, vars: &mut VarUi) -> bool {
    let width = ui.available_width();
    let response = var_edit_with(
        ui,
        text,
        "Request body",
        width,
        false,
        vars,
        Some((json, "Enter request body (JSON, XML, etc.)")),
    );
    if json && !text.trim().is_empty() {
        match serde_json::from_str::<serde::de::IgnoredAny>(text) {
            Ok(_) => ui.label(tone_text("Valid JSON", Tone::Ok)),
            Err(error) => ui.label(tone_text(
                format!("Invalid JSON, line {}: {error}", error.line()),
                Tone::Error,
            )),
        };
    }
    if text.len() > HIGHLIGHT_LIMIT {
        ui.label(tone_text("Large body: highlighting is off", Tone::Muted).small());
    }
    response.changed()
}

/// Find state and options of one code viewer.
#[derive(Debug, Default)]
pub struct ViewerState {
    pub wrap: bool,
    pub find_open: bool,
    pub query: String,
    matches: Vec<std::ops::Range<usize>>,
    current: usize,
    searched: Option<(String, usize)>,
    scroll_to: Option<usize>,
    focus_find: bool,
}

impl ViewerState {
    pub fn new() -> Self {
        Self {
            wrap: true,
            ..Self::default()
        }
    }

    pub fn open_find(&mut self) {
        self.find_open = true;
        self.focus_find = true;
    }

    pub fn match_status(&self) -> String {
        if self.query.is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "No matches".to_owned()
        } else {
            format!("Match {} of {}", self.current + 1, self.matches.len())
        }
    }

    fn refresh(&mut self, text: &str) {
        let key = (self.query.clone(), text.len());
        if self.searched.as_ref() != Some(&key) {
            self.matches = viewer::find_all(text, &self.query);
            self.current = 0;
            self.searched = Some(key);
            self.scroll_to = self.matches.first().map(|m| m.start);
        }
    }

    fn step(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len();
        self.current = if forward {
            (self.current + 1) % count
        } else {
            (self.current + count - 1) % count
        };
        self.scroll_to = Some(self.matches[self.current].start);
    }
}

/// Right-aligned tools, leaving room for the caller's tabs on the same row.
/// Narrower than this, a viewer's tabs and its tools get a row each instead of sharing one.
pub const TAB_ROW_MIN: f32 = 280.0;

pub fn viewer_tools(ui: &mut Ui, state: &mut ViewerState, text: &str, copy_label: &str) {
    viewer_tools_with(ui, state, text, copy_label, None, |_| {});
}

/// [`viewer_tools`] with fold buttons for `folding`, and `more` drawn left of Wrap.
pub fn viewer_tools_with(
    ui: &mut Ui,
    state: &mut ViewerState,
    text: &str,
    copy_label: &str,
    folding: Option<Folding<'_>>,
    more: impl FnOnce(&mut Ui),
) {
    // Fold buttons are a shortcut for the markers on each line, so they are the first to go
    // when the row is too narrow for every tool.
    let room_for_all = ui.available_width() >= 6.0 * (28.0 + ui.spacing().item_spacing.x);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if icons::button(ui, Icon::Copy, copy_label, false).clicked() {
            ui.ctx().copy_text(text.to_owned());
        }
        if icons::button(ui, Icon::Search, "Find", false).clicked() {
            state.open_find();
        }
        if icons::button(ui, Icon::Wrap, "Wrap", state.wrap).clicked() {
            state.wrap = !state.wrap;
        }
        if let Some(folding) = folding.filter(|_| room_for_all) {
            if icons::button(ui, Icon::Expand, "Expand all", false).clicked() {
                folding.folded.clear();
            }
            if icons::button(ui, Icon::Collapse, "Collapse all", false).clicked() {
                *folding.folded = folding.outline.collapse_all();
            }
        }
        more(ui);
    });
}

/// Folding and the row menu of a viewer showing formatted JSON.
pub struct Folding<'a> {
    pub outline: &'a viewer::Outline,
    /// Lines whose containers are folded.
    pub folded: &'a mut std::collections::BTreeSet<usize>,
    /// Offer "Add check" and "Set variable" in the row menu (the response belongs to the
    /// open request).
    pub add_check: bool,
}

/// What a row menu asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowAction {
    /// Add a check on the value at `path`; `value` is its text, as "Copy value" gives it.
    AddCheck { path: String, value: Option<String> },
    /// Capture the value at `path` into a variable.
    Capture { path: String },
}

const GUTTER: f32 = 12.0;

/// The fold marker of `line`, or the same space without one.
fn fold_marker(ui: &mut Ui, folding: &mut Folding<'_>, line: usize, first: bool, height: f32) {
    let foldable = first
        && folding
            .outline
            .lines
            .get(line)
            .is_some_and(|l| l.close.is_some());
    if !foldable {
        ui.allocate_space(egui::vec2(GUTTER, height));
        return;
    }
    let (rect, response) = ui.allocate_exact_size(egui::vec2(GUTTER, height), egui::Sense::click());
    let open = !folding.folded.contains(&line);
    let path = &folding.outline.lines[line].path;
    name(
        &response,
        &format!("{} {path}", if open { "Fold" } else { "Unfold" }),
    );
    if response.clicked() && !folding.folded.remove(&line) {
        folding.folded.insert(line);
    }
    let color = if response.hovered() {
        palette().accent
    } else {
        palette().text_muted
    };
    let c = rect.center();
    let points = if open {
        vec![
            c + egui::vec2(-4.0, -2.0),
            c + egui::vec2(4.0, -2.0),
            c + egui::vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + egui::vec2(-2.0, -4.0),
            c + egui::vec2(3.0, 0.0),
            c + egui::vec2(-2.0, 4.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

/// Copy path and Copy value for the line under a row, and Add check when offered.
fn row_menu(
    ui: &mut Ui,
    folding: &Folding<'_>,
    text: &str,
    line: usize,
    action: &mut Option<RowAction>,
) {
    let outline = folding.outline;
    let Some(entry) = outline.lines.get(line) else {
        return;
    };
    if folding.add_check && ui.button("Add check").clicked() {
        *action = Some(RowAction::AddCheck {
            path: entry.path.clone(),
            value: outline.copy_value(text, line),
        });
        ui.close();
    }
    if folding.add_check && ui.button("Set variable").clicked() {
        *action = Some(RowAction::Capture {
            path: entry.path.clone(),
        });
        ui.close();
    }
    if ui.button("Copy path").clicked() {
        ui.ctx().copy_text(entry.path.clone());
        ui.close();
    }
    if let Some(value) = outline.copy_value(text, line)
        && ui.button("Copy value").clicked()
    {
        ui.ctx().copy_text(value);
        ui.close();
    }
}

/// A read-only viewer over bounded text: only visible rows are laid out.
pub fn code_viewer(
    ui: &mut Ui,
    id: &str,
    state: &mut ViewerState,
    text: &str,
    language: Language,
    mut folding: Option<Folding<'_>>,
) -> Option<RowAction> {
    let mut action = None;
    let area = ui.max_rect();
    if ui.rect_contains_pointer(area)
        && ui.input_mut(|i| i.consume_shortcut(&crate::shortcuts::FIND))
    {
        state.open_find();
    }
    if state.find_open {
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Find in displayed text")
                    .desired_width(220.0),
            );
            name(&field, &format!("Find in {id}"));
            if state.focus_find {
                field.request_focus();
                state.focus_find = false;
            }
            state.refresh(text);
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.find_open = false;
            }
            if (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button("Next").clicked()
            {
                state.step(true);
            }
            if ui.button("Previous").clicked() {
                state.step(false);
            }
            if ui.button("Close").clicked() {
                state.find_open = false;
            }
            ui.label(state.match_status());
        });
    }

    egui::Frame::new()
        .fill(palette().input)
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let font = FontId::monospace(12.5);
            let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, 'M')).max(1.0);
            let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
            let gutter = if folding.is_some() {
                GUTTER + ui.spacing().item_spacing.x
            } else {
                0.0
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // a positive count of columns
            let columns = state
                .wrap
                .then(|| ((ui.available_width() - 16.0 - gutter) / char_width).max(20.0) as usize);
            let mut rows = viewer::rows(text, columns);
            if let Some(folding) = &mut folding {
                if let Some(offset) = state.scroll_to {
                    unfold_around(folding, offset, text);
                }
                rows = viewer::unfolded(rows, folding.outline, folding.folded);
            }
            let target_row = state.scroll_to.take().and_then(|offset| {
                rows.iter()
                    .position(|r| r.range.end >= offset && r.range.start <= offset)
            });
            let mut scroll = egui::ScrollArea::both()
                .id_salt(id)
                .auto_shrink([false, false]);
            if let Some(row) = target_row {
                #[allow(clippy::cast_precision_loss)]
                // row indexes are far below f32 precision limits
                let offset = row as f32 * (row_height + ui.spacing().item_spacing.y);
                scroll = scroll.vertical_scroll_offset((offset - 40.0).max(0.0));
            }
            let current = state.matches.get(state.current).cloned();
            scroll.show_rows(ui, row_height, rows.len(), |ui, visible| {
                // A row with a fold marker must stay exactly as tall as a plain row: the
                // scroll area places rows by a fixed height.
                ui.spacing_mut().interact_size.y = row_height;
                for index in visible {
                    let row = &rows[index];
                    let range = &row.range;
                    let line = &text[range.clone()];
                    let mut job =
                        spans_job(line, &highlight::line(language, line), &font, f32::INFINITY);
                    let first = index == 0 || rows[index - 1].line != row.line;
                    let last = rows.get(index + 1).is_none_or(|next| next.line != row.line);
                    for found in &state.matches {
                        if found.start < range.end && found.end > range.start {
                            let start = found.start.max(range.start) - range.start;
                            let end = found.end.min(range.end) - range.start;
                            let strong = current.as_ref() == Some(found);
                            mark(
                                &mut job,
                                start..end,
                                if strong {
                                    palette().accent.gamma_multiply(0.6)
                                } else {
                                    palette().accent.gamma_multiply(0.25)
                                },
                            );
                        }
                    }
                    let label = |job| {
                        egui::Label::new(job)
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend)
                    };
                    let Some(folding) = &mut folding else {
                        ui.add(label(job));
                        continue;
                    };
                    if last
                        && folding.folded.contains(&row.line)
                        && let Some(close) = folding.outline.lines[row.line].close
                    {
                        let closing = text.split('\n').nth(close).unwrap_or_default().trim();
                        job.append(
                            &format!(" … {closing}"),
                            0.0,
                            TextFormat::simple(font.clone(), palette().text_muted),
                        );
                    }
                    ui.horizontal(|ui| {
                        fold_marker(ui, folding, row.line, first, row_height);
                        let response = ui.add(label(job));
                        response.context_menu(|ui| {
                            row_menu(ui, folding, text, row.line, &mut action);
                        });
                    });
                }
            });
        });
    action
}

/// Unfold every container that hides the text at byte `offset`, so Find can show it.
fn unfold_around(folding: &mut Folding<'_>, offset: usize, text: &str) {
    let line = text[..offset.min(text.len())].matches('\n').count();
    folding.folded.retain(|&opener| {
        folding.outline.lines[opener]
            .close
            .is_none_or(|close| !(opener < line && line <= close))
    });
}

/// Give a byte range of a laid-out job a background, splitting sections as needed.
fn mark(job: &mut LayoutJob, range: std::ops::Range<usize>, background: Color32) {
    restyle(job, range, |format| format.background = background);
}

/// Change the format of a byte range of a laid-out job, splitting sections as needed.
fn restyle(job: &mut LayoutJob, range: std::ops::Range<usize>, change: impl Fn(&mut TextFormat)) {
    use egui::text::ByteIndex;
    let mut sections = Vec::with_capacity(job.sections.len() + 2);
    for section in job.sections.drain(..) {
        let (start, end): (usize, usize) = (
            section.byte_range.start.into(),
            section.byte_range.end.into(),
        );
        let cut = |a: usize, b: usize, highlighted: bool| {
            let mut part = section.clone();
            part.byte_range = ByteIndex(a)..ByteIndex(b);
            if highlighted {
                change(&mut part.format);
            }
            part
        };
        let (lo, hi) = (range.start.clamp(start, end), range.end.clamp(start, end));
        for (a, b, highlighted) in [(start, lo, false), (lo, hi, true), (hi, end, false)] {
            if a < b {
                sections.push(cut(a, b, highlighted));
            }
        }
    }
    job.sections = sections;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_values_are_labelled_by_their_first_line_shortened() {
        assert_eq!(saved_label("42", false), "42");
        assert_eq!(saved_label("", false), "Empty value");
        assert_eq!(saved_label("secret", true), "••••••••");
        assert_eq!(saved_label("line one\nline two", false), "line one…");
        let long = "x".repeat(61);
        assert_eq!(saved_label(&long, false), format!("{}…", "x".repeat(60)));
        assert_eq!(saved_label(&"x".repeat(60), false), "x".repeat(60));
    }

    #[test]
    fn marking_splits_sections_and_keeps_the_text() {
        let font = FontId::monospace(12.0);
        let text = "\"key\": 1";
        let mut job = spans_job(text, &highlight::json(text), &font, f32::INFINITY);
        mark(&mut job, 2..7, Color32::RED);
        let range = |s: &egui::text::LayoutSection| -> std::ops::Range<usize> {
            s.byte_range.start.into()..s.byte_range.end.into()
        };
        let covered: Vec<_> = job.sections.iter().map(range).collect();
        assert_eq!(covered.first().unwrap().start, 0);
        assert_eq!(covered.last().unwrap().end, text.len());
        assert!(covered.windows(2).all(|w| w[0].end == w[1].start));
        let marked: String = job
            .sections
            .iter()
            .filter(|s| s.format.background == Color32::RED)
            .map(|s| &text[range(s)])
            .collect();
        assert_eq!(marked, "ey\": ");
    }

    #[test]
    fn viewer_search_steps_and_wraps_around() {
        let mut state = ViewerState::new();
        state.query = "ab".into();
        state.refresh("ab ab ab");
        assert_eq!(state.match_status(), "Match 1 of 3");
        state.step(false);
        assert_eq!(state.match_status(), "Match 3 of 3");
        state.step(true);
        assert_eq!(state.match_status(), "Match 1 of 3");
        state.query = "zz".into();
        state.refresh("ab ab ab");
        assert_eq!(state.match_status(), "No matches");
        state.step(true);
        state.query.clear();
        state.refresh("ab");
        assert_eq!(state.match_status(), "");
    }
}
