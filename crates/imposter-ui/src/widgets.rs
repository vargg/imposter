//! Reusable widgets. Each returns whether the user changed anything, so the caller decides
//! what an edit means (dirty state, autosave, URL sync).

use egui::text::LayoutJob;
use egui::{Color32, FontId, RichText, TextFormat, Ui};
use imposter_app::core::model::{
    AuthKind, EnvColor, FormItem, FormKind, ParamItem, STANDARD_METHODS, is_method_token,
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

/// What a tab shows beside its name: "3", or a raised dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBadge {
    pub mark: Mark,
    pub tone: Tone,
    /// What it means, for screen readers: "3 in use".
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Count(usize),
    /// Checks that failed: "✗1". A red total read as that many failures.
    Failed(usize),
    /// The tab holds something.
    Dot,
}

/// Append `mark` to a tab's text. The dot is small and raised to the top of the line: a
/// muted "•" at mid-height read as a separator between tabs.
fn append_mark(job: &mut LayoutJob, mark: Mark, color: Color32) {
    let text = match mark {
        Mark::Count(total) => total.to_string(),
        Mark::Failed(failed) => format!("✗{failed}"),
        Mark::Dot => {
            job.append(
                "●",
                2.0,
                TextFormat {
                    font_id: FontId::proportional(DOT_SIZE),
                    color,
                    valign: egui::Align::TOP,
                    ..Default::default()
                },
            );
            return;
        }
    };
    job.append(
        &text,
        4.0,
        TextFormat {
            font_id: FontId::proportional(12.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
}

const DOT_SIZE: f32 = 7.0;

/// A tab's text: its name, muted or semibold accent when selected, then its badge, smaller.
fn tab_job(ui: &Ui, label: &str, selected: bool, badge: Option<&TabBadge>) -> LayoutJob {
    let (font, color) = if selected {
        (crate::theme::semibold(13.0), palette().accent)
    } else {
        (
            egui::TextStyle::Button.resolve(ui.style()),
            palette().text_muted,
        )
    };
    let mut job = LayoutJob::default();
    job.append(
        label,
        0.0,
        TextFormat {
            font_id: font,
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    if let Some(badge) = badge {
        append_mark(&mut job, badge.mark, palette().tone(badge.tone));
    }
    job
}

/// The "More" button's text: how many tabs it hides ("+2"), and a dot when one of them
/// holds something. A count, so it does not read as the tools' "⋯" beside it; without
/// one (`hidden` is `None` where it does not fit) only the dot and the chevron.
fn more_job(ui: &Ui, hidden: Option<usize>, held: Option<Tone>) -> LayoutJob {
    let mut job = LayoutJob::default();
    if let Some(hidden) = hidden {
        job.append(
            &format!("+{hidden}"),
            0.0,
            TextFormat {
                font_id: egui::TextStyle::Button.resolve(ui.style()),
                color: palette().text_muted,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
    }
    if let Some(tone) = held {
        append_mark(&mut job, Mark::Dot, palette().tone(tone));
    }
    job
}

/// The height of tabs and of menu buttons in their rows.
pub fn tab_height() -> f32 {
    crate::theme::spacing().control + 6.0
}

/// The width of `text` on one line.
pub fn text_width(ui: &Ui, text: RichText) -> f32 {
    egui::WidgetText::from(text)
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        )
        .size()
        .x
}

/// The width `text` takes on a button, padding included.
pub fn button_width(ui: &Ui, text: RichText) -> f32 {
    text_width(ui, text) + 2.0 * ui.spacing().button_padding.x
}

/// The width of a [`menu_text_button`] showing `text`.
pub fn menu_text_button_width(ui: &Ui, text: impl Into<egui::WidgetText>) -> f32 {
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    galley.size().x + 2.0 * ui.spacing().button_padding.x + CHEVRON
}

/// A tab: muted text, or semibold accent text with an underline when selected; no fill.
/// Hovering previews the underline in a neutral color; keyboard focus outlines the tab.
pub fn tab(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    tab_with(ui, selected, label, None)
}

/// A [`tab`] with a badge after its name. Its accessible name stays the label; the badge
/// is its description.
pub fn tab_with(
    ui: &mut Ui,
    selected: bool,
    label: &str,
    badge: Option<&TabBadge>,
) -> egui::Response {
    // Framed, so it keeps the button padding, but with nothing drawn: `frame(false)` drops
    // the padding too and runs the labels together.
    let response = ui.add(
        egui::Button::new(tab_job(ui, label, selected, badge))
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(0.0, tab_height())),
    );
    if let Some(badge) = badge {
        name(&response, label);
        let description = badge.description.clone();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_description(description));
    }
    let underline = if selected {
        Some(palette().accent)
    } else if response.hovered() {
        Some(palette().text_muted)
    } else {
        None
    };
    tab_marks(ui, &response, underline);
    response
}

/// A small dot in `tone` over the top right corner of `response` (a tab), drawn over it
/// rather than in its text, so nothing after it moves; screen readers hear `description`.
pub fn corner_dot(ui: &Ui, response: &egui::Response, tone: Tone, description: &str) {
    let center = egui::pos2(
        response.rect.right() - ui.spacing().button_padding.x + 3.0,
        response.rect.top() + 6.0,
    );
    ui.painter()
        .circle_filled(center, DOT_SIZE / 2.0, palette().tone(tone));
    let description = description.to_owned();
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_description(description));
}

/// A tab's underline in `color`, under its text, and the outline of keyboard focus.
fn tab_marks(ui: &Ui, response: &egui::Response, underline: Option<Color32>) {
    if let Some(color) = underline {
        // Under the text, which sits inside the button's padding.
        let rect = response
            .rect
            .shrink2(egui::vec2(ui.spacing().button_padding.x, 0.0));
        ui.painter().line_segment(
            [
                rect.left_bottom() + egui::vec2(0.0, -1.0),
                rect.right_bottom() + egui::vec2(0.0, -1.0),
            ],
            egui::Stroke::new(2.0, color),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            4.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
}

/// One tab of a [`tab_strip`].
pub struct TabSpec<'a, T> {
    pub value: T,
    pub label: &'a str,
    /// The accessible name, when the label alone is ambiguous (another tab has the same text).
    pub name: Option<&'a str>,
    pub badge: Option<TabBadge>,
}

impl<'a, T> TabSpec<'a, T> {
    pub fn new(value: T, label: &'a str) -> Self {
        Self {
            value,
            label,
            name: None,
            badge: None,
        }
    }

    #[must_use]
    pub fn badge(mut self, badge: Option<TabBadge>) -> Self {
        self.badge = badge;
        self
    }

    #[must_use]
    pub fn named(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }
}

/// Which tabs of `widths` show in a row `available` wide, in order, when `more` is the width
/// of the "More" button: as many as fit from the start, and always the selected one.
fn shown_tabs(
    widths: &[f32],
    selected: Option<usize>,
    available: f32,
    more: f32,
    gap: f32,
) -> Vec<usize> {
    let total: f32 = widths.iter().map(|w| w + gap).sum::<f32>() - gap;
    if total <= available {
        return (0..widths.len()).collect();
    }
    let budget = available - more;
    let mut shown = Vec::new();
    let mut used = 0.0;
    for (i, width) in widths.iter().enumerate() {
        if used + width + gap > budget {
            break;
        }
        shown.push(i);
        used += width + gap;
    }
    if let Some(selected) = selected.filter(|s| !shown.contains(s)) {
        while used + widths[selected] + gap > budget {
            let Some(last) = shown.pop() else { break };
            used -= widths[last] + gap;
        }
        shown.push(selected);
    }
    shown
}

const CHEVRON: f32 = 12.0;

/// A frameless button showing `text` and a chevron, named `label`, that opens a menu.
pub fn menu_text_button(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    label: &str,
) -> egui::Response {
    // Plain text takes the painter's muted color below; a job keeps its own colors.
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = ui.spacing().button_padding.x;
    let size = egui::vec2(galley.size().x + 2.0 * pad + CHEVRON, tab_height());
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let color = palette().text_muted;
        let text = egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0);
        painter.galley(text, galley, color);
        let c = egui::pos2(rect.right() - pad - CHEVRON / 2.0 + 2.0, rect.center().y);
        painter.add(egui::Shape::convex_polygon(
            vec![
                c + egui::vec2(-4.0, -2.0),
                c + egui::vec2(4.0, -2.0),
                c + egui::vec2(0.0, 2.5),
            ],
            color,
            egui::Stroke::NONE,
        ));
    }
    tab_marks(
        ui,
        &response,
        response.hovered().then_some(palette().text_muted),
    );
    response
}

/// One row of tabs in a horizontal `ui`. Tabs that do not fit, leaving `reserve` points for
/// what follows on the row, go into a "More" menu; the selected tab always stays in the row.
/// `more` names the menu button for screen readers. Returns whether the user chose a tab.
pub fn tab_strip<T: Copy + PartialEq>(
    ui: &mut Ui,
    tabs: &[TabSpec<'_, T>],
    selected: &mut T,
    reserve: f32,
    more: &str,
) -> bool {
    ui.scope(|ui| {
        // Frameless tabs need less padding than buttons: their labels stay 14 points apart.
        ui.spacing_mut().button_padding.x = TAB_PADDING;
        ui.spacing_mut().item_spacing.x = 2.0;
        tab_row(ui, tabs, selected, reserve, more)
    })
    .inner
}

const TAB_PADDING: f32 = 6.0;

fn tab_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    tabs: &[TabSpec<'_, T>],
    selected: &mut T,
    reserve: f32,
    more: &str,
) -> bool {
    let current = tabs.iter().position(|tab| tab.value == *selected);
    let widths: Vec<f32> = tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let job = tab_job(ui, tab.label, Some(i) == current, tab.badge.as_ref());
            ui.fonts_mut(|fonts| fonts.layout_job(job)).size().x
                + 2.0 * ui.spacing().button_padding.x
        })
        .collect();
    // Measured at its widest: a one-digit count (rows have at most 9 tabs) and the dot.
    let wide = menu_text_button_width(ui, more_job(ui, Some(9), Some(Tone::Ok)));
    let gap = ui.spacing().item_spacing.x;
    let available = ui.available_width() - reserve;
    let mut shown = shown_tabs(&widths, current, available, wide, gap);
    let used: f32 = shown.iter().map(|&i| widths[i] + gap).sum();
    // When even the selected tab leaves no room for the count, More drops it.
    let counted = shown.len() == tabs.len() || used + wide <= available;
    if !counted {
        let narrow = menu_text_button_width(ui, more_job(ui, None, Some(Tone::Ok)));
        shown = shown_tabs(&widths, current, available, narrow, gap);
    }
    let mut chosen = None;
    for &i in &shown {
        let spec = &tabs[i];
        let response = tab_with(ui, Some(i) == current, spec.label, spec.badge.as_ref());
        if let Some(label) = spec.name {
            name(&response, label);
        }
        if response.clicked() {
            chosen = Some(spec.value);
        }
    }
    if shown.len() < tabs.len() {
        // Hidden tabs that hold something: the button says so, and names them.
        let hidden: Vec<&TabSpec<'_, T>> = tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| !shown.contains(i))
            .map(|(_, spec)| spec)
            .collect();
        let held: Vec<String> = hidden
            .iter()
            .filter_map(|spec| {
                let badge = spec.badge.as_ref()?;
                Some(format!("{}: {}", spec.label, badge.description))
            })
            .collect();
        let text = more_job(
            ui,
            counted.then_some(hidden.len()),
            (!held.is_empty()).then_some(Tone::Ok),
        );
        // The hover text is also what screen readers hear as its description.
        let hint = if held.is_empty() {
            "More tabs".to_owned()
        } else {
            format!("More tabs: {}", held.join(", "))
        };
        let button = menu_text_button(ui, text, more).on_hover_text(hint);
        egui::Popup::menu(&button).show(|ui| {
            for (i, spec) in tabs.iter().enumerate() {
                if shown.contains(&i) {
                    continue;
                }
                let item = ui.button(tab_job(ui, spec.label, false, spec.badge.as_ref()));
                name(&item, spec.name.unwrap_or(spec.label));
                if item.clicked() {
                    chosen = Some(spec.value);
                    ui.close();
                }
            }
        });
    }
    match chosen {
        Some(value) if value != *selected => {
            *selected = value;
            true
        }
        _ => false,
    }
}

/// A choice in a menu or list: filled when selected or hovered, never outlined, so a selected
/// row looks the same everywhere (the theme outlines controls, and egui would outline it too).
pub fn option<'a>(ui: &mut Ui, selected: bool, atoms: impl egui::IntoAtoms<'a>) -> egui::Response {
    ui.add(egui::Button::selectable(selected, atoms).stroke(egui::Stroke::NONE))
}

/// A color's name as people say it.
pub const fn env_color_name(color: EnvColor) -> &'static str {
    match color {
        EnvColor::Red => "Red",
        EnvColor::Yellow => "Yellow",
        EnvColor::Green => "Green",
        EnvColor::Blue => "Blue",
        EnvColor::Purple => "Purple",
    }
}

/// An environment's name after a dot in its color; the name alone when it has none.
pub fn env_text(name: &str, color: Option<EnvColor>, font: FontId, text: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    if let Some(color) = color {
        job.append(
            "● ",
            0.0,
            TextFormat::simple(font.clone(), crate::theme::env_color(color)),
        );
    }
    job.append(name, 0.0, TextFormat::simple(font, text));
    job
}

/// An environment in a list: like [`env_text`], with the dot's room kept when it has no
/// color, so every name in the list starts at the same place.
pub fn env_option_text(
    name: &str,
    color: Option<EnvColor>,
    font: FontId,
    text: Color32,
) -> LayoutJob {
    if color.is_some() {
        return env_text(name, color, font, text);
    }
    let mut job = LayoutJob::default();
    job.append(
        "● ",
        0.0,
        TextFormat::simple(font.clone(), Color32::TRANSPARENT),
    );
    job.append(name, 0.0, TextFormat::simple(font, text));
    job
}

/// A dot in an environment's color, painted rather than written, so the label beside it
/// stays the name alone for screen readers.
pub fn env_dot(ui: &mut Ui, color: EnvColor) {
    let height = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, height), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.5, crate::theme::env_color(color));
}

/// The environment a send uses, as a tag: its name on its color's tint with a dot before it,
/// or muted on a neutral fill without a color. Words carry it, so hue is never the only cue.
pub fn env_pill(ui: &mut Ui, name: &str, color: Option<EnvColor>) -> egui::Response {
    const MAX_WIDTH: f32 = 160.0;
    let (text, fill) = match color {
        Some(color) => {
            let shade = crate::theme::env_color(color);
            (shade, shade.gamma_multiply(PILL_TINT))
        }
        None => (palette().text_muted, palette().hover),
    };
    let font = crate::theme::semibold(12.0);
    let dot = if color.is_some() { 8.0 + 4.0 } else { 0.0 };
    let name_width = ui
        .fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), text))
        .size()
        .x;
    // As wide as its contents, up to a limit past which the name is cut.
    let width = (dot + name_width).min(MAX_WIDTH);
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let response = egui::Frame::new()
        .fill(fill)
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(PILL_PADDING, 1))
        .show(ui, |ui| {
            // Left to right even inside a right-to-left row, so the dot leads.
            ui.allocate_ui_with_layout(
                egui::vec2(width, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(color) = color {
                        env_dot(ui, color);
                    }
                    ui.add(egui::Label::new(RichText::new(name).color(text).font(font)).truncate())
                },
            )
            .inner
        })
        .inner;
    response.on_hover_text(format!("Requests are sent with the {name} environment"))
}

/// Swatches for an environment's color: none, then each color, one selected. Returns
/// whether the user chose one.
pub fn env_swatches(ui: &mut Ui, color: &mut Option<EnvColor>) -> bool {
    let mut chosen = false;
    let side = crate::theme::spacing().control;
    ui.horizontal(|ui| {
        for choice in std::iter::once(None).chain(EnvColor::ALL.map(Some)) {
            let label = choice.map_or("No color", env_color_name);
            let selected = *color == choice;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label)
            });
            let palette = palette();
            let painter = ui.painter();
            let center = rect.center();
            let radius = side * 0.32;
            if let Some(c) = choice {
                painter.circle_filled(center, radius, crate::theme::env_color(c));
            } else {
                // "No color": an empty circle, struck through.
                let stroke = egui::Stroke::new(1.5, palette.field_border);
                painter.circle_stroke(center, radius, stroke);
                let d = radius * std::f32::consts::FRAC_1_SQRT_2;
                painter.line_segment(
                    [center + egui::vec2(-d, d), center + egui::vec2(d, -d)],
                    stroke,
                );
            }
            if selected {
                painter.circle_stroke(center, side * 0.47, egui::Stroke::new(2.0, palette.text));
            } else if response.hovered() {
                painter.circle_stroke(
                    center,
                    side * 0.47,
                    egui::Stroke::new(1.0, palette.text_muted),
                );
            }
            if response.clicked() {
                *color = choice;
                chosen = true;
            }
            response.on_hover_text(label);
        }
    });
    chosen
}

/// The grip in the middle of a split's `handle`: a short bar across the split, in the
/// control-edge color (≥3:1, so it can be found), the text color while `active`.
pub fn split_grip(ui: &Ui, handle: egui::Rect, vertical_split: bool, active: bool) {
    let color = if active {
        palette().text_muted
    } else {
        palette().field_border
    };
    let size = if vertical_split {
        egui::vec2(3.0, GRIP_LENGTH)
    } else {
        egui::vec2(GRIP_LENGTH, 3.0)
    };
    ui.painter().rect_filled(
        egui::Rect::from_center_size(handle.center(), size),
        2.0,
        color,
    );
}

/// The length of a split's grip.
pub const GRIP_LENGTH: f32 = 32.0;

/// Shortcuts drawn as key caps, with "or" between alternatives.
pub fn key_caps(ui: &mut Ui, keys: &[egui::KeyboardShortcut]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = KEY_CAPS_SPACING;
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ui.label(tone_text("or", Tone::Muted).small());
            }
            let parts = crate::shortcuts::keys(ui.ctx(), key);
            let (rect, response) =
                ui.allocate_exact_size(caps_size(ui, &parts), egui::Sense::hover());
            paint_caps(ui, rect, &parts);
            let text = parts.join(" ");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
        }
    });
}

const KEY_CAPS_SPACING: f32 = 6.0;

/// The width [`key_caps`] takes for `keys`.
pub fn key_caps_width(ui: &Ui, keys: &[egui::KeyboardShortcut]) -> f32 {
    let or = text_width(ui, RichText::new("or").small());
    let mut width = 0.0;
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            width += 2.0 * KEY_CAPS_SPACING + or;
        }
        width += caps_size(ui, &crate::shortcuts::keys(ui.ctx(), key)).x;
    }
    width
}

/// The space between two keys of a chord and inside a key.
const CAP_GAP: f32 = 3.0;
const CAP_PADDING: f32 = 5.0;

fn cap_galleys(ui: &Ui, keys: &[&str]) -> Vec<std::sync::Arc<egui::Galley>> {
    keys.iter()
        .map(|key| {
            ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    (*key).to_owned(),
                    egui::TextStyle::Monospace.resolve(ui.style()),
                    palette().text,
                )
            })
        })
        .collect()
}

/// The size of [`paint_caps`] for `keys`.
pub fn caps_size(ui: &Ui, keys: &[&str]) -> egui::Vec2 {
    let galleys = cap_galleys(ui, keys);
    let width = galleys
        .iter()
        .map(|galley| galley.size().x.max(galley.size().y) + 2.0 * CAP_PADDING + CAP_GAP)
        .sum::<f32>()
        - CAP_GAP;
    let height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max) + 4.0;
    egui::vec2(width.max(0.0), height)
}

/// One key cap per key of a chord, from `rect`'s left and centred on it, so a chord reads
/// as keys pressed together and looks the same in the palette and the shortcut sheet.
pub fn paint_caps(ui: &Ui, rect: egui::Rect, keys: &[&str]) {
    let painter = ui.painter();
    let height = caps_size(ui, keys).y;
    let mut left = rect.left();
    for galley in cap_galleys(ui, keys) {
        // At least square, so "/" is not a sliver.
        let width = galley.size().x.max(galley.size().y) + 2.0 * CAP_PADDING;
        let cap = egui::Rect::from_min_size(
            egui::pos2(left, rect.center().y - height / 2.0),
            egui::vec2(width, height),
        );
        painter.rect(
            cap,
            4.0,
            palette().input,
            egui::Stroke::new(1.0, palette().field_border),
            egui::StrokeKind::Inside,
        );
        let at = cap.center() - galley.size() / 2.0;
        painter.galley(at, galley, palette().text);
        left = cap.right() + CAP_GAP;
    }
}

/// The marker beside a selected row: a bar on its left edge.
pub fn selection_bar(ui: &Ui, rect: egui::Rect) {
    let bar = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(0.0, 3.0),
        egui::vec2(2.0, rect.height() - 6.0),
    );
    ui.painter().rect_filled(bar, 1.0, palette().selected_bar);
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
/// in the error color on a red tint, both underlined. The tint tells them apart without
/// relying on hue: the two colors are equally bright. Only colors change, never the text.
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
            if !defined {
                format.background = error_tint();
            }
        });
    }
    job
}

/// The horizontal padding inside a [`pill`].
pub const PILL_PADDING: i8 = 7;
/// How strongly a pill is tinted with its tone.
pub const PILL_TINT: f32 = 0.12;

/// A small rounded label tinted with its tone, for a status: "200 OK". It is at most
/// `max_width` wide; a longer text is cut, with all of it on hover.
pub fn pill(ui: &mut Ui, text: &str, tone: Tone, max_width: f32) -> egui::Response {
    let color = palette().tone(tone);
    let inner = (max_width - 2.0 * f32::from(PILL_PADDING)).max(24.0);
    egui::Frame::new()
        .fill(color.gamma_multiply(PILL_TINT))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(PILL_PADDING, 1))
        .show(ui, |ui| {
            ui.set_max_width(inner);
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .color(color)
                        .font(crate::theme::semibold(13.0)),
                )
                .truncate(),
            )
        })
        .inner
}

/// How strongly [`error_tint`] is tinted.
pub const ERROR_TINT: f32 = 0.12;

/// A faint red background: unknown `{{variables}}`, failed checks.
pub fn error_tint() -> Color32 {
    palette().error.gamma_multiply(ERROR_TINT)
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
        // Fields with variables hold data (URLs, header values, paths): set in monospace.
        None => (egui::TextStyle::Monospace.resolve(ui.style()), false),
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
    // A body fills what its pane shows, keeping a line for the footer under it.
    let fill = (ui.clip_rect().bottom() - ui.cursor().top() - BODY_FOOTER).max(MIN_BODY_HEIGHT);
    let mut edit = match multiline {
        Some((_, hint)) => egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(4)
            .min_size(egui::vec2(0.0, fill))
            .hint_text(hint),
        None => egui::TextEdit::singleline(text),
    }
    .id(id)
    .desired_width(width)
    .password(password);
    if single_line {
        // Centred in a field taller than its text, as the method and buttons beside it are.
        edit = edit.align(egui::Align2::LEFT_CENTER);
    }
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
        // Under the token itself, so the inspector points at what it explains.
        let start = text[..token.range.start].chars().count();
        let at = output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start))
            .left_bottom()
            + output.galley_pos.to_vec2();
        let x = at.x.clamp(response.rect.left(), response.rect.right());
        *vars.inspect = Some(Inspection {
            name: token.name,
            masked: password,
            anchor: egui::pos2(x, response.rect.bottom()),
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
                            if option(ui, index == state.selected, row).clicked() {
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
        .fixed_pos(current.anchor + egui::vec2(-INSPECTOR_POINTER_INSET, INSPECTOR_POINTER))
        .show(ctx, |ui| {
            // A control edge (≥3:1), so the frame and its pointer can be seen on the bar.
            let edge = egui::Stroke::new(1.0, palette().field_border);
            egui::Frame::popup(ui.style()).stroke(edge).show(ui, |ui| {
                ui.set_min_width(320.0);
                ui.set_max_width(480.0);
                ui.horizontal(|ui| {
                    ui.heading(current.name.as_str());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_button = ui
                            .add(egui::Button::new("×").frame_when_inactive(false))
                            .on_hover_text("Close (Esc)");
                        name(&close_button, "Close");
                        if close_button.clicked() {
                            close = true;
                        }
                    });
                });
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
                                let row = option(ui, chosen, RichText::new(&text).monospace());
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
                    let value = definitions.and_then(|d| d.first()).filter(|_| error.is_none());
                    if let Some(value) = value {
                        // Copying is explicit, so a masked value is copied as it is.
                        let copy = ui.button("Copy value").on_hover_text(
                            "Copy the value a send would use",
                        );
                        if copy.clicked() {
                            ui.ctx().copy_text(value.value.clone());
                        }
                    }
                });
            });
        });
    // A small pointer from the frame up to the token.
    let fill = ctx.global_style().visuals.window_fill;
    let stroke = egui::Stroke::new(1.0, palette().field_border);
    let tip = current.anchor + egui::vec2(0.0, 1.0);
    let half = INSPECTOR_POINTER;
    let painter = ctx.layer_painter(area.response.layer_id);
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            tip + egui::vec2(half, half + 1.0),
            tip + egui::vec2(-half, half + 1.0),
        ],
        fill,
        egui::Stroke::NONE,
    ));
    painter.line_segment([tip + egui::vec2(-half, half), tip], stroke);
    painter.line_segment([tip, tip + egui::vec2(half, half)], stroke);
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

/// The height of the inspector's pointer, and how far right of the frame's edge it sits.
const INSPECTOR_POINTER: f32 = 6.0;
const INSPECTOR_POINTER_INSET: f32 = 16.0;

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
            // With something to save the icon is in the text color, apart from disabled.
            if status.enabled {
                icons::ready_button(ui, Icon::Save, action, &hint)
            } else {
                icons::button_with_hint(ui, Icon::Save, action, &hint, false)
            }
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
    two_click(ui, id, what, height, None)
}

/// [`remove_button`] with words ("Delete route") instead of ×, in red text, for removing a
/// whole thing rather than a row.
pub fn delete_button(ui: &mut Ui, id: egui::Id, what: &str, text: &str) -> bool {
    let height = ui.spacing().interact_size.y;
    two_click(ui, id, what, height, Some(text))
}

fn two_click(ui: &mut Ui, id: egui::Id, what: &str, height: f32, text: Option<&str>) -> bool {
    let (now, pointer) = ui.input(|i| (i.time, i.pointer.latest_pos()));
    // When it was armed, and where the pointer was then.
    let armed_at: Option<(f64, Option<egui::Pos2>)> = ui.data(|d| d.get_temp(id));
    let armed = armed_at.is_some_and(|(at, _)| now - at < REMOVE_ARMED_SECONDS);
    let response = match text {
        None => {
            let button = if armed {
                egui::Button::new(RichText::new("×").strong().color(palette().error))
                    .stroke(egui::Stroke::new(1.0, palette().error))
            } else {
                egui::Button::new("×")
            };
            // 24 points in every density: tables reserve exactly that for it.
            ui.scope(|ui| {
                ui.spacing_mut().button_padding.x = 4.0;
                ui.add_sized([24.0, height], button)
            })
            .inner
        }
        Some(text) => {
            let shown = if armed { "Click again" } else { text };
            let button = egui::Button::new(RichText::new(shown).color(palette().error));
            let button = if armed {
                button.stroke(egui::Stroke::new(1.0, palette().error))
            } else {
                button
            };
            ui.add(button)
        }
    };
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
    /// A path field on the Settings screen.
    Setting(crate::settings_screen::PathField),
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

/// The auth form's label column, so fields line up whatever the label.
const AUTH_LABELS: f32 = 76.0;

pub fn auth_form(ui: &mut Ui, auth: &mut AuthForm, vars: &mut VarUi) -> bool {
    let mut changed = false;
    let selected = AUTH_CHOICES
        .iter()
        .find(|(kind, _)| *kind == auth.kind)
        .map_or("None", |(_, label)| label);
    // Fields take the rest of the pane; inside a grid cell the cell is all a field would see.
    let width = (ui.available_width() - AUTH_LABELS - 12.0).max(64.0);
    egui::Grid::new("auth form")
        .num_columns(2)
        .min_col_width(AUTH_LABELS)
        .spacing([12.0, 10.0])
        .show(ui, |ui| {
            let caption = ui.label("Type");
            let combo = egui::ComboBox::from_id_salt("auth type")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (kind, label) in AUTH_CHOICES {
                        changed |= ui.selectable_value(&mut auth.kind, kind, label).changed();
                    }
                });
            combo.response.labelled_by(caption.id);
            ui.end_row();
            match auth.kind {
                None => {
                    ui.label("");
                    ui.label(tone_text(
                        "No authentication is configured for this request.",
                        Tone::Muted,
                    ));
                    ui.end_row();
                }
                Some(AuthKind::Basic | AuthKind::Digest) => {
                    ui.label("Username");
                    changed |=
                        var_edit(ui, &mut auth.username, "Username", width, false, vars).changed();
                    ui.end_row();
                    ui.label("Password");
                    changed |= secret_edit(ui, &mut auth.password, "Password", width, vars);
                    ui.end_row();
                }
                Some(AuthKind::BearerToken) => {
                    ui.label("Token");
                    changed |= secret_edit(ui, &mut auth.token, "Token", width, vars);
                    ui.end_row();
                }
            }
        });
    changed
}

/// A secret's field, masked until its eye button shows it. Shown or not is remembered per
/// field for the session, never stored.
fn secret_edit(ui: &mut Ui, text: &mut String, label: &str, width: f32, vars: &mut VarUi) -> bool {
    let id = ui.make_persistent_id(("shown secret", label));
    let mut shown: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    let changed = ui
        .horizontal(|ui| {
            let eye = icons::size() + ui.spacing().item_spacing.x;
            let changed =
                var_edit(ui, text, label, (width - eye).max(40.0), !shown, vars).changed();
            let (icon, action) = if shown {
                (Icon::EyeOff, format!("Hide {}", label.to_lowercase()))
            } else {
                (Icon::Eye, format!("Show {}", label.to_lowercase()))
            };
            if icons::button(ui, icon, &action, false).clicked() {
                shown = !shown;
            }
            changed
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(id, shown));
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
    // While a custom method is typed, its field takes the picker's place, so the row
    // beside it (the URL) keeps its room.
    if let Some(mut draft) = custom.take() {
        // A field of its own, framed even where the picker is not (the request bar).
        ui.scope(|ui| {
            ui.set_style(ui.ctx().global_style());
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(4.0);
            let field = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .desired_width(72.0)
                    .hint_text("METHOD"),
            );
            name(&field, "Custom HTTP method");
            let valid = is_method_token(&draft);
            if ui.add_enabled(valid, egui::Button::new("Use")).clicked()
                || (valid && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                *method = draft;
                changed = true;
            } else if !ui
                .small_button("×")
                .on_hover_text("Keep the method")
                .clicked()
            {
                if !valid && !draft.is_empty() {
                    ui.label(tone_text("No spaces allowed", Tone::Error))
                        .on_hover_text(
                            "A method is one word of letters, digits and !#$%&'*+-.^_`|~",
                        );
                }
                *custom = Some(draft);
            }
            ui.add_space(4.0);
        });
        return changed;
    }
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
                if option(ui, method == standard, standard).clicked() && method != standard {
                    standard.clone_into(method);
                    changed = true;
                }
            }
            if option(ui, false, "Custom method…").clicked() {
                *custom = Some(method.clone());
            }
        });
    name(&response.response, "HTTP method");
    changed
}

/// A multiline body editor; JSON bodies are highlighted while small enough.
/// Room under a body editor for its footer ("Valid JSON" or the parse error).
const BODY_FOOTER: f32 = 24.0;
/// A body editor is never shorter than about four lines, even in a small pane.
const MIN_BODY_HEIGHT: f32 = 96.0;

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

/// A viewer's tools at the right of a row: wrap, find and copy.
pub fn viewer_tools(ui: &mut Ui, state: &mut ViewerState, text: &str, copy_label: &str) {
    viewer_tools_with(ui, state, text, copy_label, None, None);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Copy,
    Find,
    Filter,
    Wrap,
    Collapse,
    Expand,
}

/// The tools a viewer offers, right to left as drawn: the first are the last to move into
/// the "More tools" menu. Fold buttons go first, since every line has its own marker.
fn viewer_tool_list(filter: bool, folding: bool) -> Vec<Tool> {
    let mut tools = vec![Tool::Copy, Tool::Find];
    if filter {
        tools.push(Tool::Filter);
    }
    tools.push(Tool::Wrap);
    if folding {
        tools.extend([Tool::Collapse, Tool::Expand]);
    }
    tools
}

/// How many of `count` tools show as icons in `available` points; the rest need a menu
/// button, which takes a place too.
fn tools_shown(count: usize, available: f32, slot: f32, gap: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a small count ≥ 0
    let fit = ((available + gap) / slot).floor().max(0.0) as usize;
    if fit >= count {
        count
    } else {
        fit.saturating_sub(1)
    }
}

/// [`viewer_tools`] with fold buttons for `folding` and, for `filter` (whether the filter is
/// open, and where to report a click), the JSONPath filter. Tools that do not fit the row
/// move into a "More tools" menu.
pub fn viewer_tools_with(
    ui: &mut Ui,
    state: &mut ViewerState,
    text: &str,
    copy_label: &str,
    folding: Option<Folding<'_>>,
    filter: Option<(bool, &mut bool)>,
) {
    let filter_open = filter.as_ref().is_some_and(|(open, _)| *open);
    let tools = viewer_tool_list(filter.is_some(), folding.is_some());
    let gap = ui.spacing().item_spacing.x;
    let shown = tools_shown(tools.len(), ui.available_width(), icons::size() + gap, gap);
    let describe = |tool: Tool| match tool {
        Tool::Copy => (Icon::Copy, copy_label, false),
        Tool::Find => (Icon::Search, "Find", false),
        Tool::Filter => (Icon::Filter, "Filter with JSONPath", filter_open),
        Tool::Wrap => (Icon::Wrap, "Wrap", state.wrap),
        Tool::Collapse => (Icon::Collapse, "Collapse all", false),
        Tool::Expand => (Icon::Expand, "Expand all", false),
    };
    let mut chosen = None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for &tool in &tools[..shown] {
            let (icon, label, selected) = describe(tool);
            if icons::button(ui, icon, label, selected).clicked() {
                chosen = Some(tool);
            }
        }
        if shown < tools.len() {
            icons::menu(ui, Icon::More, "More tools", |ui| {
                for &tool in &tools[shown..] {
                    let (_, label, selected) = describe(tool);
                    // Wrap and the filter are on or off: a check box, since a filled row
                    // read as the one under the pointer.
                    let clicked = if matches!(tool, Tool::Wrap | Tool::Filter) {
                        let mut on = selected;
                        ui.checkbox(&mut on, label).clicked()
                    } else {
                        option(ui, false, label).clicked()
                    };
                    if clicked {
                        chosen = Some(tool);
                        ui.close();
                    }
                }
            });
        }
    });
    match chosen {
        Some(Tool::Copy) => ui.ctx().copy_text(text.to_owned()),
        Some(Tool::Find) => state.open_find(),
        Some(Tool::Filter) => {
            if let Some((_, clicked)) = filter {
                *clicked = true;
            }
        }
        Some(Tool::Wrap) => state.wrap = !state.wrap,
        Some(Tool::Collapse) => {
            if let Some(folding) = folding {
                *folding.folded = folding.outline.collapse_all();
            }
        }
        Some(Tool::Expand) => {
            if let Some(folding) = folding {
                folding.folded.clear();
            }
        }
        None => {}
    }
}

/// Headers as a table: names muted, values selectable and wrapping. The row under the
/// pointer offers a copy of `name: value`; `id` names its scroll area.
pub fn headers_table(ui: &mut Ui, id: &str, headers: &[(String, String)]) {
    egui::ScrollArea::vertical()
        .id_salt((id, "headers table"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let gap = ui.spacing().item_spacing.x;
            let width = ui.available_width();
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let widest = headers
                .iter()
                .map(|(name, _)| {
                    ui.fonts_mut(|fonts| {
                        fonts
                            .layout_no_wrap(name.clone(), font.clone(), Color32::PLACEHOLDER)
                            .size()
                            .x
                    })
                })
                .fold(0.0, f32::max);
            let names = widest.min(MAX_NAME_COLUMN);
            let copy = icons::size();
            let values = width - names - copy - 2.0 * gap;
            // Too narrow for two columns, each value goes under its name: narrow columns
            // broke "content-length" and "application/json" mid-word.
            let stacked = values < MIN_VALUE_COLUMN;
            let text = if stacked { width - copy - gap } else { values };
            let pointer = ui.input(|i| i.pointer.hover_pos());
            let name_label = |name: &str| {
                egui::Label::new(RichText::new(name).monospace().color(palette().text_muted)).wrap()
            };
            let value_label = |value: &str| {
                egui::Label::new(RichText::new(value).monospace())
                    .selectable(true)
                    .wrap()
            };
            for (name, value) in headers {
                ui.horizontal_top(|ui| {
                    let top = ui.cursor().top();
                    if stacked {
                        ui.allocate_ui(egui::vec2(text, 0.0), |ui| {
                            ui.set_width(text);
                            ui.vertical(|ui| {
                                ui.add(name_label(name));
                                ui.add(value_label(value));
                            });
                        });
                    } else {
                        ui.allocate_ui(egui::vec2(names, 0.0), |ui| {
                            ui.set_width(names);
                            ui.add(name_label(name));
                        });
                        ui.allocate_ui(egui::vec2(text, 0.0), |ui| {
                            ui.set_width(text);
                            ui.add(value_label(value));
                        });
                    }
                    let bottom = ui.min_rect().bottom();
                    let hovered = pointer.is_some_and(|p| {
                        ui.max_rect().x_range().contains(p.x) && (top..=bottom).contains(&p.y)
                    });
                    if hovered {
                        let label = format!("Copy {name}");
                        if icons::button(ui, Icon::Copy, &label, false).clicked() {
                            ui.ctx().copy_text(format!("{name}: {value}"));
                        }
                    } else {
                        ui.allocate_space(egui::vec2(copy, copy));
                    }
                });
                if stacked {
                    ui.add_space(4.0);
                }
            }
        });
}

/// The widest the names' column gets; longer names wrap.
const MAX_NAME_COLUMN: f32 = 240.0;
/// Below this, values go under their names instead of beside them.
const MIN_VALUE_COLUMN: f32 = 160.0;

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
    // What the actions act on.
    ui.label(
        RichText::new(&entry.path)
            .monospace()
            .color(palette().text_muted),
    );
    ui.separator();
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

/// The line under the pointer is highlighted and offers its row menu from a "⋯" at the
/// right of the viewer, so the actions are found without right-clicking. Both stay while
/// the menu is open.
fn row_actions(
    ui: &mut Ui,
    background: egui::layers::ShapeIdx,
    folding: &Folding<'_>,
    text: &str,
    line: usize,
    action: &mut Option<RowAction>,
) {
    let Some(entry) = folding.outline.lines.get(line) else {
        return;
    };
    let clip = ui.clip_rect();
    let row = egui::Rect::from_x_y_ranges(clip.x_range(), ui.min_rect().y_range());
    let id = ui.make_persistent_id(("row actions", line));
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pointer| row.contains(pointer));
    if !(hovered || open) {
        return;
    }
    ui.painter().set(
        background,
        egui::Shape::rect_filled(row, 2.0, palette().hover),
    );
    let spot = egui::Rect::from_min_max(
        egui::pos2(row.right() - ROW_ACTIONS_WIDTH, row.top()),
        row.max,
    );
    let button = ui.interact(spot, id, egui::Sense::click());
    // An unwrapped line can still reach the edge: the "⋯" sits on the row's own fill.
    ui.painter().rect_filled(spot, 2.0, palette().hover);
    let label = format!("Row actions {}", entry.path);
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let color = if button.hovered() || open {
        palette().text
    } else {
        palette().text_muted
    };
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter()
            .circle_filled(spot.center() + egui::vec2(dx, 0.0), 1.5, color);
    }
    // Under the row, so the line acted on stays in view.
    egui::Popup::menu(&button)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| row_menu(ui, folding, text, line, action));
}

const ROW_ACTIONS_WIDTH: f32 = 24.0;

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
            // Fold markers on the left; on the right the row actions' "⋯", which wrapped
            // lines must not run under.
            let gutter = if folding.is_some() {
                GUTTER + ui.spacing().item_spacing.x + ROW_ACTIONS_WIDTH
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
                        // Filled in below once the row's extent is known.
                        let background = ui.painter().add(egui::Shape::Noop);
                        fold_marker(ui, folding, row.line, first, row_height);
                        let response = ui.add(label(job));
                        response.context_menu(|ui| {
                            row_menu(ui, folding, text, row.line, &mut action);
                        });
                        if first {
                            row_actions(ui, background, folding, text, row.line, &mut action);
                        }
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
    fn an_unknown_variable_is_tinted_and_a_known_one_is_not() {
        let text = "{{$uuid}} {{missing}}";
        let job = variable_job(
            text,
            &vec![(0..text.len(), highlight::Token::Plain)],
            &FontId::monospace(12.0),
            f32::INFINITY,
            None,
        );
        let background = |at: usize| {
            job.sections
                .iter()
                .find(|section| section.byte_range.contains(&egui::text::ByteIndex(at)))
                .map(|section| section.format.background)
        };
        assert_eq!(
            background(2),
            Some(Color32::TRANSPARENT),
            "generated, so known"
        );
        assert_eq!(background(12), Some(error_tint()));
    }

    #[test]
    fn tabs_that_do_not_fit_leave_the_row_but_the_selected_one_stays() {
        let widths = [40.0, 40.0, 40.0, 40.0];
        // 4 × 40 + 3 gaps of 4 = 172.
        assert_eq!(shown_tabs(&widths, Some(0), 172.0, 30.0, 4.0), [0, 1, 2, 3]);
        // 100 wide, 30 for More: two tabs (88 with their gaps) fit before it.
        assert_eq!(shown_tabs(&widths, Some(0), 120.0, 30.0, 4.0), [0, 1]);
        // The selected tab takes the place of the last one that fitted.
        assert_eq!(shown_tabs(&widths, Some(3), 120.0, 30.0, 4.0), [0, 3]);
        // Even when nothing else fits.
        assert_eq!(shown_tabs(&widths, Some(2), 50.0, 30.0, 4.0), [2]);
        assert_eq!(
            shown_tabs(&widths, None, 50.0, 30.0, 4.0),
            Vec::<usize>::new()
        );
    }

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
