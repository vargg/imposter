//! Phosphor Regular glyphs keep toolbar icons consistent as the action set grows.
//! A dedicated family avoids Inter's colliding private-use glyphs.
//! Named native buttons preserve keyboard, screen-reader and tooltip behavior.
//! The application mark remains drawn in code for window and packaging icons.

use egui::{Color32, FontFamily, FontId, Response, RichText, Ui};

use crate::theme::{ICONS, palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sidebar,
    /// Two panes next to each other.
    Columns,
    /// Two panes above each other.
    Rows,
    /// Undo the unsaved changes.
    Revert,
    More,
    Plus,
    Copy,
    Search,
    Wrap,
    Indent,
    Save,
    /// A shell prompt, for curl commands.
    Terminal,
    /// Chevrons pointing together: fold everything.
    Collapse,
    /// Chevrons pointing apart: unfold everything.
    Expand,
    /// A funnel.
    Filter,
    /// Kept as a secret.
    Lock,
    /// Show a hidden value.
    Eye,
    /// Hide a shown value: the eye, struck through.
    EyeOff,
    /// One chevron each way: previous and next, newer and older.
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    /// A cross: close a bar or popup.
    Close,
    /// Choose a file.
    Folder,
    /// A clockwise arrow: load again.
    Refresh,
    Settings,
    Trash,
    Paperclip,
    Info,
    Check,
    Warning,
    Failed,
    Passed,
    Play,
    Stop,
    FolderOpen,
    Requests,
    Variables,
    History,
    Mocks,
    Load,
    Run,
    Key,
    Cookie,
    Globe,
}

impl Icon {
    pub const ALL: [Self; 45] = [
        Self::Sidebar,
        Self::Columns,
        Self::Rows,
        Self::Revert,
        Self::More,
        Self::Plus,
        Self::Copy,
        Self::Search,
        Self::Wrap,
        Self::Indent,
        Self::Save,
        Self::Terminal,
        Self::Collapse,
        Self::Expand,
        Self::Filter,
        Self::Lock,
        Self::Eye,
        Self::EyeOff,
        Self::ChevronUp,
        Self::ChevronDown,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::Close,
        Self::Folder,
        Self::Refresh,
        Self::Settings,
        Self::Trash,
        Self::Paperclip,
        Self::Info,
        Self::Check,
        Self::Warning,
        Self::Failed,
        Self::Passed,
        Self::Play,
        Self::Stop,
        Self::FolderOpen,
        Self::Requests,
        Self::Variables,
        Self::History,
        Self::Mocks,
        Self::Load,
        Self::Run,
        Self::Key,
        Self::Cookie,
        Self::Globe,
    ];

    /// Codepoints from `@phosphor-icons/web` 2.1.2, `src/regular/style.css`.
    pub const fn glyph(self) -> char {
        match self {
            Self::Sidebar => '\u{ec24}',      // sidebar-simple
            Self::Columns => '\u{e546}',      // columns
            Self::Rows => '\u{e5a2}',         // rows
            Self::Revert => '\u{e038}',       // arrow-counter-clockwise
            Self::More => '\u{e1fe}',         // dots-three
            Self::Plus => '\u{e3d4}',         // plus
            Self::Copy => '\u{e1ca}',         // copy
            Self::Search => '\u{e30c}',       // magnifying-glass
            Self::Wrap => '\u{e484}',         // text-align-left
            Self::Indent => '\u{ea1e}',       // text-indent
            Self::Save => '\u{e248}',         // floppy-disk
            Self::Terminal => '\u{eae8}',     // terminal-window
            Self::Collapse => '\u{e532}',     // arrows-in-line-vertical
            Self::Expand => '\u{e536}',       // arrows-out-line-vertical
            Self::Filter => '\u{e266}',       // funnel
            Self::Lock => '\u{e308}',         // lock-simple
            Self::Eye => '\u{e220}',          // eye
            Self::EyeOff => '\u{e224}',       // eye-slash
            Self::ChevronUp => '\u{e13c}',    // caret-up
            Self::ChevronDown => '\u{e136}',  // caret-down
            Self::ChevronLeft => '\u{e138}',  // caret-left
            Self::ChevronRight => '\u{e13a}', // caret-right
            Self::Close => '\u{e4f6}',        // x
            Self::Folder => '\u{e25a}',       // folder-simple
            Self::Refresh => '\u{e036}',      // arrow-clockwise
            Self::Settings => '\u{e272}',     // gear-six
            Self::Trash => '\u{e4a6}',        // trash
            Self::Paperclip => '\u{e39a}',    // paperclip
            Self::Info => '\u{e2ce}',         // info
            Self::Check => '\u{e182}',        // check
            Self::Warning => '\u{e4e0}',      // warning
            Self::Failed => '\u{e4f8}',       // x-circle
            Self::Passed => '\u{e184}',       // check-circle
            Self::Play => '\u{e3d0}',         // play
            Self::Stop => '\u{e46c}',         // stop
            Self::FolderOpen => '\u{e256}',   // folder-open
            Self::Requests => '\u{e398}',     // paper-plane-tilt
            Self::Variables => '\u{e860}',    // brackets-curly
            Self::History => '\u{e1a0}',      // clock-counter-clockwise
            Self::Mocks => '\u{e1da}',        // cube
            Self::Load => '\u{e628}',         // gauge
            Self::Run => '\u{e3d2}',          // play-circle
            Self::Key => '\u{e2d6}',          // key
            Self::Cookie => '\u{e6ca}',       // cookie
            Self::Globe => '\u{e288}',        // globe
        }
    }
}

/// An icon for a text label, in its own section to avoid Inter's private-use glyphs.
pub fn glyph_text(icon: Icon, size: f32) -> RichText {
    RichText::new(icon.glyph().to_string()).font(FontId::new(size, FontFamily::Name(ICONS.into())))
}

/// The side of an icon button: as tall as the other controls, so they line up in a row.
pub fn size() -> f32 {
    crate::theme::spacing().control
}

/// Native buttons keep keyboard and AccessKit behavior even when their label is only a tooltip.
pub fn button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    button_with_hint(ui, icon, label, label, selected)
}

/// An icon button named `label` whose hover text is `hint` (for example with its shortcut).
pub fn button_with_hint(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    selected: bool,
) -> Response {
    drawn(ui, icon, label, hint, selected, palette().text_muted)
}

/// An icon button with something to do now (Save with unsaved changes): drawn in the text
/// color, apart from its muted resting and faded disabled looks.
pub fn ready_button(ui: &mut Ui, icon: Icon, label: &str, hint: &str) -> Response {
    drawn(ui, icon, label, hint, false, palette().text)
}

fn drawn(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    selected: bool,
    resting: Color32,
) -> Response {
    let response = ui.add_sized(
        [size(), size()],
        egui::Button::new("")
            .selected(selected)
            .frame_when_inactive(selected),
    );
    crate::widgets::name(&response, label);
    if ui.is_rect_visible(response.rect) {
        let color = if !ui.is_enabled() {
            palette().text_muted.gamma_multiply(0.45)
        } else if selected || response.has_focus() {
            palette().accent
        } else if response.hovered() {
            // Hover is neutral; orange is for primary, selected and focused things.
            palette().text
        } else {
            resting
        };
        // The theme's text override must not replace the button's state color.
        let galley = egui::WidgetText::from(glyph_text(icon, 16.0).color(Color32::PLACEHOLDER))
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
        ui.painter()
            .galley(response.rect.center() - galley.size() / 2.0, galley, color);
    }
    if ui.is_enabled() {
        response.on_hover_text(hint)
    } else {
        response.on_disabled_hover_text(hint)
    }
}

pub fn menu<R>(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let response = button(ui, icon, label, false);
    egui::Popup::menu(&response).show(contents)
}

/// The packaging icons: file name and pixel size. macOS names 1024 pixels `512x512@2x`.
pub const APP_ICON_FILES: [(&str, u32); 5] = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("256x256.png", 256),
    ("512x512.png", 512),
    ("512x512@2x.png", 1024),
];

/// The app icon, `size`×`size` RGBA: the header's accent rounded square with a dark I/apostrophe mark,
/// inside a transparent margin as the platforms' icon grids expect. Packaging uses PNGs
/// written from this (`examples/app_icon.rs`); a test keeps them in step.
pub fn app_icon(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let inside = |x: f32, y: f32| -> (bool, bool) {
        // The rounded square, in unit coordinates with a 10% margin.
        let (min, max, radius) = (0.1, 0.9, 0.18);
        let cx = x.clamp(min + radius, max - radius);
        let cy = y.clamp(min + radius, max - radius);
        let square = (x - cx).hypot(y - cy) <= radius;
        let stem = (0.356..=0.452).contains(&x) && (0.292..=0.716).contains(&y);
        let fraction = (y - 0.284) / 0.192;
        let apostrophe = (0.0..=1.0).contains(&fraction)
            && x >= 0.5592 - 0.0192 * fraction
            && x <= 0.668 - 0.0576 * fraction;
        (square, stem || apostrophe)
    };
    let background = palette().accent;
    let letter = palette().on_accent;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    #[allow(clippy::cast_precision_loss)] // icon sizes are small
    let scale = (size * SAMPLES) as f32;
    for row in 0..size {
        for column in 0..size {
            let (mut covered, mut letter_hits) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    #[allow(clippy::cast_precision_loss)]
                    let (x, y) = (
                        ((column * SAMPLES + sx) as f32 + 0.5) / scale,
                        ((row * SAMPLES + sy) as f32 + 0.5) / scale,
                    );
                    let (square, glyph) = inside(x, y);
                    if square {
                        covered += 1;
                        letter_hits += u32::from(glyph);
                    }
                }
            }
            if covered == 0 {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // Mix letter into background by its share of the covered samples; alpha is the
            // share of the square. Colors are opaque, so straight alpha is exact.
            let mix = |a: u8, b: u8| {
                let value =
                    (u32::from(a) * (covered - letter_hits) + u32::from(b) * letter_hits) / covered;
                u8::try_from(value).unwrap_or(u8::MAX)
            };
            let total = SAMPLES * SAMPLES;
            pixels.extend_from_slice(&[
                mix(background.r(), letter.r()),
                mix(background.g(), letter.g()),
                mix(background.b(), letter.b()),
                u8::try_from(covered * 255 / total).unwrap_or(u8::MAX),
            ]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packaging icons are exactly what `app_icon` draws (regenerate with
    /// `cargo run -p imposter-ui --example app_icon -- crates/imposter/icons`).
    #[test]
    fn committed_icons_match_the_drawing() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../imposter/icons");
        for (file, size) in APP_ICON_FILES {
            #[allow(clippy::disallowed_methods)] // a test reading a fixture
            let bytes = std::fs::read(dir.join(file)).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), (size, size));
            assert!(decoded.into_raw() == app_icon(size), "{file} is stale");
        }
    }

    #[test]
    fn the_mark_has_separate_stem_and_apostrophe_at_every_size() {
        for size in [16, 32, 64, 128, 256, 512, 1024] {
            let pixels = app_icon(size);
            let pixel = |x: u32, y: u32| {
                let offset = ((y * size + x) * 4) as usize;
                &pixels[offset..offset + 4]
            };
            assert_eq!(
                pixel(size * 40 / 100, size / 2),
                palette().on_accent.to_array()
            );
            assert_eq!(
                pixel(size * 60 / 100, size * 33 / 100),
                palette().on_accent.to_array()
            );
            assert_eq!(
                pixel(size * 60 / 100, size * 60 / 100),
                palette().accent.to_array()
            );
        }
    }

    #[test]
    fn the_icon_has_a_transparent_margin_and_an_opaque_body() {
        let size = 64;
        let pixels = app_icon(size);
        let alpha = |x: u32, y: u32| pixels[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(size / 2, 2), 0);
        assert_eq!(alpha(size / 2, size / 2), 255);
    }
}
