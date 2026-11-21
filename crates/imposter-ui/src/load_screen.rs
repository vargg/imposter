//! The Load screen: one request sent many times, apart from Send (`imposter_app::load`).
//! The target and the plan come first, so Start and Stop never move; the summary of the
//! run shown follows, with this session's earlier runs a menu away.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use egui::{RichText, Ui};
use imposter_app::core::ids::RequestId;
use imposter_app::core::load::{Bucket, LoadPlan, Summary};
use imposter_app::load::{Progress, Report};
use imposter_app::runtime::TaskId;
use imposter_app::store::collections::IndexedRequest;
use serde::{Deserialize, Serialize};

use crate::app::{Action, State};
use crate::icons::Icon;
use crate::theme::{self, Tone, palette};
use crate::widgets::{self, Variant, name, tone_text};

/// How many finished runs the screen keeps, newest first; nothing is stored.
pub const RUNS_KEPT: usize = 10;

/// What the user does on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadAction {
    /// Pick the request to run, from the palette.
    Choose,
    /// Run it: `confirmed` once a red environment's question was answered.
    Start {
        confirmed: bool,
    },
    Stop,
    /// Show the `n`th kept run (0 is the newest).
    Show(usize),
}

/// The plan as typed; stored in UI state, so the last values come back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoadForm {
    pub requests: String,
    pub concurrency: String,
    /// Seconds; empty for none.
    pub time_limit: String,
    /// Seconds; empty for none.
    pub ramp_up: String,
    pub keep_alive: bool,
}

impl Default for LoadForm {
    fn default() -> Self {
        Self {
            requests: "100".into(),
            concurrency: "10".into(),
            time_limit: String::new(),
            ramp_up: String::new(),
            keep_alive: true,
        }
    }
}

impl LoadForm {
    /// The plan, or what to fix in words.
    pub fn plan(&self) -> Result<LoadPlan, String> {
        fn whole<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
            text.trim()
                .parse()
                .map_err(|_| format!("{what} must be a whole number"))
        }
        fn seconds(text: &str, what: &str) -> Result<Option<Duration>, String> {
            let text = text.trim();
            if text.is_empty() {
                return Ok(None);
            }
            whole::<u64>(text, what).map(|s| Some(Duration::from_secs(s)))
        }
        let requests = whole(&self.requests, "Requests")?;
        let concurrency = whole(&self.concurrency, "Concurrency")?;
        let time_limit = seconds(&self.time_limit, "The time limit")?;
        let ramp_up = seconds(&self.ramp_up, "Ramp-up")?.unwrap_or_default();
        LoadPlan::new(requests, concurrency, time_limit, ramp_up, self.keep_alive)
            .map_err(|e| e.to_string())
    }
}

/// The request a run sends, as the screen names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: RequestId,
    pub method: String,
    pub name: String,
    /// "Shop API / Users": its collection and folders.
    pub place: String,
}

impl Target {
    pub fn from_index(request: &IndexedRequest) -> Self {
        let mut place = vec![request.collection.clone()];
        place.extend(request.folders.iter().cloned());
        Self {
            id: request.id,
            method: request.method.clone(),
            name: request.name.clone(),
            place: place.join(" / "),
        }
    }

    /// The saved request `id` as the sidebar's tree knows it.
    pub fn from_tree(state: &State, id: RequestId) -> Option<Self> {
        let tree = state.tree.as_ref()?;
        let request = tree.request(id)?;
        let path = tree.request_path(id)?;
        let place = path
            .rsplit_once(" / ")
            .map_or(String::new(), |(place, _)| place.to_owned());
        Some(Self {
            id,
            method: request.method.clone(),
            name: request.name.clone(),
            place,
        })
    }
}

/// A run in progress.
pub struct Running {
    pub task: TaskId,
    pub progress: Arc<Progress>,
    /// egui time when it started, for the elapsed time and rate.
    pub started: f64,
    pub plan: LoadPlan,
    pub target: Target,
    pub stopping: bool,
}

/// A finished run, kept for this session.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub target: Target,
    pub plan: LoadPlan,
    pub report: Report,
    /// Local time it ended: "14:02".
    pub when: String,
}

#[derive(Default)]
pub struct LoadScreen {
    pub target: Option<Target>,
    pub form: LoadForm,
    pub run: Option<Running>,
    /// Newest first, at most [`RUNS_KEPT`].
    pub runs: Vec<Finished>,
    pub shown: usize,
    /// Why the last run could not start.
    pub error: Option<String>,
    pub(crate) generation: u64,
}

impl LoadScreen {
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Keep a finished run, newest first, and show it.
    pub fn finished(&mut self, finished: Finished) {
        self.runs.insert(0, finished);
        self.runs.truncate(RUNS_KEPT);
        self.shown = 0;
    }
}

// ---- words ---------------------------------------------------------------------------------

/// "0.4 ms", "21 ms", "1.25 s".
pub fn duration_text(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1000.0;
    if millis < 1.0 {
        format!("{millis:.1} ms")
    } else if millis < 1000.0 {
        format!("{millis:.0} ms")
    } else {
        format!("{:.2} s", millis / 1000.0)
    }
}

/// "412 req/s", "8.5 req/s".
pub fn rate_text(per_second: f64) -> String {
    if per_second < 10.0 {
        format!("{per_second:.1} req/s")
    } else {
        format!("{per_second:.0} req/s")
    }
}

/// "1.2 MB/s".
pub fn bytes_rate_text(per_second: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a display value
    let bytes = per_second.max(0.0) as usize;
    format!("{}/s", crate::viewer::format_size(bytes))
}

/// "0%", "0.2%", "12%", "100%".
pub fn percent_text(share: f64) -> String {
    let percent = share * 100.0;
    if percent == 0.0 || percent >= 10.0 {
        format!("{percent:.0}%")
    } else {
        format!("{percent:.1}%")
    }
}

/// "1,000".
pub fn count_text(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// How the run ended, when it did not send everything.
pub fn ending(finished: &Finished) -> Option<String> {
    let done = count_text(finished.report.summary.completed);
    if finished.report.stopped {
        Some(format!(
            "Stopped after {done} of {} requests",
            count_text(u64::from(finished.plan.requests()))
        ))
    } else if finished.report.timed_out {
        Some(format!("Time limit reached after {done} requests"))
    } else {
        None
    }
}

/// "<5 ms", "<1 s", "10 s or more".
pub fn bucket_label(bucket: &Bucket) -> String {
    if bucket.upper == Duration::MAX {
        "10 s or more".to_owned()
    } else {
        format!("<{}", duration_text(bucket.upper))
    }
}

/// "640 / 1,000 · 412 req/s · 3.2 s" while running.
pub fn progress_text(done: u64, planned: u32, elapsed: f64) -> String {
    #[allow(clippy::cast_precision_loss)] // counts far below 2^52
    let rate = if elapsed > 0.0 {
        done as f64 / elapsed
    } else {
        0.0
    };
    format!(
        "{} / {} · {} · {elapsed:.1} s",
        count_text(done),
        count_text(u64::from(planned)),
        rate_text(rate)
    )
}

// ---- drawing -------------------------------------------------------------------------------

const LABEL_COLUMN: f32 = 128.0;
const FIELD_WIDTH: f32 = 96.0;

/// What a load run does with the request's scripts, which differs from a Send.
pub const SCRIPTS_RUN: &str =
    "This request's scripts run for every request; variables they set are not written";

pub fn draw(state: &mut State, ui: &mut Ui) {
    // Opened with nothing chosen, the request in the editor is the natural target.
    if state.load.target.is_none()
        && let Some(id) = state.editor.as_ref().and_then(|p| p.editor.id())
    {
        state.load.target = Target::from_tree(state, id);
    }
    let now = ui.input(|i| i.time);
    let mut actions = Vec::new();
    let plan = state.load.form.plan();
    let mut execution = None;
    let scripted = state
        .editor
        .as_ref()
        .filter(|p| p.editor.id() == state.load.target.as_ref().map(|t| t.id))
        .is_some_and(|p| !p.editor.scripts.is_empty());
    widgets::screen_toolbar(
        ui,
        "Load",
        |ui| target_row(ui, &state.load, scripted, &mut actions),
        |ui| execution = controls(ui, &state.load, &plan),
    );
    actions.extend(execution.map(Action::Load));
    egui::ScrollArea::vertical()
        .id_salt("load screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::scroll_view_bounds(ui);
            widgets::tool_columns(
                ui,
                &mut state.load,
                |ui, screen| {
                    plan_form(ui, screen);
                    if let Some(problem) = screen.error.as_deref().or_else(|| {
                        plan.as_ref()
                            .err()
                            .filter(|_| screen.run.is_none())
                            .map(String::as_str)
                    }) {
                        ui.add(egui::Label::new(tone_text(problem, Tone::Error)).wrap());
                    }
                },
                |ui, screen| {
                    if let Some(run) = &screen.run {
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                        progress(ui, run, now);
                        ui.add_space(theme::spacing().sm);
                    }
                    if let Some(finished) = screen.runs.get(screen.shown) {
                        result(ui, screen, finished, &mut actions);
                    }
                },
            );
        });
    state.actions.extend(actions);
}

fn target_row(ui: &mut Ui, screen: &LoadScreen, scripted: bool, actions: &mut Vec<Action>) {
    let (label, path) = screen.target.as_ref().map_or_else(
        || ("Choose request…".to_owned(), String::new()),
        |target| {
            let label = format!("{} {}", target.method, target.name);
            let path = if target.place.is_empty() {
                label.clone()
            } else {
                format!("{} / {label}", target.place)
            };
            (label, path)
        },
    );
    let button = ui
        .add_enabled_ui(screen.run.is_none(), |ui| {
            widgets::button(ui, Variant::Secondary, None, &label)
        })
        .inner;
    name(&button, "Choose the request to load-test");
    let hint = if scripted {
        format!("{path}\n{SCRIPTS_RUN}")
    } else {
        path
    };
    let button = if hint.is_empty() {
        button
    } else {
        button
            .ctx
            .accesskit_node_builder(button.id, |node| node.set_description(hint.as_str()));
        button.on_hover_text(hint)
    };
    if button.clicked() {
        actions.push(Action::Load(LoadAction::Choose));
    }
}

fn plan_form(ui: &mut Ui, screen: &mut LoadScreen) {
    let editable = screen.run.is_none();
    let form = &mut screen.form;
    let spacing = theme::spacing();
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("load plan")
            .num_columns(2)
            .spacing([spacing.md, spacing.sm])
            .min_col_width(LABEL_COLUMN)
            .show(ui, |ui| {
                for (label, spoken, value, hint, help) in [
                    (
                        "Total requests",
                        "Total requests",
                        &mut form.requests,
                        "",
                        "",
                    ),
                    (
                        "Concurrency",
                        "Concurrency",
                        &mut form.concurrency,
                        "",
                        "Requests sent at once",
                    ),
                    (
                        "Time limit",
                        "Time limit in seconds",
                        &mut form.time_limit,
                        "none",
                        "Leave empty to run without a time limit",
                    ),
                    (
                        "Ramp-up",
                        "Ramp-up in seconds",
                        &mut form.ramp_up,
                        "none",
                        "Time to start all workers. Leave empty to start them at once",
                    ),
                ] {
                    if help.is_empty() {
                        ui.label(theme::body(label));
                    } else {
                        widgets::label_with_help(ui, label, help);
                    }
                    ui.horizontal(|ui| {
                        let field = ui.add_sized(
                            [FIELD_WIDTH, spacing.control],
                            egui::TextEdit::singleline(value).hint_text(hint),
                        );
                        name(&field, spoken);
                        if !hint.is_empty() {
                            ui.label(theme::meta("s").color(palette().text_muted));
                        }
                    });
                    ui.end_row();
                }
                widgets::label_with_help(ui, "Keep-alive", "Reuse connections between requests");
                let toggle = widgets::toggle(ui, &mut form.keep_alive);
                name(&toggle, "Reuse connections");
                ui.end_row();
            });
    });
}

fn controls(
    ui: &mut Ui,
    screen: &LoadScreen,
    plan: &Result<LoadPlan, String>,
) -> Option<LoadAction> {
    if let Some(run) = &screen.run {
        let stop = ui
            .add_enabled_ui(!run.stopping, |ui| {
                widgets::button(ui, Variant::Danger, Some(Icon::Stop), "Stop")
            })
            .inner;
        name(&stop, "Stop the load test");
        return stop.clicked().then_some(LoadAction::Stop);
    }
    let ready = screen.target.is_some() && plan.is_ok();
    let start = ui
        .add_enabled_ui(ready, |ui| {
            widgets::button(ui, Variant::Primary, Some(Icon::Play), "Start")
        })
        .inner
        .on_disabled_hover_text(match (&screen.target, plan) {
            (None, _) => "Choose a request first",
            (_, Err(reason)) => reason,
            _ => "",
        });
    name(&start, "Start the load test");
    start
        .clicked()
        .then_some(LoadAction::Start { confirmed: false })
}

fn progress(ui: &mut Ui, run: &Running, now: f64) {
    let elapsed = (now - run.started).max(0.0);
    let done = run.progress.done();
    let planned = run.plan.requests();
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // a fraction
    let mut fraction = (done as f64 / f64::from(planned)) as f32;
    if let Some(limit) = run.plan.time_limit() {
        #[allow(clippy::cast_possible_truncation)] // a fraction
        let by_time = (elapsed / limit.as_secs_f64()) as f32;
        fraction = fraction.max(by_time);
    }
    let mut text = progress_text(done, planned, elapsed);
    if run.progress.errors() > 0 {
        let _ = write!(text, " · {} errors", count_text(run.progress.errors()));
    }
    if run.stopping {
        text = format!("Stopping… {text}");
    }
    let bar = ui.add(
        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
            .text(text.clone())
            .desired_width(ui.available_width().min(520.0)),
    );
    name(&bar, &format!("Load test progress: {text}"));
}

fn result(ui: &mut Ui, screen: &LoadScreen, finished: &Finished, actions: &mut Vec<Action>) {
    let summary = &finished.report.summary;
    ui.horizontal(|ui| {
        ui.label(theme::section("Result"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if screen.runs.len() > 1 {
                runs_menu(ui, screen, actions);
            }
            let caption = format!(
                "{} {} · {} × {} · {}",
                finished.target.method,
                finished.target.name,
                count_text(u64::from(finished.plan.requests())),
                finished.plan.concurrency(),
                finished.when
            );
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(tone_text(&caption, Tone::Muted)).truncate())
                    .on_hover_text(caption);
            });
        });
    });
    if let Some(ending) = ending(finished) {
        ui.label(tone_text(ending, Tone::Warning));
    }
    // The first script error of the run, so the failing line shows without digging.
    if let Some(error) = &finished.report.script_error {
        ui.add(egui::Label::new(tone_text(error.as_str(), Tone::Error)).wrap());
    }
    let spacing = theme::spacing();
    stat_tiles(ui, summary);
    ui.add_space(spacing.xs);
    outcomes(ui, summary);
    if let Some(total) = summary.total {
        ui.add_space(spacing.sm);
        let mut rows = vec![
            ["", "min", "mean", "p50", "p90", "p95", "p99", "max"]
                .map(str::to_owned)
                .to_vec(),
        ];
        for (label, latency) in [("Total", Some(total)), ("First byte", summary.first_byte)] {
            if let Some(l) = latency {
                let mut row = vec![label.to_owned()];
                row.extend([l.min, l.mean, l.p50, l.p90, l.p95, l.p99, l.max].map(duration_text));
                rows.push(row);
            }
        }
        number_table(ui, "Latency table", &rows, true);
        ui.add_space(spacing.sm);
        ui.label(theme::section("Served within"));
        number_table(
            ui,
            "Served within table",
            &[
                summary
                    .served_within
                    .iter()
                    .map(|(p, _)| format!("{p}%"))
                    .collect(),
                summary
                    .served_within
                    .iter()
                    .map(|(_, d)| duration_text(*d))
                    .collect(),
            ],
            false,
        );
        ui.add_space(spacing.sm);
        histogram(ui, &summary.histogram);
    }
}

fn stat_tiles(ui: &mut Ui, summary: &Summary) {
    let latency = |select: fn(imposter_app::core::load::Latency) -> Duration| {
        summary
            .total
            .map(select)
            .map_or_else(|| "—".into(), duration_text)
    };
    let rate = rate_text(summary.per_second);
    let tiles = [
        (
            "Requests/s",
            rate.trim_end_matches(" req/s").to_owned(),
            Tone::Normal,
        ),
        (
            "Throughput",
            bytes_rate_text(summary.bytes_per_second),
            Tone::Normal,
        ),
        (
            "Errors",
            percent_text(summary.error_rate()),
            if summary.errors > 0 {
                Tone::Error
            } else {
                Tone::Normal
            },
        ),
        ("p50", latency(|l| l.p50), Tone::Normal),
        ("p95", latency(|l| l.p95), Tone::Normal),
        ("p99", latency(|l| l.p99), Tone::Normal),
    ];
    let tiles = tiles.map(|(caption, value, tone)| {
        let number = RichText::new(value)
            .font(egui::FontId::monospace(theme::PAGE_TITLE))
            .color(palette().tone(tone));
        (theme::meta(caption).color(palette().text_muted), number)
    });
    let spacing = theme::spacing();
    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(6, 4))
        .stroke(egui::Stroke::new(1.0, palette().border));
    let margin = frame.total_margin().sum().x;
    let widest = tiles
        .iter()
        .flat_map(|(caption, number)| [caption, number])
        .map(|text| widgets::text_width(ui, text.clone()))
        .fold(64.0_f32, f32::max);
    let available = ui.available_width();
    let single_row = (available - 5.0 * spacing.xs) / 6.0 - margin;
    let width = single_row
        .clamp(widest, widest + spacing.xl)
        .min((available - margin).max(0.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .accessibility_role(egui::Role::Group)
            .accessibility_label("Summary metrics"),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = spacing.xs;
                for (caption, number) in tiles {
                    frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            ui.add(egui::Label::new(number).truncate());
                            ui.add(egui::Label::new(caption).truncate());
                        });
                    });
                }
            })
        },
    );
}

fn number_table(ui: &mut Ui, name: &str, rows: &[Vec<String>], row_labels: bool) {
    #[allow(clippy::cast_precision_loss)] // the latency and percentile tables have 8 and 9 columns
    let columns = rows[0].len() as f32;
    let cell_text = |row: usize, column: usize, text: &str| {
        if row == 0 || (row_labels && column == 0) {
            theme::meta(text).color(palette().text_muted)
        } else {
            theme::meta(text).monospace()
        }
    };
    let widest = rows
        .iter()
        .enumerate()
        .flat_map(|(row, cells)| {
            cells
                .iter()
                .enumerate()
                .map(move |(column, text)| cell_text(row, column, text))
        })
        .map(|text| widgets::text_width(ui, text))
        .fold(0.0_f32, f32::max);
    let spacing = theme::spacing();
    let gap = spacing.md;
    let available = (ui.available_width() - (columns - 1.0) * gap) / columns;
    let width = available.clamp(0.0, widest + spacing.xl);
    let size = egui::vec2(width, ui.text_style_height(&egui::TextStyle::Body));
    ui.scope_builder(
        egui::UiBuilder::new()
            .accessibility_role(egui::Role::Group)
            .accessibility_label(name),
        |ui| {
            for (row, cells) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (column, text) in cells.iter().enumerate() {
                        // Set the layout of the cell so it places the label itself at the edge.
                        let layout = if row_labels && column == 0 {
                            egui::Layout::left_to_right(egui::Align::Center)
                        } else {
                            egui::Layout::right_to_left(egui::Align::Center)
                        };
                        ui.allocate_ui_with_layout(size, layout, |ui| {
                            ui.set_min_size(size);
                            if !text.is_empty() {
                                ui.add(egui::Label::new(cell_text(row, column, text)).truncate());
                            }
                        });
                    }
                });
            }
        },
    );
}

/// Statuses and errors as pills with counts, then the checks.
fn outcomes(ui: &mut Ui, summary: &Summary) {
    ui.horizontal_wrapped(|ui| {
        for (status, count) in &summary.statuses {
            let tone = match status {
                200..=299 => Tone::Ok,
                300..=399 => Tone::Warning,
                _ => Tone::Error,
            };
            widgets::pill(
                ui,
                &format!("{status} ×{}", count_text(*count)),
                tone,
                160.0,
            );
        }
        for (kind, count) in &summary.error_kinds {
            let title = crate::viewer::failure_title(kind);
            widgets::pill(
                ui,
                &format!("{title} ×{}", count_text(*count)),
                Tone::Error,
                240.0,
            );
        }
        let checks = summary.checks;
        if checks.passed + checks.failed + checks.unchecked > 0 {
            let mut text = format!("Checks ✓{}", count_text(checks.passed));
            if checks.failed > 0 {
                let _ = write!(text, " ✗{}", count_text(checks.failed));
            }
            if checks.unchecked > 0 {
                let _ = write!(text, " ?{}", count_text(checks.unchecked));
            }
            let tone = if checks.failed > 0 {
                Tone::Error
            } else {
                Tone::Normal
            };
            ui.label(tone_text(text, tone));
        }
    });
}

/// Bars for the latency buckets, each labelled with its bound under it; screen readers
/// hear the counts.
fn histogram(ui: &mut Ui, buckets: &[Bucket]) {
    if buckets.is_empty() {
        return;
    }
    // Bars stay narrow when there are few, so two buckets do not span the pane.
    #[allow(clippy::cast_precision_loss)] // a handful of buckets
    let width = ui
        .available_width()
        .min(560.0)
        .min(56.0 * buckets.len() as f32);
    let height = 72.0;
    let font = egui::FontId::proportional(theme::META);
    let labels: Vec<_> = buckets
        .iter()
        .map(|bucket| {
            if bucket.upper == Duration::MAX {
                "≥10 s".into()
            } else {
                bucket_label(bucket)
            }
        })
        .collect();
    #[allow(clippy::cast_precision_loss)] // a handful of buckets
    let slot = width / buckets.len() as f32;
    let stagger = labels
        .iter()
        .any(|label| widgets::text_width(ui, RichText::new(label).font(font.clone())) + 2.0 > slot);
    let line = theme::META + theme::spacing().xs;
    let label_height = if stagger { 2.0 * line } else { line };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height + label_height),
        egui::Sense::hover(),
    );
    let description: Vec<String> = buckets
        .iter()
        .map(|b| format!("{}: {}", bucket_label(b), count_text(b.count)))
        .collect();
    name(
        &response,
        &format!("Latency histogram: {}", description.join(", ")),
    );
    let most = buckets.iter().map(|b| b.count).max().unwrap_or(1).max(1);
    let painter = ui.painter_at(rect);
    for (index, bucket) in buckets.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)] // display fractions
        let share = bucket.count as f32 / most as f32;
        #[allow(clippy::cast_precision_loss)] // a handful of buckets
        let left = rect.left() + slot * index as f32;
        let bar = egui::Rect::from_min_max(
            egui::pos2(left + 2.0, rect.top() + height * (1.0 - share)),
            egui::pos2(left + slot - 2.0, rect.top() + height),
        );
        if bucket.count > 0 {
            painter.rect_filled(bar, 2.0, palette().text_muted);
        }
        let label = &labels[index];
        let half = widgets::text_width(ui, RichText::new(label).font(font.clone())) / 2.0;
        let x = (left + slot / 2.0).clamp(
            rect.left() + half,
            (rect.right() - half).max(rect.left() + half),
        );
        let offset = if stagger && index % 2 == 1 { line } else { 0.0 };
        painter.text(
            egui::pos2(x, rect.top() + height + 2.0 + offset),
            egui::Align2::CENTER_TOP,
            label,
            font.clone(),
            palette().text_muted,
        );
    }
    painter.hline(
        rect.x_range(),
        rect.top() + height,
        egui::Stroke::new(1.0, palette().field_border),
    );
}

fn runs_menu(ui: &mut Ui, screen: &LoadScreen, actions: &mut Vec<Action>) {
    let menu = widgets::combo("load runs")
        .selected_text("Runs")
        .show_ui(ui, |ui| {
            for (index, run) in screen.runs.iter().enumerate() {
                let label = format!(
                    "{} · {} {} · {} × {}",
                    run.when,
                    run.target.method,
                    run.target.name,
                    count_text(u64::from(run.plan.requests())),
                    run.plan.concurrency()
                );
                if widgets::option(ui, index == screen.shown, label).clicked() {
                    actions.push(Action::Load(LoadAction::Show(index)));
                }
            }
        });
    name(&menu.response, "Earlier load test runs");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_reads_as_a_plan_or_says_what_to_fix() {
        let plan = LoadForm::default().plan().unwrap();
        assert_eq!(
            (plan.requests(), plan.concurrency(), plan.time_limit()),
            (100, 10, None)
        );
        let form = LoadForm {
            time_limit: " 30 ".into(),
            ramp_up: "5".into(),
            ..LoadForm::default()
        };
        let plan = form.plan().unwrap();
        assert_eq!(plan.time_limit(), Some(Duration::from_secs(30)));
        assert_eq!(plan.ramp_up(), Duration::from_secs(5));
        let bad = |form: LoadForm| form.plan().unwrap_err();
        assert_eq!(
            bad(LoadForm {
                requests: "ten".into(),
                ..LoadForm::default()
            }),
            "Requests must be a whole number"
        );
        assert_eq!(
            bad(LoadForm {
                concurrency: "1.5".into(),
                ..LoadForm::default()
            }),
            "Concurrency must be a whole number"
        );
        assert!(
            bad(LoadForm {
                concurrency: "500".into(),
                ..LoadForm::default()
            })
            .starts_with("Concurrency must be between")
        );
        assert_eq!(
            bad(LoadForm {
                time_limit: "x".into(),
                ..LoadForm::default()
            }),
            "The time limit must be a whole number"
        );
    }

    #[test]
    fn numbers_read_plainly() {
        assert_eq!(duration_text(Duration::from_micros(400)), "0.4 ms");
        assert_eq!(duration_text(Duration::from_millis(21)), "21 ms");
        assert_eq!(duration_text(Duration::from_millis(1250)), "1.25 s");
        assert_eq!(rate_text(412.4), "412 req/s");
        assert_eq!(rate_text(8.46), "8.5 req/s");
        assert_eq!(bytes_rate_text(1.5 * 1024.0 * 1024.0), "1.5 MB/s");
        assert_eq!(percent_text(0.0), "0%");
        assert_eq!(percent_text(0.002), "0.2%");
        assert_eq!(percent_text(0.125), "12%");
        assert_eq!(percent_text(1.0), "100%");
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(1000), "1,000");
        assert_eq!(count_text(1_234_567), "1,234,567");
        assert_eq!(
            progress_text(640, 1000, 2.0),
            "640 / 1,000 · 320 req/s · 2.0 s"
        );
        assert_eq!(progress_text(0, 10, 0.0), "0 / 10 · 0.0 req/s · 0.0 s");
        assert_eq!(
            bucket_label(&Bucket {
                upper: Duration::from_millis(5),
                count: 1
            }),
            "<5 ms"
        );
        assert_eq!(
            bucket_label(&Bucket {
                upper: Duration::MAX,
                count: 1
            }),
            "10 s or more"
        );
    }
}
