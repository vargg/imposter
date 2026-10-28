//! Small vector icons, drawn at the current display scale without an icon font.

use egui::{Color32, Pos2, Rect, Response, Stroke, Ui, vec2};

use crate::theme::PALETTE;

#[derive(Clone, Copy)]
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
    Save,
    Pencil,
    Trash,
    /// A shell prompt, for curl commands.
    Terminal,
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
    let response = ui.add_sized(
        [28.0, 28.0],
        egui::Button::new("")
            .selected(selected)
            .frame_when_inactive(selected),
    );
    crate::widgets::name(&response, label);
    if ui.is_rect_visible(response.rect) {
        let color = if !ui.is_enabled() {
            PALETTE.text_muted.gamma_multiply(0.45)
        } else if selected || response.hovered() || response.has_focus() {
            PALETTE.accent
        } else {
            PALETTE.text_muted
        };
        paint(ui, icon, response.rect.center() - vec2(8.0, 8.0), color);
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

fn paint(ui: &Ui, icon: Icon, origin: Pos2, color: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.5, color);
    let point = |x, y| origin + vec2(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    };
    let rectangle = |min: (f32, f32), max: (f32, f32)| {
        painter.rect_stroke(
            Rect::from_min_max(point(min.0, min.1), point(max.0, max.1)),
            2.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Sidebar => {
            rectangle((1.0, 2.0), (15.0, 14.0));
            line((6.0, 2.0), (6.0, 14.0));
        }
        Icon::Columns => {
            rectangle((1.0, 2.0), (15.0, 14.0));
            line((8.0, 2.0), (8.0, 14.0));
        }
        Icon::Rows => {
            rectangle((1.0, 2.0), (15.0, 14.0));
            line((1.0, 8.0), (15.0, 8.0));
        }
        Icon::Revert => {
            // A counter-clockwise arrow.
            let center = point(8.0, 8.5);
            let arc: Vec<Pos2> = (0..=12_u8)
                .map(|step| {
                    let angle = std::f32::consts::PI * (-0.15 + 1.25 * f32::from(step) / 12.0);
                    center + vec2(angle.cos(), -angle.sin()) * 5.5
                })
                .collect();
            let tip = arc[arc.len() - 1];
            painter.add(egui::Shape::line(arc, stroke));
            line(
                (tip.x - origin.x, tip.y - origin.y),
                (tip.x - origin.x - 0.5, tip.y - origin.y - 4.0),
            );
            line(
                (tip.x - origin.x, tip.y - origin.y),
                (tip.x - origin.x + 3.8, tip.y - origin.y - 1.5),
            );
        }
        Icon::More => {
            for x in [3.0, 8.0, 13.0] {
                painter.circle_filled(point(x, 8.0), 1.5, color);
            }
        }
        Icon::Plus => {
            line((2.0, 8.0), (14.0, 8.0));
            line((8.0, 2.0), (8.0, 14.0));
        }
        Icon::Copy => {
            line((2.0, 11.0), (2.0, 2.0));
            line((2.0, 2.0), (11.0, 2.0));
            rectangle((5.0, 5.0), (14.0, 14.0));
        }
        Icon::Search => {
            painter.circle_stroke(point(6.5, 6.5), 4.5, stroke);
            line((10.0, 10.0), (14.0, 14.0));
        }
        Icon::Wrap => {
            line((2.0, 3.0), (14.0, 3.0));
            painter.add(egui::Shape::line(
                vec![
                    point(2.0, 7.0),
                    point(12.0, 7.0),
                    point(14.0, 9.0),
                    point(14.0, 11.0),
                    point(12.0, 13.0),
                    point(7.0, 13.0),
                ],
                stroke,
            ));
            line((9.0, 10.0), (6.0, 13.0));
            line((6.0, 13.0), (9.0, 16.0));
            line((2.0, 11.0), (4.0, 11.0));
        }
        Icon::Save => {
            // A floppy disk: body, shutter and label.
            painter.add(egui::Shape::closed_line(
                vec![
                    point(2.0, 2.0),
                    point(11.5, 2.0),
                    point(14.0, 4.5),
                    point(14.0, 14.0),
                    point(2.0, 14.0),
                ],
                stroke,
            ));
            rectangle((5.0, 2.0), (10.5, 6.0));
            rectangle((4.5, 9.0), (11.5, 14.0));
        }
        Icon::Pencil => {
            painter.add(egui::Shape::closed_line(
                vec![
                    point(2.5, 13.5),
                    point(3.2, 10.3),
                    point(11.0, 2.5),
                    point(13.5, 5.0),
                    point(5.7, 12.8),
                ],
                stroke,
            ));
            line((9.3, 4.2), (11.8, 6.7));
        }
        Icon::Terminal => {
            rectangle((1.0, 2.0), (15.0, 14.0));
            painter.add(egui::Shape::line(
                vec![point(4.0, 5.5), point(6.5, 8.0), point(4.0, 10.5)],
                stroke,
            ));
            line((8.0, 10.5), (12.0, 10.5));
        }
        Icon::Trash => {
            line((2.0, 4.0), (14.0, 4.0));
            painter.add(egui::Shape::line(
                vec![
                    point(6.0, 4.0),
                    point(6.0, 2.0),
                    point(10.0, 2.0),
                    point(10.0, 4.0),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(3.5, 4.0),
                    point(4.5, 14.0),
                    point(11.5, 14.0),
                    point(12.5, 4.0),
                ],
                stroke,
            ));
            line((6.5, 7.0), (6.5, 11.5));
            line((9.5, 7.0), (9.5, 11.5));
        }
    }
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
    let background = PALETTE.accent;
    let letter = PALETTE.on_accent;
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
                PALETTE.on_accent.to_array()
            );
            assert_eq!(
                pixel(size * 60 / 100, size * 33 / 100),
                PALETTE.on_accent.to_array()
            );
            assert_eq!(
                pixel(size * 60 / 100, size * 60 / 100),
                PALETTE.accent.to_array()
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
