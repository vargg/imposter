//! The themes: a dark and a light palette of named roles. Every color the UI uses comes from
//! [`palette()`], the palette of the theme the current frame draws in.

use std::cell::Cell;

use egui::{Color32, CornerRadius, Stroke, Theme, ThemePreference, Visuals};
use imposter_app::core::model::EnvColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Window and panel background.
    pub background: Color32,
    /// Raised areas: side panel, popups, tables.
    pub surface: Color32,
    /// Text inputs and code areas.
    pub input: Color32,
    pub hover: Color32,
    pub selected: Color32,
    /// Lines between areas; quiet on purpose.
    pub border: Color32,
    /// The edge of a field or control: at least 3:1 against what it sits on (WCAG 1.4.11).
    pub field_border: Color32,
    /// The marker beside a selected row.
    pub selected_bar: Color32,
    /// Backdrop separating a modal from the window behind it.
    pub modal_scrim: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    /// Disabled controls; exempt from contrast rules but still legible.
    pub text_disabled: Color32,
    pub accent: Color32,
    /// Text drawn on an `accent` fill (the Send button).
    pub on_accent: Color32,
    pub ok: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub syntax: Syntax,
    /// Environment colors, readable as text on every background, so a name can wear one.
    pub environment: EnvColors,
}

/// One shade per [`EnvColor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvColors {
    pub red: Color32,
    pub yellow: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub purple: Color32,
}

/// Highlighting colors, readable on `input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syntax {
    pub key: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub tag: Color32,
    pub comment: Color32,
}

pub const DARK: Palette = Palette {
    background: Color32::from_rgb(0x11, 0x13, 0x18),
    surface: Color32::from_rgb(0x1a, 0x1d, 0x24),
    input: Color32::from_rgb(0x13, 0x16, 0x1c),
    hover: Color32::from_rgb(0x28, 0x2d, 0x37),
    selected: Color32::from_rgb(0x3b, 0x2a, 0x24),
    border: Color32::from_rgb(0x30, 0x35, 0x40),
    field_border: Color32::from_rgb(0x5f, 0x68, 0x78),
    selected_bar: Color32::from_rgb(0xff, 0x6c, 0x37),
    modal_scrim: Color32::from_black_alpha(120),
    text: Color32::from_rgb(0xea, 0xed, 0xf3),
    text_muted: Color32::from_rgb(0xa3, 0xad, 0xbf),
    text_disabled: Color32::from_rgb(0x7a, 0x83, 0x94),
    accent: Color32::from_rgb(0xff, 0x6c, 0x37),
    on_accent: Color32::from_rgb(0x1a, 0x1a, 0x1a),
    ok: Color32::from_rgb(0x5f, 0xd0, 0x8a),
    warning: Color32::from_rgb(0xf0, 0xc0, 0x5a),
    error: Color32::from_rgb(0xff, 0x6b, 0x6b),
    syntax: Syntax {
        key: Color32::from_rgb(0x9c, 0xdc, 0xfe),
        string: Color32::from_rgb(0xce, 0x91, 0x78),
        number: Color32::from_rgb(0xb5, 0xce, 0xa8),
        keyword: Color32::from_rgb(0x56, 0x9c, 0xd6),
        tag: Color32::from_rgb(0x4e, 0xc9, 0xb0),
        comment: Color32::from_rgb(0x8a, 0x99, 0x8a),
    },
    environment: EnvColors {
        red: Color32::from_rgb(0xff, 0x6b, 0x6b),
        yellow: Color32::from_rgb(0xf0, 0xc0, 0x5a),
        green: Color32::from_rgb(0x5f, 0xd0, 0x8a),
        blue: Color32::from_rgb(0x6c, 0xb6, 0xff),
        purple: Color32::from_rgb(0xc4, 0x9b, 0xff),
    },
};

/// The accent darkens to stay readable as text on white; the tones darken likewise.
pub const LIGHT: Palette = Palette {
    background: Color32::from_rgb(0xf4, 0xf5, 0xf8),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    input: Color32::from_rgb(0xfb, 0xfb, 0xfc),
    hover: Color32::from_rgb(0xec, 0xee, 0xf3),
    selected: Color32::from_rgb(0xfc, 0xeb, 0xe3),
    border: Color32::from_rgb(0xdf, 0xe2, 0xe8),
    field_border: Color32::from_rgb(0x7d, 0x85, 0x95),
    selected_bar: Color32::from_rgb(0xb8, 0x3b, 0x0a),
    modal_scrim: Color32::from_black_alpha(132),
    text: Color32::from_rgb(0x1a, 0x1e, 0x26),
    text_muted: Color32::from_rgb(0x59, 0x62, 0x73),
    text_disabled: Color32::from_rgb(0x8a, 0x91, 0x9d),
    accent: Color32::from_rgb(0xb8, 0x3b, 0x0a),
    on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
    ok: Color32::from_rgb(0x15, 0x70, 0x2f),
    warning: Color32::from_rgb(0x87, 0x5a, 0x00),
    // As bright as the accent, and ≥4.5:1 on its own tint after rendering rounds the blend
    // (#c62828 measured 4.47:1 there).
    error: Color32::from_rgb(0xbf, 0x24, 0x24),
    syntax: Syntax {
        key: Color32::from_rgb(0x0b, 0x57, 0xa4),
        // Strings are green, away from the accent that marks `{{variables}}` inside them.
        string: Color32::from_rgb(0x1a, 0x6b, 0x2e),
        number: Color32::from_rgb(0x9b, 0x1f, 0x6a),
        keyword: Color32::from_rgb(0x6f, 0x3f, 0xb5),
        tag: Color32::from_rgb(0x0f, 0x6b, 0x5c),
        comment: Color32::from_rgb(0x5f, 0x6b, 0x6d),
    },
    environment: EnvColors {
        red: Color32::from_rgb(0xbf, 0x24, 0x24),
        yellow: Color32::from_rgb(0x7a, 0x5c, 0x00),
        green: Color32::from_rgb(0x15, 0x70, 0x2f),
        blue: Color32::from_rgb(0x0b, 0x57, 0xa4),
        purple: Color32::from_rgb(0x6f, 0x3f, 0xb5),
    },
};

thread_local! {
    /// egui draws a frame on one thread; each frame starts by setting this from its theme.
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&DARK) };
    /// Set by [`apply`] from the density.
    static SPACING: Cell<Spacing> = const { Cell::new(Density::Compact.spacing()) };
}

/// How tightly lists and controls are packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// More rows on screen: the default for a tool used all day.
    #[default]
    Compact,
    Comfortable,
}

/// The sizes a density sets, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    /// A list row: tree, History, palette.
    pub row: f32,
    /// A control: button, field, tab.
    pub control: f32,
    /// Between stacked items.
    pub gap: f32,
    /// Inside a control, left and right of its text.
    pub pad: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Density {
    pub const ALL: [(Self, &'static str); 2] = [
        (Self::Compact, "Compact"),
        (Self::Comfortable, "Comfortable"),
    ];

    pub const fn spacing(self) -> Spacing {
        let compact = Spacing {
            row: 26.0,
            control: 24.0,
            gap: 4.0,
            pad: 8.0,
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 24.0,
        };
        match self {
            Self::Compact => compact,
            Self::Comfortable => Spacing {
                row: 32.0,
                control: 28.0,
                gap: compact.sm,
                pad: compact.md,
                ..compact
            },
        }
    }
}

/// The current density's sizes.
pub fn spacing() -> Spacing {
    SPACING.with(Cell::get)
}

/// The palette of the theme the current frame draws in.
pub fn palette() -> &'static Palette {
    CURRENT.with(Cell::get)
}

/// The palette for `theme`.
pub const fn of(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Use the palette of the theme `ctx` draws in; call at the start of every frame.
pub fn begin_frame(ctx: &egui::Context) {
    CURRENT.with(|current| current.set(of(ctx.theme())));
}

/// The user's theme: stored in UI state, applied at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    /// Follow the operating system.
    System,
}

impl ThemeChoice {
    pub const ALL: [(Self, &'static str); 3] = [
        (Self::Dark, "Dark"),
        (Self::Light, "Light"),
        (Self::System, "System"),
    ];

    const fn preference(self) -> ThemePreference {
        match self {
            Self::Dark => ThemePreference::Dark,
            Self::Light => ThemePreference::Light,
            Self::System => ThemePreference::System,
        }
    }
}

/// How an environment's color is drawn in the current theme.
pub fn env_color(color: EnvColor) -> Color32 {
    let shades = palette().environment;
    match color {
        EnvColor::Red => shades.red,
        EnvColor::Yellow => shades.yellow,
        EnvColor::Green => shades.green,
        EnvColor::Blue => shades.blue,
        EnvColor::Purple => shades.purple,
    }
}

pub fn method_color(method: &str) -> Color32 {
    let palette = palette();
    match method {
        "GET" => palette.ok,
        "POST" => palette.warning,
        "DELETE" => palette.error,
        "PUT" | "PATCH" => palette.syntax.key,
        _ => palette.text_muted,
    }
}

/// Syntax colors for the viewer and editors, readable on the input background.
pub fn token_color(token: crate::highlight::Token) -> Color32 {
    use crate::highlight::Token;
    let palette = palette();
    match token {
        Token::Plain => palette.text,
        Token::Key | Token::Attribute => palette.syntax.key,
        Token::String => palette.syntax.string,
        Token::Number => palette.syntax.number,
        Token::Keyword => palette.syntax.keyword,
        Token::Punctuation => palette.text_muted,
        Token::Tag => palette.syntax.tag,
        Token::Comment => palette.syntax.comment,
    }
}

/// Meaning-bearing states, so widgets never pick raw colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Muted,
    Ok,
    Warning,
    Error,
}

impl Palette {
    pub const fn tone(&self, tone: Tone) -> Color32 {
        match tone {
            Tone::Normal => self.text,
            Tone::Muted => self.text_muted,
            Tone::Ok => self.ok,
            Tone::Warning => self.warning,
            Tone::Error => self.error,
        }
    }
}

pub fn visuals(palette: &Palette, theme: Theme) -> Visuals {
    let mut visuals = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.faint_bg_color = palette.surface;
    visuals.extreme_bg_color = palette.input;
    visuals.text_edit_bg_color = Some(palette.input);
    visuals.code_bg_color = palette.input;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.error;
    visuals.selection.bg_fill = palette.selected;
    // The selection is the accent: selected items' text and a focused field's ring.
    visuals.selection.stroke = Stroke::new(1.5, palette.accent);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(8);

    let radius = CornerRadius::same(6);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
        widget.fg_stroke = Stroke::new(1.0, palette.text);
    }
    visuals.widgets.noninteractive.bg_fill = palette.background;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.weak_bg_fill = palette.input;
    visuals.widgets.inactive.bg_fill = palette.input;
    // The edge of every control a user operates: fields and buttons (WCAG 1.4.11).
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.field_border);
    visuals.widgets.hovered.weak_bg_fill = palette.hover;
    visuals.widgets.hovered.bg_fill = palette.hover;
    // Hover is neutral: orange is kept for what is primary, selected or focused.
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.text_muted);
    visuals.widgets.active.weak_bg_fill = palette.hover;
    visuals.widgets.active.bg_fill = palette.hover;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.text_muted);
    visuals.widgets.open.weak_bg_fill = palette.hover;
    visuals.widgets.open.bg_fill = palette.hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, palette.border);
    visuals
}

/// The family of the semibold weight, for titles and the active tab.
pub const SEMIBOLD: &str = "semibold";

/// Isolated because Inter also maps private-use codepoints used by Phosphor.
pub const ICONS: &str = "icons";

/// A semibold font of `size`.
pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
}

/// Shared type sizes in points; existing screen titles migrate with their screens.
pub const PAGE_TITLE: f32 = 18.0;
pub const SECTION: f32 = 14.0;
pub const BODY: f32 = 13.0;
pub const META: f32 = 12.0;

pub fn page_title(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).font(semibold(PAGE_TITLE))
}

pub fn section(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).font(semibold(SECTION))
}

pub fn body(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).size(BODY)
}

pub fn meta(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).size(META)
}

/// `Inter` for the interface, `JetBrains Mono` for data and Phosphor Regular for icons
/// (`assets/fonts`). Text keeps egui's fallbacks; icons use only Phosphor to avoid
/// collisions with Inter's private-use glyphs.
pub fn fonts() -> egui::FontDefinitions {
    use egui::{FontData, FontFamily};
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    for (name, data) in [
        (
            "Inter",
            &include_bytes!("../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "Inter-SemiBold",
            &include_bytes!("../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
        (
            "JetBrainsMono",
            &include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")[..],
        ),
        (
            "Phosphor",
            &include_bytes!("../assets/fonts/Phosphor.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(data)));
    }
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    fonts
        .families
        .insert(FontFamily::Name(ICONS.into()), vec!["Phosphor".to_owned()]);
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());
    let mut semibold = vec!["Inter-SemiBold".to_owned(), "Inter".to_owned()];
    semibold.extend(fallbacks);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    fonts
}

/// Install both themes and choose `choice`. Each frame then draws in one of them
/// ([`begin_frame`]).
pub fn apply(ctx: &egui::Context, choice: ThemeChoice, density: Density) {
    let sizes = density.spacing();
    SPACING.with(|current| current.set(sizes));
    for theme in [Theme::Dark, Theme::Light] {
        ctx.set_visuals_of(theme, visuals(of(theme), theme));
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(sizes.sm, sizes.gap);
            // 13-point text is about 16 points tall; the rest is padding.
            style.spacing.button_padding = egui::vec2(sizes.pad, (sizes.control - 16.0) / 2.0);
            style.spacing.interact_size.y = sizes.control;
            style.spacing.window_margin = egui::Margin::same(16);
            // Legacy headings migrate to the explicit title and section helpers per screen.
            for (text_style, font) in [
                (egui::TextStyle::Small, egui::FontId::proportional(META)),
                (egui::TextStyle::Body, egui::FontId::proportional(BODY)),
                (egui::TextStyle::Button, egui::FontId::proportional(BODY)),
                (egui::TextStyle::Monospace, egui::FontId::monospace(12.5)),
                (egui::TextStyle::Heading, semibold(15.0)),
            ] {
                style.text_styles.insert(text_style, font);
            }
        });
    }
    ctx.options_mut(|options| options.theme_preference = choice.preference());
    begin_frame(ctx);
}

/// WCAG 2 relative luminance of an sRGB color.
pub fn luminance(color: Color32) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// WCAG 2 contrast ratio, from 1 (none) to 21 (black on white).
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (light, dark) = {
        let (x, y) = (luminance(a), luminance(b));
        if x > y { (x, y) } else { (y, x) }
    };
    (light + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG AA for normal-size text.
    const AA: f64 = 4.5;
    /// WCAG 1.4.11 for the edges of controls.
    const NON_TEXT: f64 = 3.0;

    const BOTH: [(&str, &Palette); 2] = [("dark", &DARK), ("light", &LIGHT)];

    #[test]
    fn toggle_edges_and_knobs_contrast_in_both_states() {
        for (name, p) in BOTH {
            for background in [p.background, p.surface] {
                assert!(
                    contrast(p.accent, background) >= NON_TEXT,
                    "{name} on track"
                );
                assert!(
                    contrast(p.field_border, background) >= NON_TEXT,
                    "{name} off edge"
                );
            }
            assert!(
                contrast(p.on_accent, p.accent) >= NON_TEXT,
                "{name} on knob and focus"
            );
            assert!(contrast(p.text, p.input) >= NON_TEXT, "{name} off knob");
            assert!(contrast(p.accent, p.input) >= NON_TEXT, "{name} off focus");
        }
    }

    #[test]
    fn icon_toggle_glyph_and_underline_contrast_on_neutral_fills() {
        for (name, p) in BOTH {
            for background in [p.background, p.surface, p.input, p.hover] {
                assert!(contrast(p.accent, background) >= NON_TEXT, "{name} glyph");
                assert!(
                    contrast(p.selected_bar, background) >= NON_TEXT,
                    "{name} underline"
                );
            }
        }
    }

    #[test]
    fn controls_kit_pairings_are_readable_in_both_palettes() {
        for (name, p) in BOTH {
            assert!(
                contrast(p.accent, p.selected) >= AA,
                "{name} selected segment"
            );
            assert!(contrast(p.text, p.selected) >= AA, "{name} selected URL");
            assert!(
                contrast(p.error, p.selected) >= AA,
                "{name} selected text in an invalid mock field"
            );
            assert!(contrast(p.on_accent, p.accent) >= AA, "{name} check glyph");
            assert!(contrast(p.warning, p.input) >= AA, "{name} warning badge");
            assert!(contrast(p.error, p.input) >= AA, "{name} danger label");
            assert!(
                contrast(p.on_accent, p.error) >= AA,
                "{name} filled danger label"
            );
            assert!(
                contrast(p.error, p.surface) >= NON_TEXT,
                "{name} filled danger edge"
            );
            assert!(contrast(p.text_muted, p.input) >= AA, "{name} caret");
            let visuals = visuals(
                p,
                if name == "dark" {
                    Theme::Dark
                } else {
                    Theme::Light
                },
            );
            assert_eq!(visuals.widgets.inactive.bg_fill, p.input);
            assert_eq!(visuals.widgets.active.bg_fill, p.hover);
            assert_eq!(
                visuals.widgets.inactive.corner_radius,
                CornerRadius::same(6)
            );
        }
    }

    #[test]
    fn spacing_uses_one_grid_and_keeps_density_heights() {
        for (density, row, control) in [
            (Density::Compact, 26.0, 24.0),
            (Density::Comfortable, 32.0, 28.0),
        ] {
            let s = density.spacing();
            assert_eq!((s.row, s.control), (row, control));
            assert_eq!([s.xs, s.sm, s.md, s.lg, s.xl], [4.0, 8.0, 12.0, 16.0, 24.0]);
            assert_eq!(s.gap % s.xs, 0.0);
            assert_eq!(s.pad % s.xs, 0.0);
        }
    }

    #[test]
    fn contrast_formula_matches_known_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn every_text_tone_is_readable_on_every_background() {
        for (name, palette) in BOTH {
            for background in [
                palette.background,
                palette.surface,
                palette.input,
                palette.hover,
                palette.selected,
            ] {
                for tone in [
                    Tone::Normal,
                    Tone::Muted,
                    Tone::Ok,
                    Tone::Warning,
                    Tone::Error,
                ] {
                    let ratio = contrast(palette.tone(tone), background);
                    assert!(ratio >= AA, "{name} {tone:?} on {background:?}: {ratio:.2}");
                }
                let accent = contrast(palette.accent, background);
                assert!(accent >= AA, "{name} accent on {background:?}: {accent:.2}");
            }
        }
    }

    #[test]
    fn field_edges_and_the_selection_marker_stand_out() {
        for (name, palette) in BOTH {
            for background in [palette.input, palette.surface, palette.background] {
                let ratio = contrast(palette.field_border, background);
                assert!(
                    ratio >= NON_TEXT,
                    "{name} field edge on {background:?}: {ratio:.2}"
                );
            }
            for background in [palette.selected, palette.surface] {
                let ratio = contrast(palette.selected_bar, background);
                assert!(ratio >= NON_TEXT, "{name} selection marker: {ratio:.2}");
            }
        }
    }

    #[test]
    fn syntax_and_method_colors_are_readable_in_both_themes() {
        use crate::highlight::Token;
        for (name, palette) in BOTH {
            CURRENT.with(|current| current.set(palette));
            for token in [
                Token::Plain,
                Token::Key,
                Token::String,
                Token::Number,
                Token::Keyword,
                Token::Punctuation,
                Token::Tag,
                Token::Attribute,
                Token::Comment,
            ] {
                let ratio = contrast(token_color(token), palette.input);
                assert!(ratio >= AA, "{name} {token:?}: {ratio:.2}");
            }
            for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] {
                for background in [
                    palette.background,
                    palette.surface,
                    palette.hover,
                    palette.selected,
                ] {
                    let ratio = contrast(method_color(method), background);
                    assert!(ratio >= AA, "{name} {method} on {background:?}: {ratio:.2}");
                }
            }
        }
        CURRENT.with(|current| current.set(&DARK));
    }

    #[test]
    fn environment_colors_are_readable_on_every_background() {
        for (name, palette) in BOTH {
            CURRENT.with(|current| current.set(palette));
            for color in EnvColor::ALL {
                for background in [
                    palette.background,
                    palette.surface,
                    palette.input,
                    palette.hover,
                    palette.selected,
                ] {
                    let ratio = contrast(env_color(color), background);
                    assert!(
                        ratio >= AA,
                        "{name} {color:?} on {background:?}: {ratio:.2}"
                    );
                }
            }
        }
        CURRENT.with(|current| current.set(&DARK));
    }

    /// Variables are marked in the accent on top of syntax colors, so no syntax color may
    /// look like it. Distance in RGB; the dark string (salmon) is the closest at about 89.
    #[test]
    fn variables_stand_out_from_every_syntax_color() {
        let distance = |a: Color32, b: Color32| {
            let d = |x: u8, y: u8| (f32::from(x) - f32::from(y)).powi(2);
            (d(a.r(), b.r()) + d(a.g(), b.g()) + d(a.b(), b.b())).sqrt()
        };
        for (name, palette) in BOTH {
            let s = &palette.syntax;
            for (role, color) in [
                ("key", s.key),
                ("string", s.string),
                ("number", s.number),
                ("keyword", s.keyword),
                ("tag", s.tag),
                ("comment", s.comment),
            ] {
                let apart = distance(palette.accent, color);
                assert!(apart >= 80.0, "{name} {role}: {apart:.0} from the accent");
            }
        }
    }

    /// `color` at `amount` over `under`, as a tinted fill looks.
    /// `color` with every channel moved by `levels`.
    fn nudged(color: Color32, levels: i16) -> Color32 {
        let channel = |value: u8| u8::try_from((i16::from(value) + levels).clamp(0, 255)).unwrap();
        Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
    }

    fn tinted(color: Color32, amount: f32, under: Color32) -> Color32 {
        let mix = |top: u8, bottom: u8| {
            let blended = f32::from(bottom) * (1.0 - amount) + f32::from(top) * amount;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // between two channel values, so within 0..=255
            let channel = blended.round() as u8;
            channel
        };
        Color32::from_rgb(
            mix(color.r(), under.r()),
            mix(color.g(), under.g()),
            mix(color.b(), under.b()),
        )
    }

    /// A pill is its tone's text on the tone at `PILL_TINT` over the pane's surface.
    #[test]
    fn pills_are_readable_in_both_themes() {
        for (name, palette) in BOTH {
            for tone in [Tone::Ok, Tone::Warning, Tone::Error] {
                let color = palette.tone(tone);
                let fill = tinted(color, crate::widgets::PILL_TINT, palette.surface);
                let ratio = contrast(color, fill);
                assert!(ratio >= AA, "{name} {tone:?} pill: {ratio:.2}");
            }
        }
    }

    /// A colored environment's name on its own tint (the picker), over the background and
    /// the surface, rounded a level either way as rendering does.
    #[test]
    fn colored_environment_names_are_readable_in_both_themes() {
        for (name, palette) in BOTH {
            CURRENT.with(|current| current.set(palette));
            for color in EnvColor::ALL {
                let shade = env_color(color);
                for under in [palette.background, palette.surface] {
                    let fill = tinted(shade, crate::widgets::PILL_TINT, under);
                    for fill in [fill, nudged(fill, -1), nudged(fill, 1)] {
                        let ratio = contrast(shade, fill);
                        assert!(ratio >= AA, "{name} {color:?} pill: {ratio:.2}");
                    }
                }
            }
            // Without a color: muted text on the hover fill.
            let ratio = contrast(palette.text_muted, palette.hover);
            assert!(ratio >= AA, "{name} uncolored pill: {ratio:.2}");
        }
        CURRENT.with(|current| current.set(&DARK));
    }

    /// Unknown variables (red on the tint, in fields) and failed checks (red and plain text
    /// on the tint, in a pane) stay readable.
    #[test]
    fn text_on_the_error_tint_is_readable_in_both_themes() {
        for (name, palette) in BOTH {
            for under in [palette.input, palette.surface] {
                let fill = tinted(palette.error, crate::widgets::ERROR_TINT, under);
                // Rendering rounds a blend by a level either way: a computed 4.52:1 was
                // measured at 4.47:1 in a screenshot.
                for fill in [fill, nudged(fill, -1), nudged(fill, 1)] {
                    for text in [palette.error, palette.text] {
                        let ratio = contrast(text, fill);
                        assert!(ratio >= AA, "{name} {text:?} on {fill:?}: {ratio:.2}");
                    }
                }
            }
        }
    }

    #[test]
    fn text_on_the_send_button_is_readable_in_both_states() {
        for (name, p) in BOTH {
            assert!(contrast(p.on_accent, p.accent) >= AA, "{name} Send label");
            assert!(contrast(p.error, p.input) >= AA, "{name} Cancel/Stop label");
            for background in [p.input, p.background, p.surface] {
                for edge in [p.error, p.accent, p.text_muted] {
                    assert!(
                        contrast(edge, background) >= NON_TEXT,
                        "{name} action edge {edge:?} on {background:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn light_selected_rows_and_modal_backdrop_remain_distinct() {
        let p = &LIGHT;
        for color in [p.text, p.text_muted] {
            assert!(
                contrast(color, p.selected) >= AA,
                "selected row label and glyph"
            );
        }
        for background in [p.selected, p.background, p.surface] {
            assert!(
                contrast(p.selected_bar, background) >= NON_TEXT,
                "selection marker or underline"
            );
        }
        for background in [p.background, p.surface, p.input] {
            let dimmed = background.blend(p.modal_scrim);
            assert!(
                contrast(background, dimmed) >= NON_TEXT,
                "light backdrop on {background:?}"
            );
            assert!(
                contrast(p.surface, dimmed) >= NON_TEXT,
                "modal surface against the backdrop"
            );
        }
    }

    #[test]
    fn apply_installs_both_themes_and_the_choice() {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeChoice::Dark, Density::Compact);
        assert_eq!(
            ctx.style_of(Theme::Dark).visuals.panel_fill,
            DARK.background
        );
        assert_eq!(
            ctx.style_of(Theme::Light).visuals.panel_fill,
            LIGHT.background
        );
        assert_eq!(ctx.theme(), Theme::Dark);
        assert_eq!(palette(), &DARK);

        assert_eq!(
            ctx.style_of(Theme::Dark)
                .visuals
                .widgets
                .inactive
                .bg_stroke
                .color,
            DARK.field_border,
            "controls get the readable edge"
        );

        let dark = ctx.style_of(Theme::Dark).visuals.clone();
        assert_ne!(
            dark.widgets.hovered.bg_stroke.color, DARK.accent,
            "hover is neutral"
        );
        assert_ne!(dark.widgets.active.bg_stroke.color, DARK.accent);
        assert_eq!(
            dark.selection.stroke.color, DARK.accent,
            "selection and focus"
        );

        apply(&ctx, ThemeChoice::Light, Density::Compact);
        assert_eq!(ctx.theme(), Theme::Light);
        assert_eq!(palette(), &LIGHT, "the next frame draws in light");
        assert_eq!(
            ctx.style_of(Theme::Light).visuals.override_text_color,
            Some(LIGHT.text)
        );

        apply(&ctx, ThemeChoice::System, Density::Comfortable);
        assert_eq!(spacing(), Density::Comfortable.spacing());
        assert_eq!(ctx.style_of(Theme::Dark).spacing.interact_size.y, 28.0);
        assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System);
        CURRENT.with(|current| current.set(&DARK));
    }

    /// Every non-ASCII character inside a string literal of the sources whose text the UI
    /// shows: its own, the domain's (operators, errors) and the services'.
    #[allow(clippy::disallowed_methods)] // a test reading sources, not the UI thread
    fn interface_characters() -> std::collections::BTreeSet<char> {
        let mut found = std::collections::BTreeSet::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dirs = ["src", "../imposter-core/src", "../imposter-app/src"];
        for entry in dirs
            .iter()
            .flat_map(|dir| std::fs::read_dir(root.join(dir)).unwrap())
        {
            let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
                // Odd-numbered pieces between quotes are inside literals.
                for literal in line.split('"').skip(1).step_by(2) {
                    found.extend(literal.chars().filter(|c| !c.is_ascii()));
                }
            }
        }
        found
    }

    #[test]
    fn the_bundled_fonts_cover_every_character_the_interface_writes() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        // One frame loads the fonts; its texture updates are not drawn anywhere.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let characters = interface_characters();
        assert!(
            characters.contains(&'…') && characters.contains(&'≠'),
            "{characters:?}"
        );
        for font in [
            egui::FontId::proportional(13.0),
            egui::FontId::monospace(12.5),
            semibold(15.0),
        ] {
            let missing: Vec<char> = characters
                .iter()
                .copied()
                .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&font, *c)))
                .collect();
            assert_eq!(missing, Vec::<char>::new(), "{font:?}");
        }
        assert!(
            ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(13.0), '●')),
            "Inter adds glyphs the old font lacked"
        );
        for icon in crate::icons::Icon::ALL {
            assert!(
                ctx.fonts_mut(|f| f.has_glyph(
                    &egui::FontId::new(16.0, egui::FontFamily::Name(ICONS.into())),
                    icon.glyph()
                )),
                "{icon:?} must render through the dedicated icon family"
            );
        }
    }

    #[test]
    fn phosphor_is_isolated_from_text_families() {
        use egui::FontFamily;
        let fonts = fonts();
        let family = &fonts.families[&FontFamily::Proportional];
        assert_eq!(family[0], "Inter");
        assert_eq!(
            &family[1..],
            egui::FontDefinitions::default().families[&FontFamily::Proportional]
        );
        assert_eq!(
            fonts.families[&FontFamily::Name(ICONS.into())],
            ["Phosphor"]
        );
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(SEMIBOLD.into()),
        ] {
            assert!(
                !fonts.families[&family]
                    .iter()
                    .any(|name| name == "Phosphor")
            );
        }
    }

    #[test]
    fn every_icon_is_in_the_embedded_phosphor_cmap() {
        use skrifa::MetadataProvider;
        let fonts = fonts();
        let font =
            skrifa::FontRef::new(fonts.font_data["Phosphor"].font.as_ref().as_ref()).unwrap();
        let cmap = font.charmap();
        for icon in crate::icons::Icon::ALL {
            let glyph = cmap.map(icon.glyph()).unwrap_or_else(|| {
                panic!(
                    "{icon:?} U+{:04X} is missing from Phosphor",
                    u32::from(icon.glyph())
                )
            });
            assert_ne!(glyph.to_u32(), 0, "{icon:?} must not map to .notdef");
        }
    }

    #[test]
    fn the_choice_is_stored_by_name() {
        assert_eq!(
            serde_json::to_string(&ThemeChoice::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::from_str::<ThemeChoice>("\"light\"").unwrap(),
            ThemeChoice::Light
        );
    }
}
