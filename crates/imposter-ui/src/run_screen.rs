//! The Run screen: a collection or a folder sent as one run (`imposter_app::runner`), as
//! `imposter run` does. The target and Run share a toolbar with Stop in the same place; the run's requests follow as they end, and the chosen one's response shows
//! in the response viewer. Values a request captures reach the requests after it and are
//! never written to variables. The last runs of the session are a menu away.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::Ui;
use imposter_app::core::check::Verdict;
use imposter_app::core::ids::{CollectionId, EnvironmentId, FolderId};
use imposter_app::core::run_result::RunOutcome;
use imposter_app::runner::Step;
use imposter_app::runtime::TaskId;

use crate::app::{Action, ResponsePane, State};
use crate::icons::Icon;
use crate::theme::{self, Tone, palette};
use crate::widgets::{self, ListEnd, ListIcon, Variant, name, tone_text};

/// How many finished runs the screen keeps, newest first; nothing is stored.
pub const RUNS_KEPT: usize = 10;

/// Space on each side of the line between the list and the details.
const GAP: f32 = 8.0;

/// What a run sends: a collection, or one of its folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTarget {
    pub collection: CollectionId,
    pub folder: Option<FolderId>,
    /// "Shop API / Orders", as the breadcrumb reads.
    pub label: String,
}

impl RunTarget {
    /// The open collection, or `folder` of it, as the sidebar's tree knows it.
    pub fn from_tree(state: &State, folder: Option<FolderId>) -> Option<Self> {
        let tree = state.tree.as_ref()?;
        let collection = &tree.source.collection;
        let label = match folder {
            None => collection.name.clone(),
            Some(id) => {
                let place = tree
                    .destinations()
                    .into_iter()
                    .find(|(_, d)| *d == Some(id))?
                    .0;
                format!("{} / {place}", collection.name)
            }
        };
        Some(Self {
            collection: collection.id,
            folder,
            label,
        })
    }
}

/// Which environment a run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvironmentChoice {
    /// The app's active one, whichever it is at Start.
    #[default]
    Active,
    /// This one, or none (globals and collection only).
    Chosen(Option<EnvironmentId>),
}

/// What the user does on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    Choose(RunTarget),
    /// Send it: `confirmed` once a red environment's question was answered.
    Start {
        confirmed: bool,
    },
    Stop,
    /// Show the `n`th kept run (0 is the newest).
    Show(usize),
    /// Show the `n`th request of the run shown.
    Select(usize),
}

/// A run in progress. The runner pushes each request as it ends into `shared`; the screen
/// moves them into `steps` every frame.
pub struct Running {
    pub task: TaskId,
    pub shared: Arc<Mutex<Vec<Step>>>,
    pub steps: Vec<Step>,
    pub target: RunTarget,
    pub environment: String,
    pub stopping: bool,
}

impl Running {
    /// Take the requests that ended since the last frame.
    pub fn drain(&mut self) {
        if let Ok(mut new) = self.shared.lock() {
            self.steps.append(&mut new);
        }
    }
}

/// A finished run, kept for this session.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedRun {
    pub target: RunTarget,
    pub environment: String,
    pub steps: Vec<Step>,
    /// Local time it ended: "14:02".
    pub when: String,
    /// Why it could not start, when it sent nothing.
    pub error: Option<String>,
}

#[derive(Default)]
pub struct RunScreen {
    pub target: Option<RunTarget>,
    pub environment: EnvironmentChoice,
    pub keep_going: bool,
    pub run: Option<Running>,
    /// Newest first, at most [`RUNS_KEPT`].
    pub runs: Vec<FinishedRun>,
    pub shown: usize,
    /// The request of the shown run whose response is open.
    pub selected: Option<usize>,
    pub pane: ResponsePane,
    pub(crate) generation: u64,
}

impl RunScreen {
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Keep a finished run, newest first, and show it.
    pub fn finished(&mut self, finished: FinishedRun) {
        self.runs.insert(0, finished);
        self.runs.truncate(RUNS_KEPT);
        self.shown = 0;
    }

    /// The requests on screen: the live run's, or the shown finished run's.
    pub fn steps(&self) -> &[Step] {
        match (&self.run, self.runs.get(self.shown)) {
            (Some(run), _) => &run.steps,
            (None, Some(finished)) => &finished.steps,
            (None, None) => &[],
        }
    }

    /// Open the `index`th request's response in the viewer.
    pub fn select(&mut self, index: usize) {
        let step = self.steps().get(index).cloned();
        self.selected = Some(index);
        self.pane = ResponsePane::default();
        if let Some(Step::Ran { record, .. }) = step {
            self.pane.show(record.response.clone());
            self.pane.sent = Some(crate::viewer::sent_request(&record).as_text());
        }
    }
}

// ---- words ---------------------------------------------------------------------------------

/// A request's outcome in a word, with its tone.
pub fn outcome_word(outcome: Option<RunOutcome>) -> (&'static str, Tone) {
    match outcome {
        Some(RunOutcome::Passed) => ("Passed", Tone::Ok),
        Some(RunOutcome::NoChecks) => ("No checks", Tone::Muted),
        Some(RunOutcome::Failed) => ("Failed", Tone::Error),
        Some(RunOutcome::NotChecked) => ("Not checked", Tone::Warning),
        Some(RunOutcome::NotSent) => ("Not sent", Tone::Error),
        Some(RunOutcome::TransportError) => ("No response", Tone::Error),
        Some(RunOutcome::ScriptFailed) => ("Script failed", Tone::Error),
        Some(RunOutcome::Cancelled) => ("Cancelled", Tone::Muted),
        None => ("Skipped", Tone::Muted),
    }
}

fn path_of(step: &Step) -> &str {
    match step {
        Step::Ran { path, .. } | Step::NotSent { path, .. } | Step::Skipped { path, .. } => path,
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

/// The run's outcome and a line about it, by the command line's rule
/// (`run_report::outcome`).
pub fn summary(steps: &[Step]) -> (RunOutcome, String) {
    let mut outcomes: Vec<Option<RunOutcome>> = steps.iter().map(Step::outcome).collect();
    let skipped = outcomes.iter().filter(|o| o.is_none()).count();
    if steps.iter().any(|step| matches!(step, Step::Skipped { reason, .. } if reason == imposter_app::runner::SKIPPED_AFTER_CANCEL)) {
        outcomes.push(Some(RunOutcome::Cancelled));
    }
    let mut checks_passed = 0;
    let mut checks_failed = 0;
    for step in steps {
        if let Step::Ran { record, .. } = step {
            for check in &record.response.checks {
                match check.verdict {
                    Verdict::Passed => checks_passed += 1,
                    Verdict::Failed => checks_failed += 1,
                    Verdict::Unchecked(_) => {}
                }
            }
            for test in record.response.script.iter().flat_map(|s| &s.tests) {
                if test.passed {
                    checks_passed += 1;
                } else {
                    checks_failed += 1;
                }
            }
        }
    }
    let outcome = imposter_app::core::run_report::outcome(&outcomes, checks_passed);
    let ran = steps.len() - skipped;
    let mut parts = vec![plural(ran, "request") + " ran"];
    if checks_passed + checks_failed > 0 {
        parts.push(format!(
            "{checks_passed} of {} checks passed",
            checks_passed + checks_failed
        ));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    let word = match outcome {
        RunOutcome::NotSent if steps.is_empty() => "Nothing to run",
        RunOutcome::NoChecks => "No checks",
        other => outcome_word(Some(other)).0,
    };
    (outcome, format!("{word} · {}", parts.join(" · ")))
}

// ---- drawing -------------------------------------------------------------------------------

pub fn draw(state: &mut State, ui: &mut Ui) {
    // Opened with nothing chosen, the open collection is the natural target.
    if state.run.target.is_none() && state.run.run.is_none() {
        state.run.target = RunTarget::from_tree(state, None);
    }
    if let Some(run) = &mut state.run.run {
        run.drain();
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let mut actions = Vec::new();
    let mut execution = None;
    widgets::screen_toolbar(
        ui,
        "Run",
        |ui| target_picker(ui, state, &mut actions),
        |ui| execution = controls(ui, &state.run),
    );
    actions.extend(execution.map(Action::Run));
    widgets::tool_columns(ui, state, configuration, |ui, state| {
        results(ui, &mut state.run, &mut actions);
    });
    state.actions.extend(actions);
}

fn configuration(ui: &mut Ui, state: &mut State) {
    let mut environment = state.run.environment;
    ui.add_enabled_ui(state.run.run.is_none(), |ui| {
        environment_picker(ui, state, &mut environment);
        ui.horizontal(|ui| {
            widgets::label_with_help(ui, "Keep going after a failure",
                "Continue to later requests after a failure. Captured values reach later requests in this run and stay within the run.");
            let toggle = widgets::toggle(ui, &mut state.run.keep_going);
            name(&toggle, "Keep going after a failure");
        });
    });
    state.run.environment = environment;
    if let Some(note) = unsaved_note(state) {
        ui.add(egui::Label::new(tone_text(note, Tone::Muted)).wrap());
    }
}

fn results(ui: &mut Ui, screen: &mut RunScreen, actions: &mut Vec<Action>) {
    result_line(ui, screen, actions);
    ui.add_space(theme::spacing().xs);
    // Fixed rectangles, as History's split: a row nested in a horizontal layout measured its
    // width differently from the response tools, which then overlapped the tabs.
    if screen.steps().is_empty() {
        return;
    }
    let area = ui.available_rect_before_wrap();
    let list_width = (area.width() * 0.4).clamp(200.0, 420.0);
    let list = egui::Rect::from_min_size(area.min, egui::vec2(list_width, area.height()));
    let details_rect = egui::Rect::from_min_max(
        egui::pos2(list.right() + 2.0 * GAP + 1.0, area.top()),
        area.max,
    );
    ui.painter().vline(
        list.right() + GAP,
        area.y_range(),
        egui::Stroke::new(1.0, palette().border),
    );
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(list).id_salt("run list"),
        |ui| steps_list(ui, screen, actions),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(details_rect)
            .id_salt("run details"),
        |ui| details(ui, screen, actions),
    );
    ui.allocate_rect(area, egui::Sense::hover());
}

/// When the open request has unsaved edits and the run will send it, say which version.
fn unsaved_note(state: &State) -> Option<String> {
    let pane = state.editor.as_ref()?;
    let id = pane.editor.id()?;
    let saved = state.saver.state(id) == imposter_app::saver::State::Saved;
    if saved || state.run.run.is_some() {
        return None;
    }
    let target = state.run.target.as_ref()?;
    let tree = state.tree.as_ref()?;
    if tree.source.collection.id != target.collection {
        return None;
    }
    let inside = match target.folder {
        None => tree.request(id).is_some(),
        Some(folder) => tree.requests_under(folder).contains(&id),
    };
    inside.then(|| {
        format!(
            "\"{}\" has unsaved changes; the run sends its saved version.",
            pane.editor.name()
        )
    })
}

fn environment_label(state: &State, choice: EnvironmentChoice) -> String {
    let id = match choice {
        EnvironmentChoice::Active => state.active_environment,
        EnvironmentChoice::Chosen(id) => id,
    };
    id.and_then(|id| state.environments.iter().find(|e| e.id == id))
        .map_or_else(|| "No environment".to_owned(), |e| e.name.clone())
}

fn target_picker(ui: &mut Ui, state: &State, actions: &mut Vec<Action>) {
    ui.add_enabled_ui(state.run.run.is_none(), |ui| {
        let selected = state
            .run
            .target
            .as_ref()
            .map_or_else(|| "Choose…".to_owned(), |t| t.label.clone());
        let target_width = ui.available_width().min(280.0);
        let picker = widgets::toolbar_combo(ui, "run target", target_width, &selected, |ui| {
            let targets: Vec<RunTarget> = match state.tree.as_ref() {
                Some(tree) => std::iter::once(None)
                    .chain(tree.destinations().into_iter().skip(1).map(|(_, f)| f))
                    .filter_map(|folder| RunTarget::from_tree(state, folder))
                    .collect(),
                None => Vec::new(),
            };
            for target in targets {
                let chosen = state.run.target.as_ref() == Some(&target);
                if widgets::option(ui, chosen, &target.label).clicked() {
                    actions.push(Action::Run(RunAction::Choose(target)));
                }
            }
        });
        name(&picker, "What to run");
        picker.on_hover_text(selected);
    });
}

fn environment_picker(ui: &mut Ui, state: &State, selected: &mut EnvironmentChoice) {
    ui.horizontal(|ui| {
        ui.label(theme::body("Environment"));
        let shown = environment_label(state, state.run.environment);
        let width = ui.available_width().min(160.0);
        let picker = widgets::toolbar_combo(ui, "run environment", width, &shown, |ui| {
            let mut choices = vec![(
                format!(
                    "{} (active)",
                    environment_label(state, EnvironmentChoice::Active)
                ),
                EnvironmentChoice::Active,
            )];
            choices.push(("No environment".to_owned(), EnvironmentChoice::Chosen(None)));
            for environment in &state.environments {
                choices.push((
                    environment.name.clone(),
                    EnvironmentChoice::Chosen(Some(environment.id)),
                ));
            }
            for (label, choice) in choices {
                if widgets::option(ui, *selected == choice, label).clicked() {
                    *selected = choice;
                }
            }
        });
        name(&picker, "Environment for the run");
        picker.on_hover_text(shown);
    });
}

fn controls(ui: &mut Ui, screen: &RunScreen) -> Option<RunAction> {
    if let Some(run) = &screen.run {
        let stop = ui
            .add_enabled_ui(!run.stopping, |ui| {
                widgets::button(ui, Variant::Danger, Some(Icon::Stop), "Stop")
            })
            .inner;
        name(&stop, "Stop the run");
        return stop.clicked().then_some(RunAction::Stop);
    }
    let start = ui
        .add_enabled_ui(screen.target.is_some(), |ui| {
            widgets::button(ui, Variant::Primary, Some(Icon::Run), "Run")
        })
        .inner
        .on_disabled_hover_text("Open a collection first");
    name(&start, "Start the run");
    start
        .clicked()
        .then_some(RunAction::Start { confirmed: false })
}

/// The shown run's outcome and totals, or its progress, and the earlier runs.
fn result_line(ui: &mut Ui, screen: &RunScreen, actions: &mut Vec<Action>) {
    if let Some(run) = &screen.run {
        ui.horizontal(|ui| {
            ui.spinner();
            let ended = plural(run.steps.len(), "request");
            let text = if run.stopping {
                format!("Stopping… {ended} ended")
            } else {
                format!(
                    "Running {} with {} · {ended} ended",
                    run.target.label, run.environment
                )
            };
            ui.add(egui::Label::new(tone_text(&text, Tone::Muted)).truncate())
                .on_hover_text(text);
        });
    } else if let Some(finished) = screen.runs.get(screen.shown) {
        ui.horizontal(|ui| {
            if screen.runs.len() > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    runs_menu(ui, screen, actions);
                    run_summary(ui, finished);
                });
            } else {
                run_summary(ui, finished);
            }
        });
        if let Some(error) = &finished.error {
            ui.add(egui::Label::new(tone_text(error, Tone::Error)).wrap());
        }
    } else {
        ui.label(tone_text("No run yet", Tone::Muted));
    }
}

fn run_summary(ui: &mut Ui, finished: &FinishedRun) {
    let (text, tone) = if finished.error.is_some() {
        ("Not run".to_owned(), Tone::Error)
    } else {
        let (outcome, text) = summary(&finished.steps);
        (text, crate::viewer::outcome_tone(outcome))
    };
    let caption = format!(
        "{} · {} · {}",
        finished.target.label, finished.environment, finished.when
    );
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let width = widgets::text_width(ui, theme::body(&text)).min(ui.available_width() * 0.6);
        ui.add_sized(
            [width, theme::spacing().control],
            egui::Label::new(tone_text(&text, tone)).truncate(),
        )
        .on_hover_text(text);
        ui.add(egui::Label::new(tone_text(&caption, Tone::Muted)).truncate())
            .on_hover_text(caption);
    });
}

fn runs_menu(ui: &mut Ui, screen: &RunScreen, actions: &mut Vec<Action>) {
    let menu = widgets::combo("earlier runs")
        .selected_text("Runs")
        .show_ui(ui, |ui| {
            for (index, run) in screen.runs.iter().enumerate() {
                let label = format!("{} · {} · {}", run.when, run.target.label, run.environment);
                if widgets::option(ui, index == screen.shown, label).clicked() {
                    actions.push(Action::Run(RunAction::Show(index)));
                }
            }
        });
    name(&menu.response, "Earlier runs");
}

fn steps_list(ui: &mut Ui, screen: &RunScreen, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .id_salt("run steps")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, step) in screen.steps().iter().enumerate() {
                let (word, tone) = outcome_word(step.outcome());
                let time = match step {
                    Step::Ran { record, .. } => {
                        crate::viewer::format_duration(record.response.elapsed)
                    }
                    Step::NotSent { .. } | Step::Skipped { .. } => String::new(),
                };
                let path = path_of(step);
                let short = path.split_once('/').map_or(path, |(_, rest)| rest);
                let icon = match tone {
                    Tone::Ok => ListIcon::Status(Icon::Passed, tone),
                    Tone::Error => ListIcon::Status(Icon::Failed, tone),
                    Tone::Warning => ListIcon::Status(Icon::Warning, tone),
                    _ => ListIcon::Dot(None),
                };
                let mut caption = short.to_owned();
                if !matches!(
                    step.outcome(),
                    Some(RunOutcome::Passed | RunOutcome::Failed)
                ) {
                    caption.push_str(" · ");
                    caption.push_str(word);
                }
                if !time.is_empty() {
                    caption.push_str(" · ");
                    caption.push_str(&time);
                }
                let hint = format!("{word} · {path} {time}");
                let (row, _) = widgets::list_row(
                    ui,
                    &caption,
                    icon,
                    screen.selected == Some(index),
                    ListEnd::None,
                    Some(&hint),
                );
                name(&row, &format!("Run step {word} {path}"));
                if row.clicked() {
                    actions.push(Action::Run(RunAction::Select(index)));
                }
            }
        });
}

fn details(ui: &mut Ui, screen: &mut RunScreen, actions: &mut Vec<Action>) {
    let Some(index) = screen.selected else {
        if !screen.steps().is_empty() {
            ui.label(tone_text(
                "Select a request to see its response.",
                Tone::Muted,
            ));
        }
        return;
    };
    match screen.steps().get(index).cloned() {
        Some(Step::Ran { path, notes, .. }) => {
            ui.add(egui::Label::new(theme::section(&path)).truncate())
                .on_hover_text(&path);
            for note in notes {
                ui.label(tone_text(note, Tone::Warning));
            }
            if let Some(crate::workspace::PaneAction::SaveBody) =
                crate::workspace::response_body(ui, &mut screen.pane, "run", true, false)
            {
                actions.push(crate::workspace::save_body_action(&screen.pane));
            }
        }
        Some(Step::NotSent { path, error, .. }) => {
            ui.add(egui::Label::new(theme::section(&path)).truncate())
                .on_hover_text(&path);
            ui.add(egui::Label::new(tone_text(format!("Not sent: {error}"), Tone::Error)).wrap());
        }
        Some(Step::Skipped { path, reason }) => {
            ui.add(egui::Label::new(theme::section(&path)).truncate())
                .on_hover_text(&path);
            ui.label(tone_text(format!("Skipped: {reason}."), Tone::Muted));
        }
        None => {}
    }
}
