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
use crate::theme::{Tone, method_color, palette};
use crate::widgets::{self, name, tone_text};

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

/// The run in one line: "412 req/s · 1.2 MB/s · errors 0.2% · p50 21 ms · p95 52 ms ·
/// p99 95 ms". Without responses the latencies are left out.
pub fn headline(summary: &Summary) -> String {
    let mut parts = vec![
        rate_text(summary.per_second),
        bytes_rate_text(summary.bytes_per_second),
        format!("errors {}", percent_text(summary.error_rate())),
    ];
    if let Some(total) = summary.total {
        parts.push(format!("p50 {}", duration_text(total.p50)));
        parts.push(format!("p95 {}", duration_text(total.p95)));
        parts.push(format!("p99 {}", duration_text(total.p99)));
    }
    parts.join(" · ")
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

const LABEL_COLUMN: f32 = 110.0;
const FIELD_WIDTH: f32 = 90.0;

pub fn draw(state: &mut State, ui: &mut Ui) {
    // Opened with nothing chosen, the request in the editor is the natural target.
    if state.load.target.is_none()
        && let Some(id) = state.editor.as_ref().and_then(|p| p.editor.id())
    {
        state.load.target = Target::from_tree(state, id);
    }
    let now = ui.input(|i| i.time);
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("load screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(760.0));
            target_row(ui, &state.load, &mut actions);
            ui.add_space(12.0);
            plan_form(ui, &mut state.load);
            ui.add_space(12.0);
            controls(ui, &state.load, &mut actions);
            if let Some(run) = &state.load.run {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                ui.add_space(8.0);
                progress(ui, run, now);
            }
            if let Some(finished) = state.load.runs.get(state.load.shown) {
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                result(ui, &state.load, finished, &mut actions);
            }
        });
    state.actions.extend(actions);
}

fn target_row(ui: &mut Ui, screen: &LoadScreen, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let choose = if let Some(target) = &screen.target {
            ui.label(
                RichText::new(&target.method)
                    .monospace()
                    .color(method_color(&target.method)),
            );
            ui.label(RichText::new(&target.name).font(crate::theme::semibold(15.0)));
            if !target.place.is_empty() {
                ui.add(egui::Label::new(tone_text(&target.place, Tone::Muted)).truncate());
            }
            "Change…"
        } else {
            ui.label(tone_text("Choose a request to load-test", Tone::Muted));
            "Choose…"
        };
        let button = ui.add_enabled(screen.run.is_none(), egui::Button::new(choose));
        name(&button, "Choose the request to load-test");
        if button.clicked() {
            actions.push(Action::Load(LoadAction::Choose));
        }
    });
}

fn plan_form(ui: &mut Ui, screen: &mut LoadScreen) {
    let editable = screen.run.is_none();
    let form = &mut screen.form;
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("load plan")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(LABEL_COLUMN)
            .show(ui, |ui| {
                // Spoken names say the unit; "Requests" alone is also the header's tab.
                for (label, spoken, value, help) in [
                    ("Total requests", "Total requests", &mut form.requests, ""),
                    (
                        "Concurrency",
                        "Concurrency",
                        &mut form.concurrency,
                        "requests at once",
                    ),
                    (
                        "Time limit",
                        "Time limit in seconds",
                        &mut form.time_limit,
                        "seconds; empty for none",
                    ),
                    (
                        "Ramp-up",
                        "Ramp-up in seconds",
                        &mut form.ramp_up,
                        "seconds to start all workers; empty starts them at once",
                    ),
                ] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        let field =
                            ui.add(egui::TextEdit::singleline(value).desired_width(FIELD_WIDTH));
                        name(&field, spoken);
                        ui.add(egui::Label::new(tone_text(help, Tone::Muted)).truncate());
                    });
                    ui.end_row();
                }
                ui.label("Keep-alive");
                ui.checkbox(&mut form.keep_alive, "Reuse connections");
                ui.end_row();
            });
    });
}

fn controls(ui: &mut Ui, screen: &LoadScreen, actions: &mut Vec<Action>) {
    let plan = screen.form.plan();
    ui.horizontal(|ui| {
        if let Some(run) = &screen.run {
            let stop = ui.add_enabled(!run.stopping, egui::Button::new("Stop"));
            name(&stop, "Stop the load test");
            if stop.clicked() {
                actions.push(Action::Load(LoadAction::Stop));
            }
        } else {
            let ready = screen.target.is_some() && plan.is_ok();
            let start = ui
                .add_enabled(
                    ready,
                    egui::Button::new(RichText::new("Start").color(palette().on_accent))
                        .fill(palette().accent),
                )
                .on_disabled_hover_text(match (&screen.target, &plan) {
                    (None, _) => "Choose a request first".to_owned(),
                    (_, Err(reason)) => reason.clone(),
                    _ => String::new(),
                });
            name(&start, "Start the load test");
            if start.clicked() {
                actions.push(Action::Load(LoadAction::Start { confirmed: false }));
            }
        }
        // Why it cannot start, in words beside the button rather than only on hover.
        let problem = match (&screen.error, &plan) {
            (Some(error), _) => Some(error.clone()),
            (None, Err(reason)) if screen.run.is_none() => Some(reason.clone()),
            _ => None,
        };
        if let Some(problem) = problem {
            ui.add(egui::Label::new(tone_text(problem, Tone::Error)).wrap());
        }
    });
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
        ui.label(RichText::new("Result").font(crate::theme::semibold(15.0)));
        ui.label(tone_text(
            format!(
                "{} {} · {} × {} · {}",
                finished.target.method,
                finished.target.name,
                count_text(u64::from(finished.plan.requests())),
                finished.plan.concurrency(),
                finished.when
            ),
            Tone::Muted,
        ));
        if screen.runs.len() > 1 {
            runs_menu(ui, screen, actions);
        }
    });
    if let Some(ending) = ending(finished) {
        ui.label(tone_text(ending, Tone::Warning));
    }
    ui.add(egui::Label::new(RichText::new(headline(summary)).strong()).wrap());
    ui.add_space(6.0);
    outcomes(ui, summary);
    if let Some(total) = summary.total {
        ui.add_space(10.0);
        egui::Grid::new("load latency")
            .num_columns(8)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                for column in ["min", "mean", "p50", "p90", "p95", "p99", "max"] {
                    ui.label(tone_text(column, Tone::Muted));
                }
                ui.end_row();
                for (label, latency) in [("Total", Some(total)), ("First byte", summary.first_byte)]
                {
                    let Some(l) = latency else { continue };
                    ui.label(tone_text(label, Tone::Muted));
                    for value in [l.min, l.mean, l.p50, l.p90, l.p95, l.p99, l.max] {
                        ui.label(RichText::new(duration_text(value)).monospace());
                    }
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        ui.label(tone_text("Served within", Tone::Muted));
        egui::Grid::new("load served within")
            .num_columns(summary.served_within.len())
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (percent, _) in &summary.served_within {
                    ui.label(tone_text(format!("{percent}%"), Tone::Muted));
                }
                ui.end_row();
                for (_, within) in &summary.served_within {
                    ui.label(RichText::new(duration_text(*within)).monospace());
                }
                ui.end_row();
            });
        ui.add_space(10.0);
        histogram(ui, &summary.histogram);
    }
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
    let label_height = 16.0;
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
    #[allow(clippy::cast_precision_loss)] // a handful of buckets
    let slot = width / buckets.len() as f32;
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
        painter.text(
            egui::pos2(left + slot / 2.0, rect.top() + height + 2.0),
            egui::Align2::CENTER_TOP,
            bucket_label(bucket),
            egui::FontId::proportional(11.0),
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
    let menu = egui::ComboBox::from_id_salt("load runs")
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
