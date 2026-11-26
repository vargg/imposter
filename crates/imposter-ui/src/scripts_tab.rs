//! The Scripts tab: a request's before-sending and after-response scripts, one under the
//! other (two levels of tabs were removed from History for good reason), each with the
//! examples for its phase and compile state in its header.

use egui::{FontId, Ui};
use imposter_app::core::model::Scripts;
use imposter_app::script::{self, Phase};

use crate::app::Action;
use crate::dialogs::Dialog;
use crate::highlight;
use crate::icons::{self, Icon};
use crate::theme::{self, Tone};
use crate::widgets::{self, Variant, name};

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
    (
        Phase::Mock,
        "Echo the request body",
        "response.set_header(\"Content-Type\", request.header(\"content-type\") ?? \"text/plain\");\nresponse.body = request.body;\n",
    ),
    (
        Phase::Mock,
        "Answer 400 when a field is missing",
        "let order = request.json();\nif order.item == () {\n    response.status = 400;\n    response.send_json(#{ error: \"item is required\" });\n}\n",
    ),
    (
        Phase::Mock,
        "Fail the first call, then succeed",
        "state.calls = (state.calls ?? 0) + 1;\nif state.calls == 1 {\n    response.status = 503;\n}\n",
    ),
    (Phase::Mock, "Answer slowly", "response.delay_ms = 2000;\n"),
];

/// The accessible name of each editor; tests and screen readers find them by it.
pub const BEFORE_EDITOR: &str = "Before-sending script";
pub const AFTER_EDITOR: &str = "After-response script";

/// Scripts this long or longer are not highlighted, like large bodies.
const HIGHLIGHT_LIMIT: usize = 64 * 1024;

/// A collection's or folder's scripts that also run around the open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outer {
    pub name: String,
    pub target: crate::dialogs::ScriptTarget,
    pub scripts: Scripts,
}

impl Outer {
    fn owner(&self) -> String {
        let kind = match self.target {
            crate::dialogs::ScriptTarget::Collection(_) => "Collection",
            crate::dialogs::ScriptTarget::Folder(_) => "Folder",
        };
        format!("{kind} \"{}\"", self.name)
    }
}

/// What a collection's or folder's scripts do to the requests inside, in a sentence; empty
/// when it has none.
pub fn runs(scripts: &Scripts) -> &'static str {
    match (
        scripts.before.trim().is_empty(),
        scripts.after.trim().is_empty(),
    ) {
        (false, false) => "Runs scripts before sending and after each response",
        (false, true) => "Runs a script before sending",
        (true, false) => "Runs a script after each response",
        (true, true) => "",
    }
}

/// The Scripts tab's badge description when outer levels run scripts: "Collection \"API\"
/// and Folder \"Users\" scripts also run".
pub fn also_run(outer: &[Outer]) -> Option<String> {
    let owners: Vec<_> = outer.iter().map(Outer::owner).collect();
    let (last, rest) = owners.split_last()?;
    let joined = if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    };
    Some(format!("{joined} scripts also run"))
}

pub fn draw(
    ui: &mut Ui,
    scripts: &mut Scripts,
    outer: &[Outer],
    actions: &mut Vec<Action>,
) -> bool {
    let height = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt("request scripts")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::scroll_view_bounds(ui);
            let top = ui.cursor().top();
            if !outer.is_empty() {
                const INHERITED: &str = "Also runs, before this request's:";
                let narrow = ui.available_width() < 480.0;
                ui.horizontal_wrapped(|ui| {
                    let caption = if narrow { "Also runs:" } else { INHERITED };
                    let heading = ui.label(theme::meta(caption).color(theme::palette().text_muted));
                    if narrow {
                        ui.ctx()
                            .accesskit_node_builder(heading.id, |node| node.set_value(INHERITED));
                        heading.on_hover_text(INHERITED);
                    }
                    for level in outer {
                        let owner = level.owner();
                        let caption = if narrow { &level.name } else { &owner };
                        let open = widgets::button(ui, Variant::Ghost, None, caption)
                            .on_hover_text(&owner);
                        name(&open, &format!("Open the scripts of {owner}"));
                        if open.clicked() {
                            actions.push(Action::Prompt(Dialog::Scripts {
                                target: level.target,
                                owner,
                                scripts: level.scripts.clone(),
                            }));
                        }
                    }
                });
            }
            // Both coordinates scroll together, so scrolling never grows either editor.
            let remaining = height - (ui.cursor().top() - top);
            script_pair(ui, scripts, None, remaining)
        })
        .inner
}

/// Share the pane or dialog body evenly between the two phases.
pub(crate) fn script_pair(
    ui: &mut Ui,
    scripts: &mut Scripts,
    owner: Option<&str>,
    height: f32,
) -> bool {
    let spacing = theme::spacing();
    let row_gap = ui.spacing().item_spacing.y;
    let header = header_height(ui) + row_gap;
    let gap = spacing.sm.max(row_gap);
    let minimum_editor =
        ui.fonts_mut(|fonts| fonts.row_height(&script_font())) + SCRIPT_MARGIN.sum().y;
    let adjacent = owner.is_none()
        && ui.available_width() >= 360.0
        && height < 2.0 * (header + minimum_editor) + gap;
    let editor_height = if adjacent {
        (height - header).max(minimum_editor)
    } else {
        ((height - gap) / 2.0 - header).max(minimum_editor)
    };
    let before = owner.map_or_else(
        || BEFORE_EDITOR.to_owned(),
        |owner| format!("{owner} before-sending script"),
    );
    let after = owner.map_or_else(
        || AFTER_EDITOR.to_owned(),
        |owner| format!("{owner} after-response script"),
    );
    let mut draw_before = |ui: &mut Ui| {
        script_editor(
            ui,
            Phase::Before,
            "Before sending",
            if owner.is_some() {
                "Runs on every request inside. Outer scripts run first, then the request's own."
            } else {
                "Runs with variables filled in. Its changes are sent and kept in History."
            },
            &before,
            &mut scripts.before,
            editor_height,
        )
    };
    let mut draw_after = |ui: &mut Ui| {
        script_editor(
            ui,
            Phase::After,
            "After response",
            if owner.is_some() {
                "Runs on every response. Its tests show with the request's."
            } else {
                "Runs on the response to test it, set variables or log values."
            },
            &after,
            &mut scripts.after,
            editor_height,
        )
    };
    if adjacent {
        ui.columns_const(|[before, after]| {
            for column in [&mut *before, &mut *after] {
                let mut clip = column.clip_rect();
                clip.min.x = clip.min.x.max(column.max_rect().left());
                clip.max.x = clip.max.x.min(column.max_rect().right());
                column.set_clip_rect(clip);
            }
            draw_before(before) | draw_after(after)
        })
    } else {
        let before = draw_before(ui);
        ui.add_space((spacing.sm - row_gap).max(0.0));
        before | draw_after(ui)
    }
}

fn header_height(ui: &Ui) -> f32 {
    theme::spacing()
        .control
        .max(icons::size())
        .max(widgets::HELP_SIZE)
        .max(ui.spacing().interact_size.y)
}

const RHAI_HELP: &str =
    "Rhai scripts. They read and change only this request and its variables: no files or network.";
const SCRIPT_MARGIN: egui::Margin = egui::Margin::symmetric(4, 2);

fn script_font() -> FontId {
    FontId::monospace(theme::BODY)
}

/// One script with a fixed header above an independently scrollable code editor.
/// The accessible name also keeps its cursor apart from other editors'.
pub fn script_editor(
    ui: &mut Ui,
    phase: Phase,
    title: &str,
    hint: &str,
    label: &str,
    text: &mut String,
    editor_height: f32,
) -> bool {
    let id = ui.make_persistent_id(("script", label));
    let compiled = (!text.trim().is_empty()).then(|| script::check(text));
    let mut inserted = None;
    let header_height = header_height(ui);
    ui.horizontal(|ui| {
        let spacing = theme::spacing();
        ui.spacing_mut().item_spacing.x = spacing.xs;
        let help = if phase == Phase::Mock {
            hint.to_owned()
        } else {
            format!("{hint} {RHAI_HELP}")
        };
        let caption = theme::section(title);
        let status_space = compiled.as_ref().map_or(0.0, |_| {
            widgets::text_width(ui, theme::meta("Compiles")) + 14.0 + 2.0 * spacing.xs
        });
        let caption_space = (ui.available_width()
            - widgets::HELP_SIZE
            - icons::size()
            - 3.0 * spacing.xs
            - status_space)
            .max(0.0);
        ui.add_sized(
            [
                widgets::text_width(ui, caption.clone()).min(caption_space),
                header_height,
            ],
            egui::Label::new(caption)
                .truncate()
                .show_tooltip_when_elided(false)
                .halign(egui::Align::Min),
        )
        .on_hover_text(&help);
        widgets::help(ui, &help);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let examples = icons::button_with_hint(
                ui,
                Icon::Plus,
                &format!("Insert an example into the {label}"),
                "Insert example…",
                false,
            );
            egui::Popup::menu(&examples).show(|ui| {
                for (_, what, code) in EXAMPLES.iter().filter(|(p, ..)| *p == phase) {
                    if widgets::option(ui, false, *what).clicked() {
                        inserted = Some(*code);
                        ui.close();
                    }
                }
            });
            if let Some(compiled) = compiled {
                match compiled {
                    Ok(()) => {
                        ui.add(
                            egui::Label::new(theme::meta("Compiles").color(theme::palette().ok))
                                .truncate(),
                        );
                        widgets::status_glyph(ui, Icon::Passed, "Script compiles", Tone::Ok);
                    }
                    Err(error) => {
                        let full = error.to_string();
                        ui.add(
                            egui::Label::new(theme::meta(&full).color(theme::palette().error))
                                .truncate()
                                .show_tooltip_when_elided(false),
                        )
                        .on_hover_text(full);
                    }
                }
            }
        });
    });
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
        let job = widgets::spans_job(content, &spans, &script_font(), wrap);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut changed = ui
        .scope(|ui| {
            // A cursor in a one-line viewport has no room for extra scroll-to-cursor padding.
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(editor_height)
                .min_scrolled_height(editor_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    widgets::scroll_view_bounds(ui);
                    let response = ui.add(
                        egui::TextEdit::multiline(text)
                            .id(id)
                            .code_editor()
                            .margin(SCRIPT_MARGIN)
                            .desired_rows(1)
                            .min_size(egui::vec2(0.0, editor_height))
                            .desired_width(ui.available_width())
                            .hint_text("// Rhai")
                            .layouter(&mut layouter),
                    );
                    name(&response, label);
                    let changed = response.changed();
                    if let Some(typing) = typing {
                        typing.help(ui, id, text, changed);
                    }
                    changed
                })
                .inner
        })
        .inner;
    if let Some(code) = inserted {
        insert_at_cursor(ui, id, text, code);
        changed = true;
    }
    if changed {
        ui.ctx().request_repaint();
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

#[cfg(test)]
mod tests {
    use imposter_app::core::ids::FolderId;

    use super::*;
    use crate::dialogs::ScriptTarget;

    fn folder(name: &str) -> Outer {
        Outer {
            name: name.to_owned(),
            target: ScriptTarget::Folder(FolderId(1)),
            scripts: Scripts {
                before: "let a = 1;".into(),
                ..Scripts::default()
            },
        }
    }

    #[test]
    fn also_run_names_every_outer_level_in_order() {
        assert_eq!(also_run(&[]), None);
        assert_eq!(
            also_run(&[folder("A"), folder("B"), folder("C")]).as_deref(),
            Some("Folder \"A\", Folder \"B\" and Folder \"C\" scripts also run")
        );
    }

    #[test]
    fn whitespace_is_no_script() {
        let scripts = Scripts {
            before: " \n".into(),
            after: "test(\"ok\", true);".into(),
        };
        assert_eq!(runs(&scripts), "Runs a script after each response");
        assert_eq!(runs(&Scripts::default()), "");
    }
}
