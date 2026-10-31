//! The themes: a dark and a light palette of named roles. Every color the UI uses comes from
//! [`palette()`], the palette of the theme the current frame draws in.

use std::cell::Cell;

use egui::{Color32, CornerRadius, Stroke, Theme, ThemePreference, Visuals};

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
    text: Color32::from_rgb(0x1a, 0x1e, 0x26),
    text_muted: Color32::from_rgb(0x59, 0x62, 0x73),
    text_disabled: Color32::from_rgb(0x8a, 0x91, 0x9d),
    accent: Color32::from_rgb(0xb8, 0x3b, 0x0a),
    on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
    ok: Color32::from_rgb(0x15, 0x70, 0x2f),
    warning: Color32::from_rgb(0x87, 0x5a, 0x00),
    error: Color32::from_rgb(0xc6, 0x28, 0x28),
    syntax: Syntax {
        key: Color32::from_rgb(0x0b, 0x57, 0xa4),
        string: Color32::from_rgb(0xa1, 0x35, 0x0f),
        number: Color32::from_rgb(0x1a, 0x6b, 0x2e),
        keyword: Color32::from_rgb(0x6f, 0x3f, 0xb5),
        tag: Color32::from_rgb(0x0f, 0x6b, 0x5c),
        comment: Color32::from_rgb(0x5f, 0x6b, 0x6d),
    },
};

thread_local! {
    /// egui draws a frame on one thread; each frame starts by setting this from its theme.
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&DARK) };
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
    visuals.selection.stroke = Stroke::new(1.0, palette.text);
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
    visuals.widgets.inactive.weak_bg_fill = palette.surface;
    visuals.widgets.inactive.bg_fill = palette.surface;
    // The edge of every control a user operates: fields and buttons (WCAG 1.4.11).
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.field_border);
    visuals.widgets.hovered.weak_bg_fill = palette.hover;
    visuals.widgets.hovered.bg_fill = palette.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals.widgets.active.weak_bg_fill = palette.selected;
    visuals.widgets.active.bg_fill = palette.selected;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals.widgets.open.weak_bg_fill = palette.hover;
    visuals.widgets.open.bg_fill = palette.hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, palette.border);
    visuals
}

/// Install both themes and choose `choice`. Each frame then draws in one of them
/// ([`begin_frame`]).
pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    for theme in [Theme::Dark, Theme::Light] {
        ctx.set_visuals_of(theme, visuals(of(theme), theme));
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.interact_size.y = 28.0;
            style.spacing.window_margin = egui::Margin::same(16);
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
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
    fn text_on_the_send_button_is_readable_in_both_states() {
        for (name, palette) in BOTH {
            // Send uses the accent fill; Cancel (while a request runs) the warning fill.
            for fill in [palette.accent, palette.warning] {
                let ratio = contrast(palette.on_accent, fill);
                assert!(ratio >= AA, "{name} {fill:?}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn apply_installs_both_themes_and_the_choice() {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeChoice::Dark);
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

        apply(&ctx, ThemeChoice::Light);
        assert_eq!(ctx.theme(), Theme::Light);
        assert_eq!(palette(), &LIGHT, "the next frame draws in light");
        assert_eq!(
            ctx.style_of(Theme::Light).visuals.override_text_color,
            Some(LIGHT.text)
        );

        apply(&ctx, ThemeChoice::System);
        assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System);
        CURRENT.with(|current| current.set(&DARK));
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
