//! The single dark theme. Every color the UI uses comes from [`PALETTE`].

use egui::{Color32, CornerRadius, Stroke, Theme, Visuals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Window and panel background.
    pub background: Color32,
    /// Raised areas: side panel, popups, tables.
    pub surface: Color32,
    /// Text inputs and code areas.
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    /// Text drawn on an `accent` fill (the Send button).
    pub on_accent: Color32,
    pub ok: Color32,
    pub warning: Color32,
    pub error: Color32,
}

pub const PALETTE: Palette = Palette {
    background: Color32::from_rgb(0x1e, 0x1e, 0x1e),
    surface: Color32::from_rgb(0x25, 0x25, 0x26),
    input: Color32::from_rgb(0x16, 0x16, 0x16),
    border: Color32::from_rgb(0x3c, 0x3c, 0x3c),
    text: Color32::from_rgb(0xe6, 0xe6, 0xe6),
    text_muted: Color32::from_rgb(0xa8, 0xa8, 0xa8),
    accent: Color32::from_rgb(0xff, 0x6c, 0x37),
    on_accent: Color32::from_rgb(0x1a, 0x1a, 0x1a),
    ok: Color32::from_rgb(0x5f, 0xd0, 0x8a),
    warning: Color32::from_rgb(0xf0, 0xc0, 0x5a),
    error: Color32::from_rgb(0xff, 0x6b, 0x6b),
};

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

pub fn visuals(palette: &Palette) -> Visuals {
    let mut visuals = Visuals::dark();
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
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, palette.text);

    let radius = CornerRadius::same(4);
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
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals
}

/// Install the theme. The dark theme is used even when the OS prefers light.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&PALETTE));
    ctx.set_theme(Theme::Dark);
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

    #[test]
    fn contrast_formula_matches_known_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn every_text_tone_is_readable_on_every_background() {
        for background in [PALETTE.background, PALETTE.surface, PALETTE.input] {
            for tone in [
                Tone::Normal,
                Tone::Muted,
                Tone::Ok,
                Tone::Warning,
                Tone::Error,
            ] {
                let ratio = contrast(PALETTE.tone(tone), background);
                assert!(ratio >= AA, "{tone:?} on {background:?}: {ratio:.2}");
            }
            let accent = contrast(PALETTE.accent, background);
            assert!(accent >= AA, "accent on {background:?}: {accent:.2}");
        }
    }

    #[test]
    fn text_on_the_accent_fill_is_readable() {
        assert!(contrast(PALETTE.on_accent, PALETTE.accent) >= AA);
    }

    #[test]
    fn apply_installs_the_palette_and_forces_dark() {
        let ctx = egui::Context::default();
        apply(&ctx);

        let style = ctx.style_of(Theme::Dark);
        assert_eq!(style.visuals.panel_fill, PALETTE.background);
        assert_eq!(style.visuals.override_text_color, Some(PALETTE.text));
        assert_eq!(ctx.theme(), Theme::Dark);
    }
}
