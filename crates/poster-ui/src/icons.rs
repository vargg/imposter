//! Small vector icons, drawn at the current display scale without an icon font.

use egui::{Color32, Pos2, Rect, Response, Stroke, Ui, vec2};

use crate::theme::PALETTE;

#[derive(Clone, Copy)]
pub enum Icon {
    Sidebar,
    Layout,
    More,
    Plus,
    Copy,
    Search,
    Wrap,
}

/// Native buttons keep keyboard and AccessKit behavior even when their label is only a tooltip.
pub fn button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let response = ui.add_sized(
        [28.0, 28.0],
        egui::Button::new("")
            .selected(selected)
            .frame_when_inactive(selected),
    );
    crate::widgets::name(&response, label);
    if ui.is_rect_visible(response.rect) {
        let color = if selected || response.hovered() || response.has_focus() {
            PALETTE.accent
        } else {
            PALETTE.text_muted
        };
        paint(ui, icon, response.rect.center() - vec2(8.0, 8.0), color);
    }
    response.on_hover_text(label)
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
        Icon::Layout => {
            rectangle((1.0, 2.0), (6.0, 14.0));
            rectangle((9.0, 2.0), (15.0, 7.0));
            rectangle((9.0, 10.0), (15.0, 14.0));
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

/// The app icon, `size`×`size` RGBA: the header's accent rounded square with a dark "P",
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
        // The "P": a stem and a bowl of the same stroke width.
        let stroke = 0.115;
        let (stem_left, top, bottom) = (0.33, 0.24, 0.76);
        let stem = (stem_left..=stem_left + stroke).contains(&x) && (top..=bottom).contains(&y);
        let (bowl_x, bowl_y, outer) = (0.52, 0.41, 0.17);
        let bars = (stem_left..=bowl_x).contains(&x)
            && ((top..=top + stroke).contains(&y)
                || (bowl_y + outer - stroke..=bowl_y + outer).contains(&y));
        let distance = (x - bowl_x).hypot(y - bowl_y);
        let arc = x >= bowl_x && (outer - stroke..=outer).contains(&distance);
        (square, stem || bars || arc)
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
    /// `cargo run -p poster-ui --example app_icon -- crates/poster/icons`).
    #[test]
    fn committed_icons_match_the_drawing() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../poster/icons");
        for (file, size) in APP_ICON_FILES {
            #[allow(clippy::disallowed_methods)] // a test reading a fixture
            let bytes = std::fs::read(dir.join(file)).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), (size, size));
            assert!(decoded.into_raw() == app_icon(size), "{file} is stale");
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
