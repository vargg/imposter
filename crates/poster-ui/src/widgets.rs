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
use crate::theme::{PALETTE, Tone, token_color};
use crate::viewer::{self, Language};

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
            let field = ui.add_sized(
                [name_width, height],
                egui::TextEdit::singleline(&mut row.name).hint_text("name"),
            );
            name(&field, &format!("{id} row {} name", index + 1));
            changed |= field.changed();
            let value = ui.add_sized(
                [value_width, height],
                egui::TextEdit::singleline(&mut row.value).hint_text("value"),
            );
            name(&value, &format!("{id} row {} value", index + 1));
            changed |= value.changed();
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
            for (text, width, part) in [
                (&mut row.name, widths[1], "name"),
                (&mut row.value, widths[2], "value"),
            ] {
                let field = ui.add_sized([width, height], egui::TextEdit::singleline(text));
                name(&field, &label(part));
                changed |= field.changed();
            }
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
            for (text, width, part) in [
                (&mut row.content_type, widths[4], "content type"),
                (&mut row.filename, widths[5], "filename"),
            ] {
                let field = ui.add_sized([width, height], egui::TextEdit::singleline(text));
                name(&field, &label(part));
                changed |= field.changed();
            }
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

pub fn auth_form(ui: &mut Ui, auth: &mut AuthForm) -> bool {
    let mut changed = false;
    let selected = AUTH_CHOICES
        .iter()
        .find(|(kind, _)| *kind == auth.kind)
        .map_or("None", |(_, label)| label);
    ui.horizontal(|ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt("auth type")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (kind, label) in AUTH_CHOICES {
                    changed |= ui.selectable_value(&mut auth.kind, kind, label).changed();
                }
            });
    });
    match auth.kind {
        None => {
            ui.label(tone_text(
                "No authentication is configured for this request.",
                Tone::Muted,
            ));
        }
        Some(AuthKind::Basic | AuthKind::Digest) => {
            egui::Grid::new("auth fields")
                .num_columns(2)
                .show(ui, |ui| {
                    let label = ui.label("Username");
                    changed |= ui
                        .text_edit_singleline(&mut auth.username)
                        .labelled_by(label.id)
                        .changed();
                    ui.end_row();
                    let label = ui.label("Password");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut auth.password).password(true))
                        .labelled_by(label.id)
                        .changed();
                    ui.end_row();
                });
        }
        Some(AuthKind::BearerToken) => {
            ui.horizontal(|ui| {
                let label = ui.label("Token");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut auth.token)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(label.id)
                    .changed();
            });
        }
    }
    changed
}

fn override_text(ui: &mut Ui, field: OptionField, value: &mut Override<String>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let inherit = ui.checkbox(&mut value.inherit, "Use application default");
        name(
            &inherit,
            &format!("Use application default for {}", field.label()),
        );
        changed |= inherit.changed();
        let edit = egui::TextEdit::singleline(&mut value.value)
            .password(field == OptionField::Password)
            .desired_width(f32::INFINITY);
        let response = ui.add_enabled(!value.inherit, edit);
        name(&response, field.label());
        changed |= response.changed();
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
) -> bool {
    let mut changed = false;
    for (title, fields) in OPTION_GROUPS {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            for &field in fields {
                ui.label(field.label());
                changed |= match field {
                    OptionField::Timeout => override_text(ui, field, &mut options.timeout),
                    OptionField::DownloadLimit => {
                        override_text(ui, field, &mut options.download_limit)
                    }
                    OptionField::ProxyUrl => override_text(ui, field, &mut options.proxy_url),
                    OptionField::CaBundle => override_text(ui, field, &mut options.ca_bundle),
                    OptionField::CertificatePath => {
                        override_text(ui, field, &mut options.certificate_path)
                    }
                    OptionField::KeyFile => override_text(ui, field, &mut options.key_file),
                    OptionField::Password => override_text(ui, field, &mut options.password),
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
        .selected_text(method.as_str())
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
pub fn body_editor(ui: &mut Ui, text: &mut String, json: bool) -> bool {
    let font = FontId::monospace(13.0);
    let highlighted = json && text.len() <= HIGHLIGHT_LIMIT;
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
        let content = buffer.as_str();
        let spans = if highlighted {
            highlight::json(content)
        } else {
            vec![(0..content.len(), highlight::Token::Plain)]
        };
        ui.fonts_mut(|fonts| fonts.layout_job(spans_job(content, &spans, &font, wrap)))
    };
    let response = ui.add(
        egui::TextEdit::multiline(text)
            .code_editor()
            .desired_width(f32::INFINITY)
            .desired_rows(10)
            .hint_text("Enter request body (JSON, XML, etc.)")
            .layouter(&mut layouter),
    );
    name(&response, "Request body");
    if json && !text.trim().is_empty() {
        match serde_json::from_str::<serde::de::IgnoredAny>(text) {
            Ok(_) => ui.label(tone_text("Valid JSON", Tone::Ok)),
            Err(error) => ui.label(tone_text(
                format!("Invalid JSON, line {}: {error}", error.line()),
                Tone::Error,
            )),
        };
    }
    if !highlighted && json && !text.is_empty() {
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

/// A read-only viewer over bounded text: only visible rows are laid out.
pub fn code_viewer(
    ui: &mut Ui,
    id: &str,
    state: &mut ViewerState,
    text: &str,
    language: Language,
    copy_label: &str,
) {
    ui.horizontal(|ui| {
        if ui.button(copy_label).clicked() {
            ui.ctx().copy_text(text.to_owned());
        }
        if ui.button("Find").clicked() {
            state.open_find();
        }
        ui.toggle_value(&mut state.wrap, "Wrap");
    });
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
        #[allow(clippy::cast_precision_loss)] // row indexes are far below f32 precision limits
        let offset = row as f32 * (row_height + ui.spacing().item_spacing.y);
        scroll = scroll.vertical_scroll_offset((offset - 40.0).max(0.0));
    }
    let current = state.matches.get(state.current).cloned();
    scroll.show_rows(ui, row_height, rows.len(), |ui, visible| {
        for range in &rows[visible] {
            let line = &text[range.clone()];
            let mut job = spans_job(line, &highlight::line(language, line), &font, f32::INFINITY);
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
}

/// Give a byte range of a laid-out job a background, splitting sections as needed.
fn mark(job: &mut LayoutJob, range: std::ops::Range<usize>, background: Color32) {
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
                part.format.background = background;
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
