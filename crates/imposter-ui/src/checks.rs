//! Checks in the UI: the request's Tests tab, where checks are edited, and the response's
//! Tests tab, where their results are shown.

use egui::Ui;
use imposter_app::core::check::{Check, CheckResult, Op, Verdict};
use imposter_app::core::select::Selector;

use crate::theme::Tone;
use crate::widgets::{self, VarUi, name, remove_button, tone_text};

/// The kinds of selector, as the source picker offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Status,
    Header,
    Json,
    Body,
    Duration,
    Size,
}

impl Source {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Status, "Status"),
        (Self::Header, "Header"),
        (Self::Json, "JSON path"),
        (Self::Body, "Body"),
        (Self::Duration, "Duration (ms)"),
        (Self::Size, "Size (bytes)"),
    ];

    fn of(selector: &Selector) -> Self {
        match selector {
            Selector::Status => Self::Status,
            Selector::Header(_) => Self::Header,
            Selector::Json(_) => Self::Json,
            Selector::Body => Self::Body,
            Selector::Duration => Self::Duration,
            Selector::Size => Self::Size,
        }
    }

    fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(source, _)| *source == self)
            .map_or("", |(_, label)| label)
    }

    /// The selector of this kind, keeping a header name or path typed for the other kind.
    fn selector(self, argument: String) -> Selector {
        match self {
            Self::Status => Selector::Status,
            Self::Header => Selector::Header(argument),
            Self::Json => Selector::Json(argument),
            Self::Body => Selector::Body,
            Self::Duration => Selector::Duration,
            Self::Size => Selector::Size,
        }
    }
}

const SOURCE_WIDTH: f32 = 120.0;
const OP_WIDTH: f32 = 130.0;
const MIN_FIELD: f32 = 90.0;

/// The check a new row starts with: the most common expectation.
pub fn new_check() -> Check {
    Check::new(Selector::Status, Op::Equals, "200")
}

/// The check "Add check" in the JSON viewer creates for the value on a line.
pub fn check_for(path: &str, value: Option<&str>) -> Check {
    let selector = Selector::Json(path.to_owned());
    match value {
        Some(value) if !value.starts_with(['{', '[']) => Check::new(selector, Op::Equals, value),
        _ => Check::new(selector, Op::Exists, ""),
    }
}

/// The request's checks: enabled, what to read, operator, expected value, remove. Returns
/// whether anything changed.
pub fn editor(ui: &mut Ui, checks: &mut Vec<Check>, vars: &mut VarUi) -> bool {
    let mut changed = false;
    let mut remove = None;
    let height = ui.spacing().interact_size.y;
    let spacing = ui.spacing().item_spacing.x;
    // Checkbox, both pickers and ×, with the spacing between six cells; the text fields share
    // the rest, so a row fits on one line when the pane allows it and wraps otherwise.
    let fixed = 24.0 + SOURCE_WIDTH + OP_WIDTH + 24.0 + 6.0 * spacing + 16.0;
    let free = (ui.available_width() - fixed).max(2.0 * MIN_FIELD);
    if checks.is_empty() {
        ui.label(tone_text(
            "Checks run after every response: status, headers, JSON paths, duration or size.",
            Tone::Muted,
        ));
    }
    for (index, check) in checks.iter_mut().enumerate() {
        let number = index + 1;
        ui.horizontal_wrapped(|ui| {
            let enabled = ui.add_sized(
                [24.0, height],
                egui::Checkbox::without_text(&mut check.enabled),
            );
            name(&enabled, &format!("Check {number} enabled"));
            changed |= enabled.changed();

            let mut source = Source::of(&check.selector);
            let combo = egui::ComboBox::from_id_salt(("check source", index))
                .width(SOURCE_WIDTH)
                .selected_text(source.label())
                .show_ui(ui, |ui| {
                    for (choice, label) in Source::ALL {
                        ui.selectable_value(&mut source, choice, label);
                    }
                });
            name(&combo.response, &format!("Check {number} source"));
            if source != Source::of(&check.selector) {
                let argument = match &mut check.selector {
                    Selector::Header(text) | Selector::Json(text) => std::mem::take(text),
                    _ => String::new(),
                };
                let argument = if argument.is_empty() && source == Source::Json {
                    "$.".to_owned()
                } else {
                    argument
                };
                check.selector = source.selector(argument);
                changed = true;
            }
            let takes_argument = matches!(check.selector, Selector::Header(_) | Selector::Json(_));
            let argument_width = if !takes_argument {
                0.0
            } else if check.op.takes_value() {
                (free * 0.55).max(MIN_FIELD)
            } else {
                free - spacing
            };
            // A path field adds a cell, and with it one more spacing.
            let expected_width =
                (free - argument_width - if takes_argument { spacing } else { 0.0 }).max(MIN_FIELD);
            match &mut check.selector {
                Selector::Header(argument) | Selector::Json(argument) => {
                    let what = if source == Source::Header {
                        "header name"
                    } else {
                        "JSON path"
                    };
                    let field = ui.add(
                        egui::TextEdit::singleline(argument)
                            .hint_text(if source == Source::Header {
                                "Content-Type"
                            } else {
                                "$.items[0].id"
                            })
                            .desired_width(argument_width),
                    );
                    name(&field, &format!("Check {number} {what}"));
                    changed |= field.changed();
                }
                _ => {}
            }

            let combo = egui::ComboBox::from_id_salt(("check op", index))
                .width(OP_WIDTH)
                .selected_text(check.op.label())
                .show_ui(ui, |ui| {
                    for op in Op::ALL {
                        changed |= ui.selectable_value(&mut check.op, op, op.label()).changed();
                    }
                });
            name(&combo.response, &format!("Check {number} operator"));
            if check.op.takes_value() {
                changed |= widgets::var_edit(
                    ui,
                    &mut check.expected,
                    &format!("Check {number} expected value"),
                    expected_width,
                    false,
                    vars,
                )
                .changed();
            }
            let what = format!("check {number}");
            if remove_button(ui, egui::Id::unique(("remove check", index)), &what, height) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        checks.remove(index);
        changed = true;
    }
    if ui.button("Add check").clicked() {
        checks.push(new_check());
        changed = true;
    }
    changed
}

/// "3/4 passed", toned by the worst result: a failure, then an undecided check.
pub fn summary(results: &[CheckResult]) -> (String, Tone) {
    let passed = results
        .iter()
        .filter(|r| r.verdict == Verdict::Passed)
        .count();
    let tone = if results.iter().any(|r| r.verdict == Verdict::Failed) {
        Tone::Error
    } else if passed < results.len() {
        Tone::Warning
    } else {
        Tone::Ok
    };
    (format!("{passed}/{} passed", results.len()), tone)
}

fn verdict_text(verdict: &Verdict) -> (&'static str, Tone) {
    match verdict {
        Verdict::Passed => ("Passed", Tone::Ok),
        Verdict::Failed => ("Failed", Tone::Error),
        Verdict::Unchecked(_) => ("Not checked", Tone::Warning),
    }
}

/// The results of a response's checks, one line each with what was read or why it could
/// not be decided.
/// The summary is the status line's badge, so it is not repeated here.
pub fn results(ui: &mut Ui, results: &[CheckResult]) {
    egui::ScrollArea::vertical()
        .id_salt("check results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for result in results {
                ui.horizontal_wrapped(|ui| {
                    let (word, tone) = verdict_text(&result.verdict);
                    ui.label(tone_text(word, tone).strong());
                    ui.label(result.check.describe());
                    let detail = match (&result.verdict, &result.actual) {
                        (Verdict::Unchecked(reason), _) => reason.clone(),
                        (_, Some(actual)) => format!("got {actual}"),
                        (_, None) => "nothing there".to_owned(),
                    };
                    ui.label(tone_text(detail, Tone::Muted));
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(verdict: Verdict) -> CheckResult {
        CheckResult {
            check: new_check(),
            verdict,
            actual: None,
        }
    }

    #[test]
    fn summaries_count_passes_and_take_the_worst_tone() {
        let passed = result(Verdict::Passed);
        let failed = result(Verdict::Failed);
        let unchecked = result(Verdict::Unchecked("why".into()));
        assert_eq!(
            summary(std::slice::from_ref(&passed)),
            ("1/1 passed".into(), Tone::Ok)
        );
        assert_eq!(
            summary(&[passed.clone(), unchecked.clone()]),
            ("1/2 passed".into(), Tone::Warning)
        );
        assert_eq!(
            summary(&[unchecked, failed, passed]),
            ("1/3 passed".into(), Tone::Error)
        );
    }

    #[test]
    fn checks_from_a_value_compare_scalars_and_test_containers_for_presence() {
        assert_eq!(
            check_for("$.id", Some("7")),
            Check::new(Selector::Json("$.id".into()), Op::Equals, "7")
        );
        assert_eq!(check_for("$.name", Some("Ann")).describe(), "$.name = Ann");
        assert_eq!(check_for("$.tags", Some("[\n  1\n]")).op, Op::Exists);
        assert_eq!(check_for("$.meta", None).op, Op::Exists);
    }

    #[test]
    fn switching_the_source_keeps_a_typed_argument() {
        assert_eq!(
            Source::Json.selector("X-Id".into()),
            Selector::Json("X-Id".into())
        );
        for (source, _) in Source::ALL {
            assert_eq!(Source::of(&source.selector(String::new())), source);
        }
    }
}
