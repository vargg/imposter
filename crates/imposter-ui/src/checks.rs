//! Checks in the UI: the request's Tests tab, where checks are edited, and the response's
//! Tests tab, where their results are shown.

use egui::Ui;
use imposter_app::core::capture::{Capture, Captured, Taken};
use imposter_app::core::check::{Check, CheckResult, Op, Verdict};
use imposter_app::core::select::Selector;
use imposter_app::core::variables::Snapshot;

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

/// The source picker. `row` names it ("Check 2 source"); `salt` keeps its id apart from
/// other rows'.
fn source_picker(
    ui: &mut Ui,
    salt: (&str, usize),
    row: &str,
    selector: &mut Selector,
    width: f32,
) -> bool {
    let mut source = Source::of(selector);
    let combo = egui::ComboBox::from_id_salt(salt)
        .width(width)
        .selected_text(source.label())
        .show_ui(ui, |ui| {
            for (choice, label) in Source::ALL {
                ui.selectable_value(&mut source, choice, label);
            }
        });
    name(&combo.response, &format!("{row} source"));
    if source == Source::of(selector) {
        return false;
    }
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
    true
}

/// A header name or JSON path field when the source takes one; otherwise an empty cell of
/// the same width when `keep_cell`, so the cells after it stay in their column.
fn argument_field(
    ui: &mut Ui,
    row: &str,
    selector: &mut Selector,
    width: f32,
    keep_cell: bool,
) -> bool {
    let height = ui.spacing().interact_size.y;
    let header = matches!(selector, Selector::Header(_));
    if let Selector::Header(argument) | Selector::Json(argument) = selector {
        let field = ui.add_sized(
            [width, height],
            egui::TextEdit::singleline(argument)
                .desired_width(width)
                .hint_text(if header {
                    "Content-Type"
                } else {
                    "$.items[0].id"
                }),
        );
        let what = if header { "header name" } else { "JSON path" };
        name(&field, &format!("{row} {what}"));
        field.changed()
    } else {
        if keep_cell {
            ui.allocate_space(egui::vec2(width, height));
        }
        false
    }
}

/// How a table's rows are laid out, by the width they have: every cell on one line; what
/// to read on one line and the rest under it; or each part on a line of its own, as in a
/// narrow pane. Text fields keep a usable width in each, and cells keep their columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lines {
    One,
    Two,
    Stacked,
}

/// The check box's cell and the remove button's.
const CHECKBOX: f32 = 24.0;
const REMOVE: f32 = 24.0;
/// The target scope's cell in a capture row.
const TARGET_WIDTH: f32 = 110.0;

/// The cells of a row in order, one line at a time, never wrapping.
fn line<R>(ui: &mut Ui, indent: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        add(ui)
    })
    .inner
}

/// Checks in a fixed grid: enabled, what to read, its path or header (an empty cell for
/// status, body, duration and size), operator, expected value, remove: the Tests tab.
/// Returns whether anything changed.
pub fn checks_table(ui: &mut Ui, checks: &mut Vec<Check>, vars: &mut VarUi) -> bool {
    let mut changed = false;
    let mut remove = None;
    let spacing = ui.spacing().item_spacing.x;
    // A check box is as wide as a control, which a denser layout makes 24.
    let checkbox = CHECKBOX.max(ui.spacing().interact_size.y);
    let height = ui.spacing().interact_size.y;
    let width = ui.available_width();
    let indent = checkbox + spacing;
    let one = checkbox + SOURCE_WIDTH + OP_WIDTH + REMOVE + 5.0 * spacing + 2.0 * MIN_FIELD;
    let two = checkbox + SOURCE_WIDTH + REMOVE + 3.0 * spacing + MIN_FIELD;
    let lines = if width >= one {
        Lines::One
    } else if width >= two {
        Lines::Two
    } else {
        Lines::Stacked
    };
    let rest = width - indent;
    let (source_width, path_width, op_width, expected_width) = match lines {
        Lines::One => {
            // The two text fields share what the fixed cells leave.
            let fields = width - (one - 2.0 * MIN_FIELD);
            let free = (fields / 2.0).floor();
            (SOURCE_WIDTH, free, OP_WIDTH, free)
        }
        Lines::Two => (
            SOURCE_WIDTH,
            width - checkbox - SOURCE_WIDTH - REMOVE - 3.0 * spacing,
            OP_WIDTH,
            (rest - OP_WIDTH - spacing).max(MIN_FIELD),
        ),
        Lines::Stacked => (
            width - checkbox - REMOVE - 2.0 * spacing,
            rest,
            rest.min(OP_WIDTH),
            rest,
        ),
    };
    if checks.is_empty() {
        ui.label(tone_text(
            "Checks run after every response: status, headers, JSON paths, duration or size.",
            Tone::Muted,
        ));
    }
    for (index, check) in checks.iter_mut().enumerate() {
        let row = format!("Check {}", index + 1);
        // The operator and the expected value; `op` and `value` say which to draw, as a
        // stacked row puts each on a line of its own.
        // Read before the row draws; a changed operator shows its value line next frame.
        let takes_value = check.op.takes_value();
        let mut comparison = |ui: &mut Ui, changed: &mut bool, op: bool, value: bool| {
            if op {
                let combo = egui::ComboBox::from_id_salt(("check op", index))
                    .width(op_width)
                    .selected_text(check.op.label())
                    .show_ui(ui, |ui| {
                        for op in Op::ALL {
                            *changed |=
                                ui.selectable_value(&mut check.op, op, op.label()).changed();
                        }
                    });
                name(&combo.response, &format!("{row} operator"));
            }
            if !value {
                return;
            }
            // The last field takes what the line has left, however wide the operator's
            // picker grew for a long label ("does not contain").
            let left = ui.available_width()
                - if lines == Lines::One {
                    REMOVE + ui.spacing().item_spacing.x
                } else {
                    0.0
                };
            let expected_width = left.min(expected_width).max(48.0);
            if check.op.takes_value() {
                *changed |= widgets::var_edit(
                    ui,
                    &mut check.expected,
                    &format!("{row} expected value"),
                    expected_width,
                    false,
                    vars,
                )
                .changed();
            } else if lines == Lines::One {
                ui.allocate_space(egui::vec2(expected_width, height));
            }
        };
        line(ui, 0.0, |ui| {
            let enabled = ui.add_sized(
                [checkbox, height],
                egui::Checkbox::without_text(&mut check.enabled),
            );
            name(&enabled, &format!("{row} enabled"));
            changed |= enabled.changed();
            changed |= source_picker(
                ui,
                ("check source", index),
                &row,
                &mut check.selector,
                source_width,
            );
            if lines != Lines::Stacked {
                changed |= argument_field(ui, &row, &mut check.selector, path_width, true);
            }
            if lines == Lines::One {
                comparison(ui, &mut changed, true, true);
            }
            let what = row.to_lowercase();
            if remove_button(ui, egui::Id::unique(("remove check", index)), &what, height) {
                remove = Some(index);
            }
        });
        if lines == Lines::Stacked && takes_argument(&check.selector) {
            line(ui, indent, |ui| {
                changed |= argument_field(ui, &row, &mut check.selector, path_width, false);
            });
        }
        if lines != Lines::One {
            if lines == Lines::Stacked {
                line(ui, indent, |ui| comparison(ui, &mut changed, true, false));
                if takes_value {
                    line(ui, indent, |ui| comparison(ui, &mut changed, false, true));
                }
            } else {
                line(ui, indent, |ui| comparison(ui, &mut changed, true, true));
            }
            ui.add_space(6.0);
        }
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

fn takes_argument(selector: &Selector) -> bool {
    matches!(selector, Selector::Header(_) | Selector::Json(_))
}

/// Where a capture of `variable` writes, as the executor decides it (`capture::target`):
/// the scope that defines the name, else the active environment, else Global. The short
/// name for the row, and a sentence for its hover text.
fn capture_target(
    snapshot: Option<&Snapshot>,
    fallback: Option<&str>,
    variable: &str,
) -> (String, String) {
    if let Some(definition) = snapshot.and_then(|s| s.effective(variable))
        && crate::vars::Scope::of(&definition.source).is_some()
    {
        return (
            definition.source.label().to_owned(),
            format!(
                "Sets {variable} in {}, which defines it",
                crate::vars::where_defined(&definition.source)
            ),
        );
    }
    match fallback {
        Some(environment) => (
            environment.to_owned(),
            format!(
                "Nothing defines {variable}, so it goes to the active environment, {environment}"
            ),
        ),
        None => (
            "Global".to_owned(),
            format!(
                "Nothing defines {variable} and no environment is active, so it goes to Global"
            ),
        ),
    }
}

/// Captures in a fixed grid: enabled, variable, where it goes, what to read and its path,
/// remove: the Captures tab. `fallback` is the active environment, where a capture of a
/// name nobody defines goes (Global without one). Returns whether anything changed.
pub fn captures_table(
    ui: &mut Ui,
    captures: &mut Vec<Capture>,
    snapshot: Option<&Snapshot>,
    fallback: Option<&str>,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    let spacing = ui.spacing().item_spacing.x;
    // A check box is as wide as a control, which a denser layout makes 24.
    let checkbox = CHECKBOX.max(ui.spacing().interact_size.y);
    let height = ui.spacing().interact_size.y;
    let width = ui.available_width();
    let indent = checkbox + spacing;
    let one = checkbox
        + VARIABLE_WIDTH
        + TARGET_WIDTH
        + SOURCE_WIDTH
        + REMOVE
        + 5.0 * spacing
        + MIN_FIELD;
    let two = checkbox + MIN_FIELD + TARGET_WIDTH + REMOVE + 3.0 * spacing;
    let lines = if width >= one {
        Lines::One
    } else if width >= two {
        Lines::Two
    } else {
        Lines::Stacked
    };
    let rest = width - indent;
    let (variable_width, source_width, path_width) = match lines {
        Lines::One => (VARIABLE_WIDTH, SOURCE_WIDTH, width - one + MIN_FIELD),
        Lines::Two => (
            width - checkbox - TARGET_WIDTH - REMOVE - 3.0 * spacing,
            SOURCE_WIDTH,
            (rest - SOURCE_WIDTH - spacing).max(MIN_FIELD),
        ),
        Lines::Stacked => (width - checkbox - REMOVE - 2.0 * spacing, rest, rest),
    };
    if captures.is_empty() {
        ui.label(tone_text(
            "Values taken from each response go into variables: the scope that defines the \
             name, or the active environment.",
            Tone::Muted,
        ));
    }
    for (index, capture) in captures.iter_mut().enumerate() {
        let row = format!("Capture {}", index + 1);
        let (target, why) = capture_target(snapshot, fallback, capture.variable.trim());
        let target_cell = |ui: &mut Ui, width: f32| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let label = ui
                        .add(
                            egui::Label::new(tone_text(format!("→ {target}"), Tone::Muted))
                                .truncate(),
                        )
                        .on_hover_text(&why);
                    name(&label, &format!("{row} goes to {target}"));
                },
            );
        };
        line(ui, 0.0, |ui| {
            let enabled = ui.add_sized(
                [checkbox, height],
                egui::Checkbox::without_text(&mut capture.enabled),
            );
            name(&enabled, &format!("{row} enabled"));
            changed |= enabled.changed();
            let field = ui.add_sized(
                [variable_width, height],
                egui::TextEdit::singleline(&mut capture.variable).hint_text("variable"),
            );
            name(&field, &format!("{row} variable"));
            changed |= field.changed();
            if lines != Lines::Stacked {
                target_cell(ui, TARGET_WIDTH);
            }
            if lines == Lines::One {
                changed |= source_picker(
                    ui,
                    ("capture source", index),
                    &row,
                    &mut capture.selector,
                    source_width,
                );
                changed |= argument_field(ui, &row, &mut capture.selector, path_width, true);
            }
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
        if lines == Lines::Stacked {
            line(ui, indent, |ui| target_cell(ui, rest));
        }
        if lines != Lines::One {
            line(ui, indent, |ui| {
                changed |= source_picker(
                    ui,
                    ("capture source", index),
                    &row,
                    &mut capture.selector,
                    source_width,
                );
                if lines == Lines::Two {
                    changed |= argument_field(ui, &row, &mut capture.selector, path_width, false);
                }
            });
            if lines == Lines::Stacked && takes_argument(&capture.selector) {
                line(ui, indent, |ui| {
                    changed |= argument_field(ui, &row, &mut capture.selector, path_width, false);
                });
            }
            ui.add_space(6.0);
        }
    }
    if let Some(index) = remove {
        captures.remove(index);
        changed = true;
    }
    if ui.button("Add capture").clicked() {
        captures.push(Capture::new("", Selector::Json("$.".into())));
        changed = true;
    }
    changed
}

/// What a response's checks came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub passed: usize,
    pub failed: usize,
    pub unchecked: usize,
}

impl Tally {
    pub fn of(results: &[CheckResult]) -> Self {
        results.iter().fold(Self::default(), |mut tally, result| {
            match result.verdict {
                Verdict::Passed => tally.passed += 1,
                Verdict::Failed => tally.failed += 1,
                Verdict::Unchecked(_) => tally.unchecked += 1,
            }
            tally
        })
    }

    /// A response's checks and its after-response script's tests together; a script error
    /// counts as a failure, since it never passes silently.
    pub fn of_response(response: &imposter_app::core::execution::Response) -> Self {
        let mut tally = Self::of(&response.checks);
        if let Some(script) = &response.script {
            tally.passed += script.tests.iter().filter(|t| t.passed).count();
            tally.failed += script.failures();
        }
        tally
    }

    /// Whether there is anything to count.
    pub fn is_empty(self) -> bool {
        self.passed + self.failed + self.unchecked == 0
    }

    /// "✓3", "✗1", "?1" (not checked), each in its tone; counts of zero are left out.
    pub fn marks(self) -> Vec<(String, Tone)> {
        [
            ('✓', self.passed, Tone::Ok),
            ('✗', self.failed, Tone::Error),
            ('?', self.unchecked, Tone::Warning),
        ]
        .into_iter()
        .filter(|(_, count, _)| *count > 0)
        .map(|(mark, count, tone)| (format!("{mark}{count}"), tone))
        .collect()
    }

    /// "3 passed, 1 failed, 1 not checked", for hover text and screen readers.
    pub fn words(self) -> String {
        [
            (self.passed, "passed"),
            (self.failed, "failed"),
            (self.unchecked, "not checked"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect::<Vec<_>>()
        .join(", ")
    }
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
/// be decided, failures first and tinted, then what its captures took. The checks' tally is
/// in the status line, so it is not repeated here.
/// What the response's Tests tab lists: the script's error first, then failed checks and
/// tests, the rest, the variables captured, and the live run's console.
pub struct Results<'a> {
    pub checks: &'a [CheckResult],
    pub captured: &'a [Captured],
    pub script: Option<&'a imposter_app::core::execution::ScriptReport>,
    pub console: &'a [String],
}

pub fn results(ui: &mut Ui, all: &Results<'_>) {
    let Results {
        checks: results,
        captured,
        script,
        console,
    } = *all;
    egui::ScrollArea::vertical()
        .id_salt("check results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Failures first, then undecided checks: what needs attention leads.
            let mut ordered: Vec<&CheckResult> = results.iter().collect();
            ordered.sort_by_key(|result| match result.verdict {
                Verdict::Failed => 0,
                Verdict::Unchecked(_) => 1,
                Verdict::Passed => 2,
            });
            if let Some(script) = script.filter(|s| !s.error.is_empty()) {
                tinted(ui, true, |ui| {
                    ui.label(tone_text("Error", Tone::Error).strong());
                    ui.label(script.error.as_str());
                });
            }
            let tests = script.map_or(&[][..], |s| s.tests.as_slice());
            for test in tests.iter().filter(|t| !t.passed) {
                tinted(ui, true, |ui| {
                    ui.label(tone_text("Failed", Tone::Error).strong());
                    ui.label(test.name.as_str());
                    ui.label(tone_text(test.message.as_str(), Tone::Muted));
                });
            }
            for result in ordered {
                let failed = result.verdict == Verdict::Failed;
                let tint = if failed {
                    widgets::error_tint()
                } else {
                    egui::Color32::TRANSPARENT
                };
                egui::Frame::new()
                    .fill(tint)
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(4, 2))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
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
                    });
            }
            for test in tests.iter().filter(|t| t.passed) {
                tinted(ui, false, |ui| {
                    ui.label(tone_text("Passed", Tone::Ok).strong());
                    ui.label(test.name.as_str());
                });
            }
            if !captured.is_empty() {
                if !results.is_empty() || !tests.is_empty() {
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
            if !console.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Console").font(crate::theme::semibold(13.0)));
                ui.label(tone_text("Console output is not kept in History", Tone::Muted).small());
                for line in console {
                    ui.add(egui::Label::new(egui::RichText::new(line.as_str()).monospace()).wrap());
                }
            }
        });
}

/// One result row, on a red tint when it failed.
fn tinted(ui: &mut Ui, failed: bool, add: impl FnOnce(&mut Ui)) {
    let tint = if failed {
        widgets::error_tint()
    } else {
        egui::Color32::TRANSPARENT
    };
    egui::Frame::new()
        .fill(tint)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(add);
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
    fn tallies_count_each_verdict() {
        let passed = result(Verdict::Passed);
        let failed = result(Verdict::Failed);
        let unchecked = result(Verdict::Unchecked("why".into()));
        let one = Tally::of(std::slice::from_ref(&passed));
        assert_eq!(one.marks(), [("✓1".to_owned(), Tone::Ok)]);
        assert_eq!(one.words(), "1 passed");
        let all = Tally::of(&[unchecked, failed, passed.clone(), passed]);
        assert_eq!(
            all.marks(),
            [
                ("✓2".to_owned(), Tone::Ok),
                ("✗1".to_owned(), Tone::Error),
                ("?1".to_owned(), Tone::Warning)
            ]
        );
        assert_eq!(all.words(), "2 passed, 1 failed, 1 not checked");
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
