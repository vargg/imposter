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
