//! An event stream's events: one row each (time since the response began, event name, the
//! data's first line), virtualized as the body viewer is, so a long stream costs only the
//! rows on screen; the event chosen is shown whole under the list.

use egui::{RichText, Ui};
use imposter_app::core::sse::Event;

use crate::theme::{Tone, palette};
use crate::widgets::{name, tone_text};

/// How tall the whole event under the list may grow before it scrolls.
const DETAIL_HEIGHT: f32 = 120.0;

/// "+1.25 s", "+350 ms".
pub fn since_start(at_ms: u64) -> String {
    if at_ms < 1000 {
        format!("+{at_ms} ms")
    } else {
        #[allow(clippy::cast_precision_loss)] // seconds to two decimals
        let seconds = at_ms as f64 / 1000.0;
        format!("+{seconds:.2} s")
    }
}

/// The list of `events`, newest last; `follow` keeps the newest in view while a stream
/// runs. Clicking a row chooses it for the whole-event view.
pub fn list(ui: &mut Ui, id: &str, events: &[Event], selected: &mut Option<usize>, follow: bool) {
    if events.is_empty() {
        ui.add_space(12.0);
        ui.label(tone_text("No events yet.", Tone::Muted));
        return;
    }
    let row = ui.spacing().interact_size.y;
    let chosen = selected.and_then(|index| events.get(index));
    let detail = if chosen.is_some() {
        DETAIL_HEIGHT + 12.0
    } else {
        0.0
    };
    let list_height = (ui.available_height() - detail).max(row * 3.0);
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    egui::ScrollArea::vertical()
        .id_salt((id, "events"))
        .max_height(list_height)
        .auto_shrink([false, true])
        .stick_to_bottom(follow)
        .show_rows(ui, row, events.len(), |ui, rows| {
            for index in rows {
                let event = &events[index];
                let first = event.data.lines().next().unwrap_or_default();
                let width = ui.available_width();
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, row), egui::Sense::click());
                if *selected == Some(index) {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        palette().selected_bar.gamma_multiply(0.15),
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 0.0, palette().surface);
                }
                let text = |x: f32, w: f32, text: &str, color: egui::Color32| {
                    let galley =
                        ui.painter()
                            .layout(text.to_owned(), mono.clone(), color, f32::INFINITY);
                    let clip = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + x, rect.top()),
                        egui::vec2(w.max(0.0), row),
                    );
                    ui.painter().with_clip_rect(clip).galley(
                        egui::pos2(clip.left(), rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                };
                text(4.0, 76.0, &since_start(event.at_ms), palette().text_muted);
                text(84.0, 110.0, &event.event, palette().accent);
                text(200.0, width - 204.0, first, palette().text);
                name(
                    &response,
                    &format!("Event {}: {} {first}", index + 1, event.event),
                );
                if response.clicked() {
                    *selected = (*selected != Some(index)).then_some(index);
                }
            }
        });
    if let Some(event) = chosen {
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt((id, "event"))
            .max_height(DETAIL_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut heading = format!("{} · {}", event.event, since_start(event.at_ms));
                if !event.id.is_empty() {
                    heading.push_str(" · id ");
                    heading.push_str(&event.id);
                }
                ui.label(tone_text(heading, Tone::Muted));
                ui.add(
                    egui::Label::new(RichText::new(event.data.as_str()).monospace())
                        .wrap()
                        .selectable(true),
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::since_start;

    #[test]
    fn times_read_from_the_start_of_the_response() {
        assert_eq!(since_start(350), "+350 ms");
        assert_eq!(since_start(1250), "+1.25 s");
    }
}
