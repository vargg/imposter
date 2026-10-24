//! Reusable widgets. Each returns whether the user changed anything, so the caller decides
//! what an edit means (dirty state, autosave, URL sync).

use egui::text::LayoutJob;
use egui::{Color32, FontId, RichText, TextFormat, Ui};
use poster_app::core::model::{
    AuthKind, FormItem, FormKind, ParamItem, STANDARD_METHODS, is_method_token,
};
use poster_app::core::settings::AppSettings;

use crate::editor::{
    AUTH_CHOICES, AuthForm, OPTION_GROUPS, OptionField, OptionsForm, Override, default_hint,
};
use crate::highlight::{self, Spans};
use crate::icons::{self, Icon};
use crate::theme::{PALETTE, Tone, token_color};
use crate::vars;
use crate::viewer::{self, Language};
use poster_app::core::variables::Snapshot;

/// Give a widget an accessible name when it has no visible label (screen readers, UI tests).
pub fn name(response: &egui::Response, label: &str) {
    let label = label.to_owned();
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
}

pub fn tone_text(text: impl Into<String>, tone: Tone) -> RichText {
    RichText::new(text.into()).color(PALETTE.tone(tone))
}

pub fn tab(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    let text = RichText::new(label).color(if selected {
        PALETTE.accent
    } else {
        PALETTE.text_muted
    });
    let response = ui.add(egui::Button::selectable(selected, text).min_size(egui::vec2(0.0, 30.0)));
    if selected {
        let rect = response.rect;
        ui.painter().line_segment(
            [
                rect.left_bottom() + egui::vec2(8.0, -1.0),
                rect.right_bottom() + egui::vec2(-8.0, -1.0),
            ],
            egui::Stroke::new(2.0, PALETTE.accent),
        );
    }
    response
}

pub fn badge(ui: &mut Ui, text: &str, tone: Tone) -> egui::Response {
    egui::Frame::new()
        .fill(PALETTE.input)
        .stroke(egui::Stroke::new(1.0, PALETTE.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| ui.label(tone_text(text, tone).small().strong()))
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
#[derive(Debug, Clone, Copy, Default)]
struct Completion {
    selected: usize,
    /// The popup's rect last frame, so clicking it does not count as leaving the field.
    rect: Option<egui::Rect>,
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
        let defined = snapshot.is_some_and(|s| s.definitions.contains_key(&token.name));
        let color = if defined {
            PALETTE.accent
        } else {
            PALETTE.error
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

fn var_edit_with(
    ui: &mut Ui,
    text: &mut String,
    label: &str,
    width: f32,
    password: bool,
    vars: &mut VarUi,
    multiline: Option<(bool, &str)>,
) -> egui::Response {
    let id = ui.make_persistent_id(("variable field", label));
    let ctx = ui.ctx().clone();
    let completion: Option<Completion> = ctx.data(|d| d.get_temp(id));
    let hovering_popup = completion
        .and_then(|c| c.rect)
        .is_some_and(|rect| ui.rect_contains_pointer(rect));
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
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
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
    let alt_enter = response.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::Enter));
    if (response.clicked() || alt_enter)
        && let Some(token) = cursor.and_then(|c| vars::token_at(text, c))
    {
        *vars.inspect = Some(Inspection {
            name: token.name,
            masked: password,
            anchor: response.rect.left_bottom(),
        });
    }

    // Suggest names while typing an unfinished `{{name`.
    let open = (response.has_focus() || hovering_popup) && !dismiss;
    let candidates = match (open, snapshot, cursor) {
        (true, Some(snapshot), Some(cursor)) => vars::open_token(text, cursor)
            .map(|(start, query)| (start, cursor, vars::suggestions(snapshot, query))),
        _ => None,
    };
    match candidates {
        Some((start, cursor, items)) if !items.is_empty() => {
            let mut state = completion.unwrap_or_default();
            #[allow(
                clippy::cast_possible_wrap,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )] // tiny list indexes
            {
                state.selected =
                    (state.selected as i64 + step).rem_euclid(items.len() as i64) as usize;
            }
            let mut chosen = accept.then(|| items[state.selected].0.to_owned());
            let area = egui::Area::new(id.with("completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 2.0))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(width.clamp(200.0, 360.0));
                        for (index, (name, definition)) in items.iter().enumerate() {
                            let row = format!(
                                "{name} · {} · {}",
                                vars::scope_label(&definition.source),
                                definition.source.label()
                            );
                            if ui.selectable_label(index == state.selected, row).clicked() {
                                chosen = Some((*name).to_owned());
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

/// The inspector popover for an open [`Inspection`]. Returns where the user wants to go.
pub fn inspector(
    ctx: &egui::Context,
    inspection: &mut Option<Inspection>,
    snapshot: Option<&Snapshot>,
    error: Option<&str>,
) -> Option<vars::Scope> {
    let current = inspection.as_mut()?;
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut open = None;
    let reveal_id = egui::Id::unique(("inspector reveal", current.name.as_str()));
    let mut revealed: bool = ctx.data(|d| d.get_temp(reveal_id)).unwrap_or(false);
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
                    (None, None) => {
                        ui.label(tone_text("Not defined", Tone::Error));
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
                        open = target;
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
        ctx.data_mut(|d| d.remove_temp::<bool>(reveal_id));
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
fn heading_cell(ui: &mut Ui, width: f32, text: &str) {
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
            if ui
                .add_sized([24.0, height], egui::Button::new("×"))
                .on_hover_text("Remove row")
                .clicked()
            {
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
            if ui
                .add_sized([widths[6], height], egui::Button::new("×"))
                .on_hover_text("Remove field")
                .clicked()
            {
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
pub fn viewer_tools(ui: &mut Ui, state: &mut ViewerState, text: &str, copy_label: &str) {
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
    });
}

/// A read-only viewer over bounded text: only visible rows are laid out.
pub fn code_viewer(ui: &mut Ui, id: &str, state: &mut ViewerState, text: &str, language: Language) {
    let area = ui.max_rect();
    let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
    if ui.rect_contains_pointer(area) && ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
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
        .fill(PALETTE.input)
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let font = FontId::monospace(12.5);
            let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, 'M')).max(1.0);
            let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // a positive count of columns
            let columns = state
                .wrap
                .then(|| ((ui.available_width() - 16.0) / char_width).max(20.0) as usize);
            let rows = viewer::rows(text, columns);
            let target_row = state.scroll_to.take().and_then(|offset| {
                rows.iter()
                    .position(|r| r.end >= offset && r.start <= offset)
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
                for range in &rows[visible] {
                    let line = &text[range.clone()];
                    let mut job =
                        spans_job(line, &highlight::line(language, line), &font, f32::INFINITY);
                    for found in &state.matches {
                        if found.start < range.end && found.end > range.start {
                            let start = found.start.max(range.start) - range.start;
                            let end = found.end.min(range.end) - range.start;
                            let strong = current.as_ref() == Some(found);
                            mark(
                                &mut job,
                                start..end,
                                if strong {
                                    PALETTE.accent.gamma_multiply(0.6)
                                } else {
                                    PALETTE.accent.gamma_multiply(0.25)
                                },
                            );
                        }
                    }
                    ui.add(
                        egui::Label::new(job)
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            });
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
