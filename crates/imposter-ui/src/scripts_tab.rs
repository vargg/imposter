//! The Scripts tab: a request's before-sending and after-response scripts, one under the
//! other (two levels of tabs were removed from History for good reason), each with the
//! examples for its phase and a line saying whether it compiles.

use egui::{FontId, RichText, Ui};
use imposter_app::core::model::Scripts;
use imposter_app::script::{self, Phase};

use crate::app::Action;
use crate::dialogs::Dialog;
use crate::highlight;
use crate::theme::Tone;
use crate::widgets::{self, name, tone_text};

/// Ready-made scripts, by phase: what to do, and the code inserted at the cursor.
pub const EXAMPLES: &[(Phase, &str, &str)] = &[
    (
        Phase::Before,
        "Sign the body with HMAC-SHA256",
        "request.set_header(\"X-Signature\", hex(hmac_sha256(vars.get(\"secret\"), request.body)));\n",
    ),
    (
        Phase::Before,
        "Set a header from a variable",
        "request.set_header(\"Authorization\", \"Bearer \" + vars.get(\"token\"));\n",
    ),
    (
        Phase::Before,
        "Add a timestamp and a request id",
        "request.set_query(\"ts\", `${now_ms()}`);\nrequest.set_header(\"X-Request-Id\", uuid());\n",
    ),
    (
        Phase::After,
        "Test the status",
        "test(\"status is 200\", response.status == 200);\n",
    ),
    (
        Phase::After,
        "Test every item of a JSON array",
        "test(\"every item has an id\", || {\n    for item in response.path(\"$.items[*]\") {\n        if item.id == () { throw \"an item has no id\"; }\n    }\n});\n",
    ),
    (
        Phase::After,
        "Set a variable from the response",
        "vars.set(\"token\", response.path(\"$.access_token\"));\n",
    ),
    (
        Phase::After,
        "Log a value",
        "print(`${response.status} in ${response.time_ms} ms`);\n",
    ),
];

/// The accessible name of each editor; tests and screen readers find them by it.
pub const BEFORE_EDITOR: &str = "Before-sending script";
pub const AFTER_EDITOR: &str = "After-response script";

/// Scripts this long or longer are not highlighted, like large bodies.
const HIGHLIGHT_LIMIT: usize = 64 * 1024;

pub fn draw(ui: &mut Ui, scripts: &mut Scripts, actions: &mut Vec<Action>) -> bool {
    let mut edited = false;
    // The tab scrolls both ways; holding everything to the pane's width keeps a narrow pane
    // from widening as rows are laid out.
    ui.set_max_width(ui.clip_rect().width().max(160.0));
    ui.add(
        egui::Label::new(tone_text(
            "Rhai scripts. They read and change only this request and its variables: no files or network.",
            Tone::Muted,
        ))
        .wrap(),
    );
    if ui.button("Script reference").clicked() {
        actions.push(Action::Prompt(Dialog::ScriptReference));
    }
    ui.add_space(8.0);
    for (phase, title, hint, label, text) in [
        (
            Phase::Before,
            "Before sending",
            "Runs with variables filled in; its changes are sent and kept in History",
            BEFORE_EDITOR,
            &mut scripts.before,
        ),
        (
            Phase::After,
            "After response",
            "Runs on the response: tests, variables, logging",
            AFTER_EDITOR,
            &mut scripts.after,
        ),
    ] {
        edited |= section(ui, phase, title, hint, label, text);
        ui.add_space(12.0);
    }
    edited
}

fn section(
    ui: &mut Ui,
    phase: Phase,
    title: &str,
    hint: &str,
    label: &str,
    text: &mut String,
) -> bool {
    let id = ui.make_persistent_id(("script", label));
    let mut inserted = None;
    // Wrapped: in a narrow pane the menu goes under the title rather than past the edge.
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(title).font(crate::theme::semibold(13.0)));
        let examples = ui.menu_button("Insert example…", |ui| {
            for (_, what, code) in EXAMPLES.iter().filter(|(p, ..)| *p == phase) {
                if ui.button(*what).clicked() {
                    inserted = Some(*code);
                    ui.close();
                }
            }
        });
        name(
            &examples.response,
            &format!("Insert an example into the {label}"),
        );
    });
    ui.add(egui::Label::new(tone_text(hint, Tone::Muted).small()).wrap());
    let typing = (text.len() <= HIGHLIGHT_LIMIT)
        .then(|| widgets::Typing::read(ui, id, text))
        .flatten();
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
        let content = buffer.as_str();
        let spans = if content.len() <= HIGHLIGHT_LIMIT {
            highlight::rhai(content)
        } else {
            vec![(0..content.len(), highlight::Token::Plain)]
        };
        let job = widgets::spans_job(content, &spans, &FontId::monospace(13.0), wrap);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let response = ui.add(
        egui::TextEdit::multiline(text)
            .id(id)
            .code_editor()
            .desired_rows(6)
            .desired_width(ui.available_width())
            .hint_text("// Rhai")
            .layouter(&mut layouter),
    );
    name(&response, label);
    let mut changed = response.changed();
    if let Some(typing) = typing {
        typing.help(ui, id, text, changed);
    }
    if let Some(code) = inserted {
        insert_at_cursor(ui, id, text, code);
        changed = true;
    }
    if !text.trim().is_empty() {
        let status = match script::check(text) {
            Ok(()) => tone_text("Compiles", Tone::Ok),
            Err(error) => tone_text(error.to_string(), Tone::Error),
        };
        ui.add(egui::Label::new(status.small()).wrap());
    }
    changed
}

/// Put `code` where the cursor is (at the end when the field never had one), on a line of
/// its own, and leave the cursor after it.
fn insert_at_cursor(ui: &Ui, id: egui::Id, text: &mut String, code: &str) {
    let ctx = ui.ctx();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let at = state.cursor.char_range().map_or(text.len(), |range| {
        let [start, _] = range.sorted_cursors();
        text.char_indices()
            .nth(start.index.0)
            .map_or(text.len(), |(byte, _)| byte)
    });
    let lead = if at > 0 && !text[..at].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let insert = format!("{lead}{code}");
    text.insert_str(at, &insert);
    let after = text[..at + insert.len()].chars().count();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(after),
        )));
    state.store(ctx, id);
}
