//! Checks in the UI: the request's Tests tab, where checks are edited, and the response's
//! Tests tab, where their results are shown.

use egui::Ui;
use imposter_app::core::capture::{Capture, Captured, Taken};
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
const VARIABLE_WIDTH: f32 = 140.0;
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

/// A capture of the value at `path`, into a variable named after its last member name
/// (`$.data.items[0].id` becomes `id`), or `value` when the path names none.
pub fn capture_for(path: &str) -> Capture {
    Capture::new(
        last_member(path).unwrap_or("value"),
        Selector::Json(path.to_owned()),
    )
}

/// The last member name in a path written by `select::member`, if it is a usable variable
/// name.
fn last_member(path: &str) -> Option<&str> {
    let mut last = None;
    let mut rest = path.strip_prefix('$')?;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            last = Some(&after[..end]);
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix("['") {
            // Quoted names may hold anything; only plain ones make variable names.
            let end = after.find("']")?;
            let name = &after[..end];
            last = (!name.is_empty() && !name.contains(['\\', '{', '}'])).then_some(name);
            rest = &after[end + 2..];
        } else {
            let after = rest.strip_prefix('[')?;
            rest = &after[after.find(']')? + 1..];
        }
    }
    last
}

/// The source picker and, for a header or a JSON path, its field. `row` names the cells
/// ("Check 2"); `salt` keeps the picker's id apart from other rows'.
fn selector_cells(
    ui: &mut Ui,
    salt: (&str, usize),
    row: &str,
    selector: &mut Selector,
    argument_width: f32,
) -> bool {
    let mut changed = false;
    let mut source = Source::of(selector);
    let combo = egui::ComboBox::from_id_salt(salt)
        .width(SOURCE_WIDTH)
        .selected_text(source.label())
        .show_ui(ui, |ui| {
            for (choice, label) in Source::ALL {
                ui.selectable_value(&mut source, choice, label);
            }
        });
    name(&combo.response, &format!("{row} source"));
    if source != Source::of(selector) {
        let argument = match selector {
            Selector::Header(text) | Selector::Json(text) => std::mem::take(text),
            _ => String::new(),
        };
        let argument = if argument.is_empty() && source == Source::Json {
            "$.".to_owned()
        } else {
            argument
        };
        *selector = source.selector(argument);
        changed = true;
    }
    if let Selector::Header(argument) | Selector::Json(argument) = selector {
        let header = source == Source::Header;
        let field = ui.add(
            egui::TextEdit::singleline(argument)
                .hint_text(if header {
                    "Content-Type"
                } else {
                    "$.items[0].id"
                })
                .desired_width(argument_width),
        );
        let what = if header { "header name" } else { "JSON path" };
        name(&field, &format!("{row} {what}"));
        changed |= field.changed();
    }
    changed
}

fn takes_argument(selector: &Selector) -> bool {
    matches!(selector, Selector::Header(_) | Selector::Json(_))
}

/// The request's Tests tab: its checks, then the variables it sets from each response.
/// Returns whether anything changed.
pub fn tests_tab(
    ui: &mut Ui,
    checks: &mut Vec<Check>,
    captures: &mut Vec<Capture>,
    vars: &mut VarUi,
) -> bool {
    ui.label(tone_text("Checks", Tone::Muted));
    let mut changed = checks_table(ui, checks, vars);
    ui.add_space(12.0);
    ui.label(tone_text("Set variables from the response", Tone::Muted));
    changed |= captures_table(ui, captures);
    changed
}

/// Checks: enabled, what to read, operator, expected value, remove.
fn checks_table(ui: &mut Ui, checks: &mut Vec<Check>, vars: &mut VarUi) -> bool {
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
        let row = format!("Check {}", index + 1);
        ui.horizontal_wrapped(|ui| {
            let enabled = ui.add_sized(
                [24.0, height],
                egui::Checkbox::without_text(&mut check.enabled),
            );
            name(&enabled, &format!("{row} enabled"));
            changed |= enabled.changed();
            let argument = takes_argument(&check.selector);
            let argument_width = if !argument {
                0.0
            } else if check.op.takes_value() {
                (free * 0.55).max(MIN_FIELD)
            } else {
                free - spacing
            };
            changed |= selector_cells(
                ui,
                ("check source", index),
                &row,
                &mut check.selector,
                argument_width,
            );
            // A path field adds a cell, and with it one more spacing.
            let argument_width = if takes_argument(&check.selector) {
                argument_width.max(MIN_FIELD) + spacing
            } else {
                0.0
            };
            let expected_width = (free - argument_width).max(MIN_FIELD);
            let combo = egui::ComboBox::from_id_salt(("check op", index))
                .width(OP_WIDTH)
                .selected_text(check.op.label())
                .show_ui(ui, |ui| {
                    for op in Op::ALL {
                        changed |= ui.selectable_value(&mut check.op, op, op.label()).changed();
                    }
                });
            name(&combo.response, &format!("{row} operator"));
            if check.op.takes_value() {
                changed |= widgets::var_edit(
                    ui,
                    &mut check.expected,
                    &format!("{row} expected value"),
                    expected_width,
                    false,
                    vars,
                )
                .changed();
            }
            let what = row.to_lowercase();
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

/// Captures: enabled, variable name, what to read, remove.
fn captures_table(ui: &mut Ui, captures: &mut Vec<Capture>) -> bool {
    let mut changed = false;
    let mut remove = None;
    let height = ui.spacing().interact_size.y;
    let spacing = ui.spacing().item_spacing.x;
    let fixed = 24.0 + VARIABLE_WIDTH + SOURCE_WIDTH + 24.0 + 6.0 * spacing + 16.0;
    let free = (ui.available_width() - fixed).max(MIN_FIELD);
    if captures.is_empty() {
        ui.label(tone_text(
            "Values taken from each response go into variables: the scope that defines the \
             name, or the active environment.",
            Tone::Muted,
        ));
    }
    for (index, capture) in captures.iter_mut().enumerate() {
        let row = format!("Capture {}", index + 1);
        ui.horizontal_wrapped(|ui| {
            let enabled = ui.add_sized(
                [24.0, height],
                egui::Checkbox::without_text(&mut capture.enabled),
            );
            name(&enabled, &format!("{row} enabled"));
            changed |= enabled.changed();
            let field = ui.add(
                egui::TextEdit::singleline(&mut capture.variable)
                    .hint_text("variable")
                    .desired_width(VARIABLE_WIDTH),
            );
            name(&field, &format!("{row} variable"));
            changed |= field.changed();
            changed |= selector_cells(
                ui,
                ("capture source", index),
                &row,
                &mut capture.selector,
                free,
            );
            let what = row.to_lowercase();
            if remove_button(
                ui,
                egui::Id::unique(("remove capture", index)),
                &what,
                height,
            ) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        captures.remove(index);
        changed = true;
    }
    if ui.button("Add variable").clicked() {
        captures.push(Capture::new("", Selector::Json("$.".into())));
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

/// Captured values are shortened in the results: a token can be long.
const SHOWN_VALUE: usize = 80;

/// The results of a response's checks, one line each with what was read or why it could not
/// be decided, then what its captures took. The checks' summary is the status line's badge,
/// so it is not repeated here.
pub fn results(ui: &mut Ui, results: &[CheckResult], captured: &[Captured]) {
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
            if !captured.is_empty() {
                if !results.is_empty() {
                    ui.add_space(8.0);
                }
                for capture in captured {
                    ui.horizontal_wrapped(|ui| {
                        let (word, tone, detail) = captured_text(capture);
                        ui.label(tone_text(word, tone).strong());
                        ui.label(&capture.variable);
                        ui.label(tone_text(detail, Tone::Muted));
                    });
                }
            }
        });
}

fn captured_text(capture: &Captured) -> (&'static str, Tone, String) {
    match &capture.taken {
        Taken::Value(value) => {
            let shown = match value.char_indices().nth(SHOWN_VALUE) {
                Some((cut, _)) => format!("{}…", &value[..cut]),
                None => value.clone(),
            };
            ("Set", Tone::Ok, format!("in {}: {shown}", capture.target))
        }
        Taken::Missing => (
            "Kept",
            Tone::Warning,
            "nothing there; the variable keeps its value".to_owned(),
        ),
        Taken::Unreadable(reason) => ("Kept", Tone::Warning, reason.clone()),
    }
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
    fn captures_from_a_path_are_named_after_its_last_member() {
        assert_eq!(
            capture_for("$.data.items[0].id"),
            Capture::new("id", Selector::Json("$.data.items[0].id".into()))
        );
        assert_eq!(capture_for("$.token").variable, "token");
        assert_eq!(capture_for("$.users[3]").variable, "users");
        assert_eq!(capture_for("$['a b']").variable, "a b");
        assert_eq!(capture_for("$['x\\'y']").variable, "value");
        assert_eq!(capture_for("$").variable, "value");
        assert_eq!(capture_for("$[0][1]").variable, "value");
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
