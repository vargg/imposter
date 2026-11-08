//! The eframe application: owns all UI state and turns user actions into background work.
//!
//! Widgets never touch the database or network. They emit [`Action`]s while drawing; after the
//! frame, [`ImposterApp::handle`] applies them. Background results come back as [`Message`]s,
//! drained at the start of each frame. Results carry the selection generation they were
//! started under, so a late result never replaces another request's state.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use egui::ViewportCommand;
use imposter_app::core::execution::{ExecutionRecord, Outcome as ExchangeOutcome, Response};
use imposter_app::core::ids::{CollectionId, EnvironmentId, ExecutionId, FolderId, RequestId};
use imposter_app::core::model::{EnvColor, FormItem, Request, SavedValues, Variables};
use imposter_app::core::run_result::RunResult;
use imposter_app::core::settings::AppSettings;
use imposter_app::core::variables::{SecretScope, Snapshot};
use imposter_app::executor::{self, ExecuteError, Executed};
use imposter_app::formats::native::NativeDocument;
use imposter_app::runtime::{Bridge, TaskId};
use imposter_app::saver::{Saver, State as SaveState};
use imposter_app::store::collections::{self, Collection, IndexedRequest, Tree};
use imposter_app::store::environments::{self, Environment};
use imposter_app::store::executions::{self, ExecutionSummary};
use imposter_app::store::kv::{self, Table};
use imposter_app::store::{Store, StoreError, StoreHandle, scopes, secrets};
use imposter_app::transfer::{self, ExportScope, ImportKind, ImportPreview};
use imposter_app::{ops, store};
use serde_json::Value;

use crate::dialogs::{ConfirmPurpose, Dialog, GuardChoice, NamePurpose, Outcome};
use crate::editor::{Editor, InvalidOptions, OptionField};
use crate::layout::WorkspaceLayout;
use crate::settings_screen::SettingsForm;
use crate::theme::{self, Tone};
use crate::tree::{NodeId, TreeView};
use crate::vars::{self, Scope};
use crate::widgets::{FileRequest, Inspection, SaveStatus, SaveWord, ViewerState};

/// How long shutdown waits for background work before aborting it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// Autosave waits this long after the last edit.
pub const AUTOSAVE_DELAY: f64 = 0.5;

const LAST_KEY: &str = "last";
const RECENT_KEY: &str = "recent_requests";
const APPEARANCE_KEY: &str = "appearance";
/// The Load screen's last plan.
const LOAD_KEY: &str = "load_form";

/// How the app looks: a UI preference, kept in UI state beside the layout rather than in the
/// application settings, which also govern sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: theme::ThemeChoice,
    pub density: theme::Density,
}
const LAYOUT_KEY: &str = "workspace_layout";
const DRAFT_KEY: &str = "draft";
const WINDOW_KEY: &str = "window";

type Failure = String;

fn failure(error: impl std::fmt::Display) -> Failure {
    error.to_string()
}

/// What startup reads in one database command.
#[derive(Debug, Clone)]
pub struct Startup {
    settings: AppSettings,
    collections: Vec<Collection>,
    environments: Vec<Environment>,
    active_environment: Option<EnvironmentId>,
    last: Last,
    recent: Vec<RequestId>,
    appearance: Appearance,
    layout: Value,
    draft: Option<Request>,
    load_form: crate::load_screen::LoadForm,
}

/// The last selection, restored at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Last {
    pub collection_id: Option<CollectionId>,
    pub request_id: Option<RequestId>,
}

/// The window's last size and position, restored by `main` before the window opens.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

/// Read before the window opens, so blocking is fine: no UI thread is running yet.
#[allow(clippy::disallowed_methods)]
pub fn window_state(store: &StoreHandle) -> Option<WindowState> {
    store
        .call_blocking(|db| kv::get::<WindowState>(db, Table::UiState, WINDOW_KEY))
        .ok()
        .flatten()
}

/// What `--smoke-test` reports: the response status, or why there was none.
pub type SmokeResult = Result<u16, String>;

/// `--smoke-test`: send one request through the real app as soon as it started, then close.
struct SmokeTest {
    url: String,
    sent: bool,
    result: Arc<OnceLock<SmokeResult>>,
}

/// Results of background work, applied on the UI thread.
#[derive(Debug)]
pub enum Message {
    Started(Result<Box<Startup>, Failure>),
    Filtered(PaneKind, Box<Filtered>),
    MocksLoaded(Result<imposter_app::core::mock::MockConfig, Failure>),
    MocksSaved {
        config: imposter_app::core::mock::MockConfig,
        result: Result<(), Failure>,
    },
    MockListening {
        generation: u64,
        address: String,
    },
    /// A request the mock server answered.
    MockLog {
        generation: u64,
        entry: Box<imposter_app::core::mock::MockLogEntry>,
    },
    MockStopped {
        generation: u64,
        result: Result<(), Failure>,
    },
    LoadFinished {
        generation: u64,
        result: Result<imposter_app::load::Report, NotSent>,
    },
    /// A run ended; `Err` when it could not start (its requests could not be read).
    RunFinished {
        generation: u64,
        result: Result<(), String>,
    },
    Collections {
        select: Option<CollectionId>,
        result: Result<Vec<Collection>, Failure>,
    },
    Tree {
        generation: u64,
        result: Result<Option<Tree>, Failure>,
    },
    Opened {
        generation: u64,
        result: Result<Option<Box<Request>>, Failure>,
    },
    Duplicated {
        generation: u64,
        result: Result<Box<Request>, Failure>,
    },
    Created {
        result: Result<Box<Request>, Failure>,
    },
    Saved {
        id: RequestId,
        result: Result<(), Failure>,
    },
    DraftSaved {
        generation: u64,
        result: Result<Box<Request>, Failure>,
    },
    Executed {
        generation: u64,
        result: Result<Box<Executed>, NotSent>,
    },
    Runs {
        generation: u64,
        result: Result<Vec<ExecutionSummary>, Failure>,
    },
    Run {
        generation: u64,
        result: Result<Option<Box<ExecutionRecord>>, Failure>,
    },
    /// A tree or collection change finished; reload what it touched.
    Changed {
        reload_collections: bool,
        result: Result<(), Failure>,
    },
    Snapshot {
        generation: u64,
        result: Result<Box<Snapshot>, Failure>,
    },
    Environments {
        select: Option<Scope>,
        result: Result<(Vec<Environment>, Option<EnvironmentId>), Failure>,
    },
    VariablesLoaded {
        scope: Scope,
        /// Its variables, saved values and secrets (none for a collection).
        result: Result<(Variables, SavedValues, Variables), Failure>,
    },
    ValueChosen {
        scope: Scope,
        name: String,
        result: Result<(), Failure>,
    },
    CapturesWritten {
        scopes: Vec<Scope>,
        names: Vec<String>,
        result: Result<(), Failure>,
    },
    VariablesSaved {
        scope: Scope,
        result: Result<(), Failure>,
    },
    HistoryPage {
        generation: u64,
        result: Result<Vec<ExecutionSummary>, Failure>,
    },
    HistoryUsage(Result<executions::Usage, Failure>),
    HistoryRecord {
        id: ExecutionId,
        result: Result<Option<Box<ExecutionRecord>>, Failure>,
    },
    /// The result file chosen to open in History, none when cancelled.
    ResultPicked(Option<PathBuf>),
    ResultRead {
        file: String,
        result: Result<Box<RunResult>, String>,
    },
    Flushed(Result<(), Failure>),
    /// Background work with nothing to apply (it logs its own failures).
    Done,
    /// Whether a Settings path names a file.
    PathChecked {
        path: String,
        exists: bool,
    },
    FilePicked {
        target: FileRequest,
        path: Option<PathBuf>,
    },
    ImportPicked(Option<PathBuf>),
    ImportDetected {
        path: PathBuf,
        result: Result<ImportKind, Failure>,
    },
    CompanionsPicked {
        path: PathBuf,
        companions: Vec<PathBuf>,
    },
    ImportPreviewed(Result<Box<ImportPreview>, Failure>),
    Imported(Result<CollectionId, Failure>),
    NativeExportPicked {
        scope: Box<ExportScope>,
        path: Option<PathBuf>,
    },
    /// Where to save a response body, if the user chose a place.
    BodySavePicked {
        path: Option<PathBuf>,
        bytes: Vec<u8>,
        cut: bool,
    },
    BodySaved {
        result: Result<PathBuf, String>,
        cut: bool,
    },
    PostmanReady(Result<Box<PostmanExport>, Failure>),
    PostmanExportPicked(Option<PathBuf>),
    Exported(Result<PathBuf, Failure>),
    Curl(Result<(String, bool), Failure>),
    RequestIndex(Result<Vec<IndexedRequest>, Failure>),
    SettingsLoaded(Result<AppSettings, Failure>),
    SettingsSaved {
        form: SettingsForm,
        result: Result<AppSettings, Failure>,
    },
}

/// A Postman export waiting for review of its warnings and a destination.
#[derive(Debug)]
pub struct PostmanExport {
    document: NativeDocument,
    data: Value,
    warnings: Vec<String>,
}

/// A native file dialog. Tests script the answers instead (see
/// [`ImposterApp::script_file_dialogs`]).
#[derive(Debug, Clone)]
enum FileDialog {
    Open {
        title: &'static str,
        filter: Option<(&'static str, &'static [&'static str])>,
    },
    OpenMany {
        title: &'static str,
    },
    Save {
        title: &'static str,
        name: &'static str,
        filter: Option<(&'static str, &'static str)>,
    },
}

/// What the user asked for while drawing a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SelectCollection(CollectionId),
    /// Highlight a tree node without opening anything (a folder becomes the parent for new items).
    Select(NodeId),
    OpenRequest(RequestId),
    DuplicateRequest(RequestId),
    Prompt(Dialog),
    /// Put `node` under `target` (a folder, or the collection root) at `index` among its
    /// new siblings, counted without it; `usize::MAX` puts it last.
    Move {
        node: NodeId,
        target: Option<FolderId>,
        index: usize,
    },
    Save,
    Send,
    PickFile(FileRequest),
    /// Save a response body to a file the user picks; `cut` when it was cut at capture.
    SaveBody {
        bytes: Vec<u8>,
        name: &'static str,
        cut: bool,
    },
    /// Append a check to the open request and show its Tests tab.
    AddCheck(imposter_app::core::check::Check),
    /// Append a capture to the open request and show its Tests tab.
    AddCapture(imposter_app::core::capture::Capture),
    ShowRun(ExecutionId),
    Screen(Screen),
    /// Open a request from any collection, switching to that collection first.
    OpenIn {
        collection: CollectionId,
        request: RequestId,
    },
    ToggleSidebar,
    /// Switch between side by side and stacked.
    ToggleLayout,
    /// Choose and remember the theme and density.
    SetAppearance(Appearance),
    /// A curl command pasted over the whole URL: it replaces the open request's contents.
    PasteCurl(String),
    /// Re-indent a JSON body, its tokens and placeholders as typed.
    FormatBody,
    /// Switch the active execution environment (`None`: globals only).
    SetEnvironment(Option<EnvironmentId>),
    /// Open the Variables screen on the active environment (Global without one).
    ManageEnvironments,
    /// The inspector's "Open definition": edit `name` in `scope`.
    OpenDefinition(Scope, String),
    /// Open the Variables screen on a scope with a new row for an undefined name.
    DefineVariable(Scope, String),
    /// The inspector's saved values: make `value` the current value of `name` in `scope`.
    ChooseValue {
        scope: Scope,
        name: String,
        value: String,
    },
    Variables(VariablesAction),
    History(HistoryAction),
    /// Pick a collection file and review its import.
    Import,
    /// Choose what to export from the selected collection.
    Export,
    /// A Settings screen field changed.
    SettingsEdited,
    /// Find out whether a path typed on the Settings screen names a file.
    CheckPath(String),
    Mock(crate::mock_screen::MockAction),
    Load(crate::load_screen::LoadAction),
    /// Load-test this request: make it the Load screen's target and go there.
    LoadTarget(crate::load_screen::Target),
    /// The same for a request of the open collection's tree.
    LoadRequest(RequestId),
    Run(crate::run_screen::RunAction),
    /// Run this collection or folder: make it the Run screen's target and go there.
    RunTarget(crate::run_screen::RunTarget),
    /// The same for the open collection (`None`) or one of its folders.
    RunFolder(Option<FolderId>),
    /// Copy the open request as a curl command with resolved values.
    CopyCurl,
    /// The editor changed: `url` when the URL itself was edited, `params` for the query rows.
    Edited {
        url: bool,
        params: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Requests,
    Variables,
    History,
    Settings,
    Mocks,
    Load,
    Run,
}

/// The Settings screen: the form, the saved baseline it is compared with, and save state.
#[derive(Debug)]
pub struct SettingsScreen {
    pub form: SettingsForm,
    /// The stored settings, including those the screen does not show.
    base: AppSettings,
    baseline: Option<SettingsForm>,
    /// A problem the save controls show until the next edit; otherwise the state follows
    /// from the form.
    pub status: Option<SaveWord>,
    saving: bool,
    loading: bool,
    /// Saving or discarding resolves an app close.
    closing: bool,
    /// Whether each path typed in a path field names a file; `None` while it is checked.
    pub paths: std::collections::HashMap<String, Option<bool>>,
}

impl Default for SettingsScreen {
    fn default() -> Self {
        let base = AppSettings::default();
        Self {
            form: SettingsForm::new(&base),
            base,
            baseline: None,
            status: None,
            saving: false,
            loading: false,
            closing: false,
            paths: std::collections::HashMap::new(),
        }
    }
}

impl SettingsScreen {
    pub fn is_loaded(&self) -> bool {
        self.baseline.is_some()
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn dirty(&self) -> bool {
        self.baseline.as_ref().is_some_and(|b| *b != self.form)
    }

    fn refresh_status(&mut self) {
        self.status = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    Refresh,
    /// The filter text changed: start again from the newest match.
    Filtered,
    Newer,
    Older,
    OnlyCurrent(bool),
    Select(ExecutionId),
    Restore,
    /// Copy the selected execution's request as curl.
    CopyCurl,
    Clear,
    /// Choose a result file `imposter send` wrote and show it in the details.
    OpenResult,
    CloseResult,
}

/// A result file shown in History's details. It is not stored: closing it, or choosing an
/// execution, puts it away.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedResult {
    pub file: String,
    pub result: RunResult,
}

/// What History's details show; the response first, as it is what a run is looked up for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryTab {
    Request,
    #[default]
    Response,
}

pub const HISTORY_PAGE: u64 = 50;

/// The History screen: pages of executions and the details of the selected one.
#[derive(Default)]
pub struct HistoryScreen {
    pub only_current: bool,
    /// The filter language of the sidebar, with `status:` and `env:`.
    pub filter: String,
    pub offset: u64,
    pub rows: Vec<ExecutionSummary>,
    pub selected: Option<ExecutionId>,
    pub record: Option<ExecutionRecord>,
    /// A result file shown instead of a selected execution.
    pub opened: Option<OpenedResult>,
    /// The selected execution's request as sent.
    pub request: Option<crate::viewer::SentRequest>,
    pub request_tab: SentTab,
    pub request_body_view: ViewerState,
    pub request_headers_view: ViewerState,
    pub response: ResponsePane,
    pub tab: HistoryTab,
    pub status: Option<(String, Tone)>,
    pub loaded: bool,
    /// What history holds now, beside the limits; `None` until first read.
    pub usage: Option<executions::Usage>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariablesAction {
    SelectScope(Scope),
    Save,
    /// Back to the stored rows, after asking.
    Revert,
    /// Remove every row, after asking; an unsaved edit.
    ClearAll,
    Edited,
    NewEnvironment,
    RenameEnvironment(EnvironmentId),
    DeleteEnvironment(EnvironmentId),
}

/// Changes that replace the Variables screen's rows go through its own guard.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VarTransition {
    Scope(Scope),
    NewEnvironment,
    RenameEnvironment(EnvironmentId),
    DeleteEnvironment(EnvironmentId),
    OpenDefinition(Scope, String),
    /// Show the Variables screen on `scope`.
    Open(Scope),
    /// Open `scope` with a new row for `name`, unless it already has one.
    Define(Scope, String),
    Close,
}

/// The Variables screen: one scope's rows, edited independently of requests.
#[derive(Debug, Default)]
pub struct VariablesScreen {
    pub scope: Scope,
    pub rows: Vec<vars::Row>,
    baseline: Vec<vars::Entry>,
    /// The scope's secrets, edited and saved with its variables; a collection has none.
    pub secrets: Vec<vars::Row>,
    secret_baseline: Vec<vars::Entry>,
    /// A problem the save controls show until the next edit; otherwise the state follows
    /// from the rows.
    pub status: Option<SaveWord>,
    pub loaded: bool,
    /// A name to highlight after "Open definition".
    pub highlight: Option<String>,
    /// A name to add as a new row once the scope has loaded ("Define" from a blocked send).
    define: Option<String>,
    pending: Option<VarTransition>,
    saving: bool,
}

impl VariablesScreen {
    pub fn dirty(&self) -> bool {
        self.loaded
            && (vars::entries(&self.rows) != self.baseline
                || vars::entries(&self.secrets) != self.secret_baseline)
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }
}

/// Navigations that replace the editor go through the unsaved-changes guard.
#[derive(Debug, Clone, PartialEq)]
enum Navigation {
    Open(RequestId),
    OpenIn {
        collection: CollectionId,
        request: RequestId,
    },
    Duplicate {
        id: RequestId,
        edited: Option<Box<imposter_app::core::model::RequestTemplate>>,
    },
    SelectCollection(CollectionId),
    CreateRequest {
        folder: Option<FolderId>,
        name: String,
    },
    CreateCollection(String),
    /// Put a request in the editor as an unsaved draft (from history or curl), then show
    /// `status`.
    Restore {
        request: Box<Request>,
        status: &'static str,
        /// The run the draft comes from, shown as its response.
        response: Option<Box<Response>>,
    },
    /// Write a reviewed import, then show the new collection.
    Import {
        preview: Box<ImportPreview>,
        remove_unsupported: bool,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuardStage {
    Asking,
    /// Waiting for the save of this request (or a draft save).
    Saving,
    NamingDraft,
}

#[derive(Debug, Clone, PartialEq)]
struct Guard {
    next: Navigation,
    stage: GuardStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseState {
    Open,
    /// Waiting for background work to finish.
    Draining,
    /// Unsaved changes are being resolved.
    Guarding,
    /// UI state is being written.
    Flushing,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTab {
    Params,
    Headers,
    #[default]
    Body,
    Auth,
    Cookies,
    Settings,
    Docs,
    Tests,
    Captures,
    Scripts,
}

impl EditorTab {
    pub const ALL: [(Self, &'static str); 10] = [
        (Self::Body, "Body"),
        (Self::Auth, "Auth"),
        (Self::Params, "Query"),
        (Self::Headers, "Headers"),
        (Self::Cookies, "Cookies"),
        (Self::Settings, "Settings"),
        (Self::Docs, "Docs"),
        (Self::Tests, "Tests"),
        (Self::Captures, "Captures"),
        (Self::Scripts, "Scripts"),
    ];
}

pub struct EditorPane {
    pub editor: Editor,
    pub tab: EditorTab,
    pub invalid: Vec<(OptionField, &'static str)>,
    pub custom_method: Option<String>,
    /// "Clear all" confirmations per table: queries, headers, cookies, form, multipart.
    pub confirm_clear: [bool; 5],
}

impl EditorPane {
    fn new(editor: Editor) -> Self {
        Self {
            editor,
            tab: EditorTab::default(),
            invalid: Vec::new(),
            custom_method: None,
            confirm_clear: [false; 5],
        }
    }
}

/// The tabs of History's request as sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SentTab {
    #[default]
    Body,
    Headers,
}

/// The tabs of a response viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseTab {
    #[default]
    Body,
    Headers,
    /// Results of the request's checks; offered only when the response has some.
    Tests,
    /// The request as sent, generated values included; offered once one was sent.
    Request,
}

pub struct ResponsePane {
    pub response: Option<Response>,
    pub notice: Option<(String, Tone)>,
    pub pending: bool,
    pub runs: Vec<ExecutionSummary>,
    pub shown_run: Option<ExecutionId>,
    pub tab: ResponseTab,
    pub body_view: ViewerState,
    pub headers_view: ViewerState,
    /// Headers as the raw text the viewer shows, instead of a table.
    pub headers_raw: bool,
    /// The formatted preview of `response`, computed once per response.
    pub preview: Option<(crate::viewer::Preview, String)>,
    /// Folded lines of the formatted body.
    pub body_folds: BTreeSet<usize>,
    /// The last send went nowhere, and why; shown instead of a response.
    pub blocked: Option<NotSent>,
    pub filter: ResponseFilter,
    /// The request that brought the shown response, as one text (`SentRequest::as_text`).
    pub sent: Option<String>,
    pub sent_view: ViewerState,
    /// Identifies the shown response, so a late filter result for another one is dropped.
    shown: u64,
}

/// The JSONPath filter of a response viewer. One run is in flight at a time; when it ends,
/// the newest query runs if it changed meanwhile.
#[derive(Debug, Default)]
pub struct ResponseFilter {
    pub open: bool,
    pub query: String,
    pub focus: bool,
    /// The query whose result `result` is, with that result.
    pub result: Option<(String, Result<crate::viewer::Filtered, String>)>,
    pub folds: BTreeSet<usize>,
    document: Option<crate::viewer::Document>,
    running: bool,
}

impl ResponseFilter {
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
    }

    /// The result to show instead of the body: the last finished one while a filter is set.
    pub fn shown(&self) -> Option<&Result<crate::viewer::Filtered, String>> {
        (self.open && !self.query.trim().is_empty())
            .then_some(self.result.as_ref())
            .flatten()
            .map(|(_, result)| result)
    }

    /// Whether the shown result is for an older query.
    pub fn stale(&self) -> bool {
        self.result
            .as_ref()
            .is_none_or(|(query, _)| query != self.query.trim())
    }
}

/// Which response viewer a background result belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Request,
    History,
    Run,
}

/// A filter run to start in the background.
pub struct FilterJob {
    shown: u64,
    query: String,
    source: crate::viewer::FilterSource,
}

static NEXT_SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Default for ResponsePane {
    fn default() -> Self {
        Self {
            response: None,
            notice: None,
            pending: false,
            runs: Vec::new(),
            shown_run: None,
            tab: ResponseTab::Body,
            body_view: ViewerState::new(),
            headers_view: ViewerState::new(),
            headers_raw: false,
            preview: None,
            body_folds: BTreeSet::new(),
            blocked: None,
            filter: ResponseFilter::default(),
            sent: None,
            sent_view: ViewerState::new(),
            shown: 0,
        }
    }
}

impl ResponsePane {
    pub fn show(&mut self, response: Response) {
        let preview = crate::viewer::preview(&response);
        let headers = crate::viewer::headers_text(&response);
        self.notice = Some((crate::viewer::notice(&response, &preview), Tone::Muted))
            .filter(|(n, _)| !n.is_empty());
        self.preview = Some((preview, headers));
        if self.tab == ResponseTab::Tests
            && response.checks.is_empty()
            && response.captures.is_empty()
        {
            self.tab = ResponseTab::Body;
        }
        self.response = Some(response);
        self.blocked = None;
        self.sent = None;
        self.body_folds.clear();
        self.shown = NEXT_SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The filter stays open with its query and runs again on the new body.
        let filter = &mut self.filter;
        filter.result = None;
        filter.document = None;
        filter.running = false;
        filter.folds.clear();
    }

    /// The filter run this pane needs now, if any; marks it as running.
    pub fn filter_job(&mut self) -> Option<FilterJob> {
        let response = self.response.as_ref()?;
        let filter = &mut self.filter;
        let query = filter.query.trim();
        if !filter.open
            || query.is_empty()
            || filter.running
            || !filter.stale()
            || !crate::viewer::is_json(response)
        {
            return None;
        }
        filter.running = true;
        let source = match &filter.document {
            Some(document) => crate::viewer::FilterSource::Parsed(Arc::clone(document)),
            None => crate::viewer::FilterSource::Body {
                bytes: response.body.clone(),
                truncated: response.truncated,
            },
        };
        Some(FilterJob {
            shown: self.shown,
            query: query.to_owned(),
            source,
        })
    }

    fn filtered(&mut self, done: Filtered) {
        if done.shown != self.shown {
            return;
        }
        let filter = &mut self.filter;
        filter.running = false;
        if done.document.is_some() {
            filter.document = done.document;
        }
        filter.result = Some((done.query, done.result));
        filter.folds.clear();
    }

    /// Back to no response. Nothing is said here: the request pane says whether a request
    /// is open, and this pane invites a send.
    fn clear(&mut self) {
        *self = Self::default();
    }
}

struct Execution {
    task: TaskId,
    /// When it was first shown (egui time), for the elapsed time.
    shown_at: Option<f64>,
}

/// What the status bar says: the latest action's result, in plain language.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusLine {
    pub text: String,
    pub tone: Tone,
    lasting: Lasting,
}

impl StatusLine {
    /// Whether the message still applies with `generation` and `screen` showing at `now`.
    fn applies(&mut self, generation: u64, screen: Screen, now: f64) -> bool {
        match &mut self.lasting {
            Lasting::While {
                generation: g,
                screen: s,
            } => *g == generation && *s == screen,
            Lasting::For { seconds, shown_at } => now - *shown_at.get_or_insert(now) < *seconds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lasting {
    /// About the open request: gone once another request or screen is shown.
    While { generation: u64, screen: Screen },
    /// Gone this many seconds after it is first drawn.
    For { seconds: f64, shown_at: Option<f64> },
}

/// How long a passing status message stays.
pub const STATUS_SECONDS: f64 = 6.0;

impl State {
    /// The status bar's message at `now`, dropping it once it no longer applies: another
    /// request or screen for a message about the request, its time for a passing one.
    pub fn status_line(&mut self, now: f64) -> Option<&StatusLine> {
        let (generation, screen) = (self.generation, self.screen);
        if !self.status.as_mut()?.applies(generation, screen, now) {
            self.status = None;
        }
        self.status.as_ref()
    }

    /// Seconds since a send started, while one runs.
    pub fn sending_for(&mut self, now: f64) -> Option<f64> {
        let execution = self.execution.as_mut()?;
        Some(now - *execution.shown_at.get_or_insert(now))
    }
}

/// Why a send went nowhere: the request could not be prepared. `unresolved` names the
/// variables nothing defines, so the response pane can offer to define them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSent {
    pub message: String,
    pub unresolved: Vec<String>,
}

impl From<ExecuteError> for NotSent {
    fn from(error: ExecuteError) -> Self {
        let unresolved = match &error {
            ExecuteError::Unresolved(names) => names.clone(),
            ExecuteError::Invalid(_) | ExecuteError::Script(_) | ExecuteError::Store(_) => {
                Vec::new()
            }
        };
        Self {
            message: format!("Not sent: {error}"),
            unresolved,
        }
    }
}

/// A finished filter run.
#[derive(Debug)]
pub struct Filtered {
    shown: u64,
    query: String,
    /// `None` when the run failed before parsing finished.
    document: Option<crate::viewer::Document>,
    result: Result<crate::viewer::Filtered, String>,
}

/// All UI state. Kept apart from the bridge and store so drawing can borrow it freely.
pub struct State {
    pub settings: AppSettings,
    pub collections: Vec<Collection>,
    pub collection: Option<CollectionId>,
    pub tree: Option<TreeView>,
    pub selected: Option<NodeId>,
    pub editor: Option<EditorPane>,
    pub saver: Saver,
    pub response: ResponsePane,
    pub layout: WorkspaceLayout,
    pub dialog: Option<Dialog>,
    status: Option<StatusLine>,
    pub screen: Screen,
    pub environments: Vec<Environment>,
    pub active_environment: Option<EnvironmentId>,
    /// Variables the open request resolves against, with provenance.
    pub snapshot: Option<Snapshot>,
    pub snapshot_error: Option<String>,
    snapshot_generation: u64,
    pub inspect: Option<Inspection>,
    pub variables: VariablesScreen,
    pub history: HistoryScreen,
    pub settings_screen: SettingsScreen,
    pub mocks: crate::mock_screen::MockScreen,
    pub load: crate::load_screen::LoadScreen,
    pub run: crate::run_screen::RunScreen,
    /// Bumped by every navigation; results of older work are ignored.
    pub generation: u64,
    tree_generation: u64,
    autosave_due: Option<f64>,
    execution: Option<Execution>,
    guard: Option<Guard>,
    close: CloseState,
    restore: Option<RequestId>,
    started: bool,
    /// A reviewed Postman export waiting for its destination.
    postman: Option<PostmanExport>,
    /// Text to put on the clipboard at the end of this frame.
    pub clipboard: Option<String>,
    /// Focus and select the request URL on the next frame (Cmd/Ctrl+L).
    pub focus_url: bool,
    /// The sidebar filter as typed (see `filter::Query`).
    pub sidebar_filter: String,
    /// The header logo, uploaded on first use.
    pub logo: Option<egui::TextureHandle>,
    /// Requests opened recently, newest first (the palette lists them first).
    pub recent: Vec<RequestId>,
    pub appearance: Appearance,
    pub actions: Vec<Action>,
}

pub struct ImposterApp {
    bridge: Bridge<Message>,
    store: Option<Store>,
    db: StoreHandle,
    pub state: State,
    window: Option<WindowState>,
    first_frame: Option<std::time::Instant>,
    /// Scripted file dialog answers (tests); `None` opens native dialogs.
    file_answers: Option<VecDeque<Option<PathBuf>>>,
    smoke: Option<SmokeTest>,
    /// The appearance last installed in egui, to reinstall only on a change.
    applied: Option<Appearance>,
}

impl ImposterApp {
    pub fn new(ctx: &egui::Context, store: Store) -> std::io::Result<Self> {
        ctx.set_fonts(theme::fonts());
        theme::apply(
            ctx,
            theme::ThemeChoice::default(),
            theme::Density::default(),
        );
        let repaint = ctx.clone();
        let bridge = Bridge::new(move || repaint.request_repaint())?;
        let db = store.handle();
        let mut app = Self {
            bridge,
            db,
            store: Some(store),
            state: State {
                settings: AppSettings::default(),
                collections: Vec::new(),
                collection: None,
                tree: None,
                selected: None,
                editor: None,
                saver: Saver::default(),
                response: ResponsePane::default(),
                layout: WorkspaceLayout::default(),
                dialog: None,
                status: Some(StatusLine {
                    text: "Starting…".into(),
                    tone: Tone::Muted,
                    // Until startup finishes and clears it.
                    lasting: Lasting::For {
                        seconds: f64::INFINITY,
                        shown_at: None,
                    },
                }),
                screen: Screen::Requests,
                environments: Vec::new(),
                active_environment: None,
                snapshot: None,
                snapshot_error: None,
                snapshot_generation: 0,
                inspect: None,
                variables: VariablesScreen::default(),
                history: HistoryScreen::default(),
                settings_screen: SettingsScreen::default(),
                mocks: crate::mock_screen::MockScreen::default(),
                load: crate::load_screen::LoadScreen::default(),
                run: crate::run_screen::RunScreen::default(),
                generation: 0,
                tree_generation: 0,
                autosave_due: None,
                execution: None,
                guard: None,
                close: CloseState::Open,
                restore: None,
                started: false,
                postman: None,
                clipboard: None,
                focus_url: false,
                sidebar_filter: String::new(),
                logo: None,
                recent: Vec::new(),
                appearance: Appearance::default(),
                actions: Vec::new(),
            },
            window: None,
            first_frame: Some(std::time::Instant::now()),
            file_answers: None,
            smoke: None,
            applied: None,
        };
        app.state.response.clear();
        app.task(
            |db| {
                Ok(Startup {
                    settings: kv::app_settings(db)?,
                    collections: collections::collections(db)?,
                    environments: environments::environments(db)?,
                    active_environment: environments::active_environment(db)?,
                    last: kv::get(db, Table::UiState, LAST_KEY)?.unwrap_or_default(),
                    // A convenience list: an unreadable one starts empty rather than failing.
                    recent: kv::get(db, Table::UiState, RECENT_KEY)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    // A preference too: unreadable means the defaults.
                    appearance: kv::get(db, Table::UiState, APPEARANCE_KEY)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    layout: kv::get(db, Table::UiState, LAYOUT_KEY)?.unwrap_or(Value::Null),
                    draft: kv::get(db, Table::UiState, DRAFT_KEY)?.flatten(),
                    // A convenience: unreadable means the defaults.
                    load_form: kv::get(db, Table::UiState, LOAD_KEY)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                })
            },
            |result| Message::Started(result.map(Box::new).map_err(failure)),
        );
        Ok(app)
    }

    pub fn bridge(&self) -> &Bridge<Message> {
        &self.bridge
    }

    pub fn store(&self) -> &StoreHandle {
        &self.db
    }

    pub fn close_state(&self) -> CloseState {
        self.state.close
    }

    pub fn is_started(&self) -> bool {
        self.state.started
    }

    /// Send `GET url` as an unsaved draft once started, then close the app; the outcome lands
    /// in `report`. Used by the packaged-binary smoke test.
    pub fn smoke_test(&mut self, url: String, report: Arc<OnceLock<SmokeResult>>) {
        self.smoke = Some(SmokeTest {
            url,
            sent: false,
            result: report,
        });
        self.start_smoke();
    }

    fn start_smoke(&mut self) {
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if smoke.sent || !self.state.started {
            return;
        }
        smoke.sent = true;
        let mut request = Request::default();
        request.template.name = "Smoke test".into();
        request.template.url.clone_from(&smoke.url);
        self.load_editor(&request);
        self.send();
    }

    fn finish_smoke(&mut self, result: SmokeResult) {
        let Some(smoke) = self.smoke.take() else {
            return;
        };
        let _ = smoke.result.set(result);
        // The draft was only a vehicle; closing must not ask about it.
        self.state.editor = None;
        self.request_close();
    }

    /// Answer the next file dialogs with these paths (`None`: cancelled) instead of opening
    /// native dialogs, which tests cannot drive. Multi-file dialogs take one answer per file
    /// until a `None`.
    pub fn script_file_dialogs(&mut self, answers: impl IntoIterator<Item = Option<PathBuf>>) {
        self.file_answers
            .get_or_insert_with(VecDeque::new)
            .extend(answers);
    }

    /// Show a file dialog in the background; `done` turns the chosen paths (none when
    /// cancelled) into a message.
    fn pick<M>(&mut self, dialog: FileDialog, done: M)
    where
        M: FnOnce(Vec<PathBuf>) -> Message + Send + 'static,
    {
        if let Some(answers) = &mut self.file_answers {
            let mut paths = Vec::new();
            match dialog {
                FileDialog::OpenMany { .. } => {
                    while let Some(Some(path)) = answers.pop_front() {
                        paths.push(path);
                    }
                }
                FileDialog::Open { .. } | FileDialog::Save { .. } => {
                    paths.extend(answers.pop_front().flatten());
                }
            }
            self.bridge.spawn(move |_| async move { done(paths) });
            return;
        }
        self.bridge.spawn(move |_| async move {
            let paths = match dialog {
                FileDialog::Open { title, filter } => {
                    let mut builder = rfd::AsyncFileDialog::new().set_title(title);
                    if let Some((label, extensions)) = filter {
                        builder = builder.add_filter(label, extensions);
                    }
                    builder
                        .pick_file()
                        .await
                        .into_iter()
                        .map(|file| file.path().to_owned())
                        .collect()
                }
                FileDialog::OpenMany { title } => rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter("JSON", &["json"])
                    .pick_files()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|file| file.path().to_owned())
                    .collect(),
                FileDialog::Save {
                    title,
                    name,
                    filter,
                } => {
                    let mut builder = rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .set_file_name(name);
                    if let Some((label, extension)) = filter {
                        builder = builder.add_filter(label, &[extension]);
                    }
                    builder
                        .save_file()
                        .await
                        .into_iter()
                        .map(|file| file.path().to_owned())
                        .collect()
                }
            };
            done(paths)
        });
    }

    /// Run one database command in the background and turn its result into a message.
    fn task<T, F, M>(&self, work: F, done: M)
    where
        T: Send + 'static,
        F: FnOnce(&store::Tx<'_>) -> Result<T, StoreError> + Send + 'static,
        M: FnOnce(Result<T, StoreError>) -> Message + Send + 'static,
    {
        let db = self.db.clone();
        self.bridge
            .spawn(move |_| async move { done(db.call(work).await) });
    }

    fn write_ui_state(&self, key: &'static str, value: Value) {
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            if let Err(error) = db
                .call(move |tx| kv::set(tx, Table::UiState, key, &value))
                .await
            {
                tracing::warn!(%error, key, "saving UI state failed");
            }
            Message::Done
        });
    }

    fn remember_selection(&self) {
        let last = Last {
            collection_id: self.state.collection,
            request_id: self.state.editor.as_ref().and_then(|pane| pane.editor.id()),
        };
        self.write_ui_state(LAST_KEY, serde_json::to_value(last).unwrap_or(Value::Null));
    }

    // ---- messages -------------------------------------------------------------------------

    fn apply(&mut self, message: Message) {
        match message {
            Message::Filtered(pane, done) => self.pane(pane).filtered(*done),
            Message::MocksLoaded(result) => self.mocks_loaded(result),
            Message::MocksSaved { config, result } => self.mocks_saved(config, result),
            Message::MockListening {
                generation,
                address,
            } => {
                if generation == self.state.mocks.generation
                    && let Some(run) = &mut self.state.mocks.run
                {
                    run.address = Some(address);
                }
            }
            Message::MockLog { generation, entry } => {
                if generation == self.state.mocks.generation {
                    self.state.mocks.logged(*entry);
                }
            }
            Message::LoadFinished { generation, result } => self.load_finished(generation, result),
            Message::RunFinished { generation, result } => self.run_finished(generation, result),
            Message::MockStopped { generation, result } => {
                if generation == self.state.mocks.generation {
                    self.state.mocks.run = None;
                    self.state.mocks.notice = Some(match result {
                        Ok(()) => "Stopped".into(),
                        Err(error) => error,
                    });
                }
            }
            Message::Done => {}
            Message::Started(Ok(startup)) => {
                self.started(*startup);
                self.start_smoke();
            }
            Message::Started(Err(error)) => {
                self.state.started = true;
                if self.smoke.is_some() {
                    return self.finish_smoke(Err(format!("Cannot load your data: {error}")));
                }
                self.error("Cannot load your data", error);
            }
            Message::Collections {
                select,
                result: Ok(collections),
            } => {
                self.state.collections = collections;
                let known = |id: &CollectionId| self.state.collections.iter().any(|c| c.id == *id);
                let target = select
                    .filter(known)
                    .or(self.state.collection.filter(known))
                    .or_else(|| self.state.collections.first().map(|c| c.id));
                if let Some(id) = target {
                    self.switch_collection(id);
                } else {
                    self.state.collection = None;
                    self.state.tree = None;
                }
            }
            Message::Tree {
                generation,
                result: Ok(tree),
            } => {
                if generation != self.state.tree_generation {
                    return;
                }
                self.state.tree = tree.map(TreeView::build);
                if let Some(id) = self.state.restore.take()
                    && self.state.editor.is_none()
                    && self
                        .state
                        .tree
                        .as_ref()
                        .is_some_and(|t| t.request(id).is_some())
                {
                    self.open(id);
                }
            }
            Message::Opened {
                generation,
                result: Ok(found),
            } => {
                if generation != self.state.generation {
                    return;
                }
                match found.map(|request| *request) {
                    Some(request) => {
                        let request = request
                            .id
                            .and_then(|id| self.state.saver.pending(id).cloned())
                            .unwrap_or(request);
                        self.load_editor(&request);
                    }
                    None => self.status("That request no longer exists"),
                }
            }
            Message::Duplicated { generation, result } => match result {
                Ok(request) => {
                    if generation == self.state.generation {
                        self.state.generation += 1;
                        self.load_editor(&request);
                    }
                    self.reload_tree();
                }
                Err(error) => self.error("Cannot duplicate request", error),
            },
            Message::Created {
                result: Ok(request),
            } => {
                self.state.generation += 1;
                self.load_editor(&request);
                self.reload_tree();
            }
            Message::Saved { id, result } => self.saved(id, result),
            Message::DraftSaved { generation, result } => self.draft_saved(generation, result),
            Message::Executed { generation, result } => self.executed(generation, result),
            Message::Runs { generation, result } => {
                if generation == self.state.generation {
                    match result {
                        Ok(runs) => self.state.response.runs = runs,
                        Err(error) => {
                            self.state.response.notice = Some((
                                format!("Response history could not be loaded: {error}"),
                                Tone::Error,
                            ));
                        }
                    }
                }
            }
            Message::Run { generation, result } => {
                if generation == self.state.generation {
                    match result {
                        Ok(Some(record)) => {
                            let sent = crate::viewer::sent_request(&record).as_text();
                            self.state.response.show(record.response);
                            self.state.response.sent = Some(sent);
                        }
                        Ok(None) => self.status("That run is no longer in history"),
                        Err(error) => self.error("History could not be loaded", error),
                    }
                }
            }
            Message::Changed {
                reload_collections,
                result: Ok(()),
            } => {
                if reload_collections {
                    self.reload_collections(None);
                } else {
                    self.reload_tree();
                }
            }
            Message::Snapshot { generation, result } => {
                if generation == self.state.snapshot_generation {
                    match result {
                        Ok(snapshot) => {
                            self.state.snapshot = Some(*snapshot);
                            self.state.snapshot_error = None;
                        }
                        Err(_) => {
                            self.state.snapshot_error =
                                Some("Variable definitions could not be loaded".into());
                        }
                    }
                }
            }
            Message::Environments { select, result } => match result {
                Ok((environments, active)) => {
                    self.state.environments = environments;
                    self.state.active_environment = active;
                    if let Some(scope) = select {
                        self.load_variables(scope);
                    } else if let Scope::Environment(id) = self.state.variables.scope
                        && !self.state.environments.iter().any(|e| e.id == id)
                    {
                        self.load_variables(Scope::Global);
                    }
                    self.load_snapshot();
                }
                Err(error) => self.error("Environment operation failed", error),
            },
            Message::VariablesLoaded { scope, result } => self.variables_loaded(scope, result),
            Message::VariablesSaved { scope, result } => self.variables_saved(scope, result),
            Message::ValueChosen {
                scope,
                name,
                result,
            } => self.value_chosen(scope, &name, result),
            Message::CapturesWritten {
                scopes,
                names,
                result,
            } => self.captures_written(&scopes, &names, result),
            Message::HistoryPage { generation, result } => self.history_page(generation, result),
            Message::HistoryUsage(result) => match result {
                Ok(usage) => self.state.history.usage = Some(usage),
                Err(error) => tracing::warn!(%error, "reading the history usage failed"),
            },
            Message::HistoryRecord { id, result } => {
                if self.state.history.selected == Some(id) {
                    match result {
                        Ok(Some(record)) => {
                            let history = &mut self.state.history;
                            history.request = Some(crate::viewer::sent_request(&record));
                            history.response.show(record.response.clone());
                            history.record = Some(*record);
                        }
                        Ok(None) => {
                            self.state.history.status = Some((
                                "That execution is no longer in history".into(),
                                Tone::Warning,
                            ));
                        }
                        Err(error) => {
                            self.state.history.status =
                                Some((format!("History unavailable: {error}"), Tone::Error));
                        }
                    }
                }
            }
            Message::Flushed(Ok(())) => self.state.close = CloseState::Ready,
            Message::Flushed(Err(error)) => {
                self.state.close = CloseState::Open;
                self.state.dialog = Some(Dialog::Confirm {
                    title: "Changes could not be saved".into(),
                    message: format!("{error}\nClose anyway and discard them, or cancel to retry."),
                    confirm: "Discard and close".into(),
                    purpose: ConfirmPurpose::DiscardOnClose,
                });
            }
            Message::PathChecked { path, exists } => {
                self.state.settings_screen.paths.insert(path, Some(exists));
            }
            Message::FilePicked { target, path } => {
                if let Some(path) = path {
                    self.file_picked(target, &path);
                }
            }
            Message::ResultPicked(path) => {
                if let Some(path) = path {
                    let file = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into(),
                    );
                    self.bridge.spawn(move |_| async move {
                        let result = transfer::read_result(path).await.map(Box::new);
                        Message::ResultRead { file, result }
                    });
                }
            }
            Message::ResultRead { file, result } => match result {
                Ok(result) => {
                    let history = &mut self.state.history;
                    history.selected = None;
                    history.record = None;
                    history.request = result.request.as_ref().map(crate::viewer::result_request);
                    history.response = ResponsePane::default();
                    if let Some(response) = result.response() {
                        history.response.show(response);
                    }
                    if let Some(notice) = result
                        .response
                        .as_ref()
                        .and_then(crate::viewer::result_notice)
                    {
                        history.response.notice = Some((notice, Tone::Muted));
                    }
                    history.tab = HistoryTab::Response;
                    history.status = None;
                    history.opened = Some(OpenedResult {
                        file,
                        result: *result,
                    });
                }
                Err(error) => {
                    self.state.history.status =
                        Some((format!("Cannot open {file}: {error}"), Tone::Error));
                }
            },
            Message::ImportPicked(path) => {
                if let Some(path) = path {
                    self.bridge.spawn(move |_| async move {
                        let result = transfer::detect(path.clone()).await.map_err(failure);
                        Message::ImportDetected { path, result }
                    });
                }
            }
            Message::ImportDetected { path, result } => match result {
                Ok(kind) if kind.takes_companions() => {
                    self.state.dialog = Some(Dialog::Companions { path });
                }
                Ok(_) => self.read_import(path, Vec::new()),
                Err(error) => self.error("Import failed", error),
            },
            Message::CompanionsPicked { path, companions } => self.read_import(path, companions),
            Message::ImportPreviewed(Ok(preview)) => {
                self.state.dialog = Some(Dialog::Import {
                    preview,
                    remove_unsupported: false,
                });
            }
            Message::ImportPreviewed(Err(error)) | Message::Imported(Err(error)) => {
                self.error("Import failed", error);
            }
            Message::Imported(Ok(id)) => self.imported(id),
            Message::BodySavePicked { path, bytes, cut } => {
                if let Some(path) = path {
                    self.bridge.spawn(move |_| async move {
                        let result = transfer::write_file(path.clone(), bytes).await;
                        Message::BodySaved {
                            result: result.map(|()| path).map_err(|error| error.to_string()),
                            cut,
                        }
                    });
                }
            }
            Message::BodySaved { result, cut } => match result {
                Ok(path) => {
                    let file = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    let cut = if cut { " (cut when captured)" } else { "" };
                    self.status(&format!("Saved the body to {file}{cut}"));
                }
                Err(error) => self.error("Couldn't save the body", error),
            },
            Message::NativeExportPicked { scope, path } => {
                if let Some(path) = path {
                    let db = self.db.clone();
                    self.bridge.spawn(move |_| async move {
                        let result = transfer::export_native(&db, *scope, path.clone()).await;
                        Message::Exported(result.map(|()| path).map_err(failure))
                    });
                }
            }
            Message::PostmanReady(Ok(export)) => {
                let warnings = export.warnings.join("\n");
                self.state.postman = Some(*export);
                if warnings.is_empty() {
                    self.pick_postman_destination();
                } else {
                    self.state.dialog = Some(Dialog::Confirm {
                        title: "Postman export limitations".into(),
                        message: warnings,
                        confirm: "Export".into(),
                        purpose: ConfirmPurpose::PostmanExport,
                    });
                }
            }
            Message::PostmanExportPicked(path) => {
                if let (Some(path), Some(export)) = (path, self.state.postman.take()) {
                    self.bridge.spawn(move |_| async move {
                        let result =
                            transfer::write_postman(export.document, export.data, path.clone())
                                .await;
                        Message::Exported(result.map(|()| path).map_err(failure))
                    });
                }
            }
            Message::PostmanReady(Err(error)) | Message::Exported(Err(error)) => {
                self.error("Export failed", error);
            }
            Message::SettingsLoaded(result) => self.settings_loaded(result),
            Message::Curl(Ok((command, host_identities))) => {
                self.state.clipboard = Some(command);
                if host_identities {
                    self.status(executor::CURL_IDENTITY_NOTICE);
                } else {
                    self.status("Copied as curl");
                }
            }
            Message::Curl(Err(error)) => self.error("Copy as curl failed", error),
            Message::RequestIndex(Ok(index)) => {
                // Requests deleted since they were opened leave the recent list.
                let before = self.state.recent.len();
                self.state
                    .recent
                    .retain(|id| index.iter().any(|r| r.id == *id));
                if self.state.recent.len() != before {
                    self.write_recent();
                }
                if let Some(Dialog::Palette(palette)) = &mut self.state.dialog {
                    palette.recent.clone_from(&self.state.recent);
                    palette.index = Some(index);
                }
            }
            Message::RequestIndex(Err(error)) => {
                tracing::warn!(%error, "loading the request index failed");
            }
            Message::SettingsSaved { form, result } => self.settings_saved(form, result),
            Message::Exported(Ok(path)) => {
                self.state.dialog = Some(Dialog::info(
                    "Export complete",
                    format!("Saved {}", path.display()),
                ));
            }
            Message::Collections {
                result: Err(error), ..
            }
            | Message::Tree {
                result: Err(error), ..
            }
            | Message::Opened {
                result: Err(error), ..
            }
            | Message::Created { result: Err(error) }
            | Message::Changed {
                result: Err(error), ..
            } => {
                self.error("Collection operation failed", error);
                self.reload_tree();
            }
        }
    }

    fn started(&mut self, startup: Startup) {
        let state = &mut self.state;
        state.started = true;
        state.settings = startup.settings;
        state.layout = WorkspaceLayout::from_stored(&startup.layout);
        state.status = None;
        state.environments = startup.environments;
        state.active_environment = startup.active_environment;
        state.recent = startup.recent;
        state.appearance = startup.appearance;
        state.load.form = startup.load_form;
        if let Some(draft) = startup.draft {
            self.load_editor(&draft);
            self.request_status("Restored the draft you left open", Tone::Muted);
        } else {
            self.state.restore = startup.last.request_id;
        }
        self.state.collections = startup.collections;
        let target = startup
            .last
            .collection_id
            .filter(|id| self.state.collections.iter().any(|c| c.id == *id))
            .or_else(|| self.state.collections.first().map(|c| c.id));
        self.state.collection = target;
        if let Some(id) = target {
            self.load_tree(id);
        }
    }

    /// A passing message that fades after [`STATUS_SECONDS`].
    fn status(&mut self, text: &str) {
        self.status_for(text, Tone::Muted, STATUS_SECONDS);
    }

    fn status_for(&mut self, text: &str, tone: Tone, seconds: f64) {
        self.state.status = Some(StatusLine {
            text: text.to_owned(),
            tone,
            lasting: Lasting::For {
                seconds,
                shown_at: None,
            },
        });
    }

    /// A message about the open request, kept while it stays open on this screen.
    fn request_status(&mut self, text: &str, tone: Tone) {
        self.state.status = Some(StatusLine {
            text: text.to_owned(),
            tone,
            lasting: Lasting::While {
                generation: self.state.generation,
                screen: self.state.screen,
            },
        });
    }

    fn error(&mut self, title: &str, message: impl Into<String>) {
        self.state.dialog = Some(Dialog::error(title, message));
    }

    // ---- loading --------------------------------------------------------------------------

    /// Store a collection's or folder's scripts, then reload what shows them.
    fn save_scripts(
        &mut self,
        target: crate::dialogs::ScriptTarget,
        scripts: imposter_app::core::model::Scripts,
    ) {
        use crate::dialogs::ScriptTarget;
        match target {
            ScriptTarget::Collection(id) => self.task(
                move |db| collections::set_collection_scripts(db, id, &scripts),
                |result| Message::Changed {
                    reload_collections: true,
                    result: result.map_err(failure),
                },
            ),
            ScriptTarget::Folder(id) => self.task(
                move |db| collections::set_folder_scripts(db, id, &scripts),
                |result| Message::Changed {
                    reload_collections: false,
                    result: result.map_err(failure),
                },
            ),
        }
    }

    fn reload_collections(&mut self, select: Option<CollectionId>) {
        self.task(
            |db| collections::collections(db),
            move |result| Message::Collections {
                select,
                result: result.map_err(failure),
            },
        );
    }

    fn load_tree(&mut self, id: CollectionId) {
        self.state.tree_generation += 1;
        let generation = self.state.tree_generation;
        self.task(
            move |db| collections::tree(db, id),
            move |result| Message::Tree {
                generation,
                result: result.map_err(failure),
            },
        );
    }

    fn reload_tree(&mut self) {
        if let Some(id) = self.state.collection {
            self.load_tree(id);
        }
    }

    fn load_runs(&mut self, request: RequestId) {
        let generation = self.state.generation;
        self.task(
            move |db| executions::page(db, Some(request), 0, 50),
            move |result| Message::Runs {
                generation,
                result: result.map_err(failure),
            },
        );
    }

    /// Put `request` in the editor; its normalized form becomes the saved baseline.
    fn load_editor(&mut self, request: &Request) {
        let editor = Editor::load(request);
        if let Ok(normalized) = editor.to_request() {
            self.state.saver.remember(&normalized);
        }
        self.state.selected = editor.id().map(NodeId::Request);
        let id = editor.id();
        self.state.editor = Some(EditorPane::new(editor));
        self.state.response.clear();
        self.state.inspect = None;
        self.load_snapshot();
        if let Some(id) = id {
            self.load_runs(id);
            self.remember_selection();
            if self.state.recent.first() != Some(&id) {
                crate::palette::remember(&mut self.state.recent, id);
                self.write_recent();
            }
        }
    }

    fn write_recent(&self) {
        let recent = serde_json::to_value(&self.state.recent).unwrap_or(Value::Null);
        self.write_ui_state(RECENT_KEY, recent);
    }

    fn clear_editor(&mut self) {
        if let Some(id) = self.state.editor.as_ref().and_then(|pane| pane.editor.id()) {
            self.state.saver.forget(id);
        }
        self.state.editor = None;
        self.state.selected = None;
        self.state.generation += 1;
        self.state.response.clear();
        self.remember_selection();
    }

    fn open(&mut self, id: RequestId) {
        self.state.generation += 1;
        let generation = self.state.generation;
        self.state.selected = Some(NodeId::Request(id));
        self.task(
            move |db| collections::request(db, id),
            move |result| Message::Opened {
                generation,
                result: result.map(|found| found.map(Box::new)).map_err(failure),
            },
        );
    }

    // ---- navigation and the unsaved-changes guard ---------------------------------------------

    fn is_dirty(&self) -> bool {
        self.state.editor.as_ref().is_some_and(|pane| {
            !pane.invalid.is_empty()
                || pane.editor.is_draft()
                || pane
                    .editor
                    .id()
                    .is_some_and(|id| self.state.saver.state(id) != SaveState::Saved)
        })
    }

    fn navigate(&mut self, next: Navigation) {
        if self.state.guard.is_some() {
            return;
        }
        if !self.is_dirty() {
            return self.perform(next);
        }
        let pane = self.state.editor.as_ref().expect("dirty implies an editor");
        let autosaving = self.state.settings.autosave && pane.invalid.is_empty();
        match pane.editor.id() {
            Some(id) if autosaving => {
                self.state.guard = Some(Guard {
                    next,
                    stage: GuardStage::Saving,
                });
                self.state.autosave_due = None;
                self.pump_saves(Some(id));
                self.check_guard_save(id);
            }
            _ => {
                let request = if pane.editor.name().is_empty() {
                    "Untitled request"
                } else {
                    pane.editor.name()
                }
                .to_owned();
                self.state.guard = Some(Guard {
                    next,
                    stage: GuardStage::Asking,
                });
                let draft = self.guard_draft();
                self.state.dialog = Some(Dialog::Guard {
                    request,
                    draft,
                    failed: None,
                });
            }
        }
    }

    /// For a draft in the editor: the collection Save would add it to.
    fn guard_draft(&self) -> Option<String> {
        self.state
            .editor
            .as_ref()
            .filter(|pane| pane.editor.is_draft())?;
        Some(
            self.state
                .collection
                .and_then(|id| self.state.collections.iter().find(|c| c.id == id))
                .map_or_else(|| "the selected collection".to_owned(), |c| c.name.clone()),
        )
    }

    fn perform(&mut self, next: Navigation) {
        match next {
            Navigation::Open(id) => self.open(id),
            Navigation::OpenIn {
                collection,
                request,
            } => {
                if self.state.collection == Some(collection) {
                    self.open(request);
                } else {
                    // The tree of the other collection opens it once loaded (`restore`).
                    self.clear_editor();
                    self.state.restore = Some(request);
                    self.switch_collection(collection);
                }
            }
            Navigation::Duplicate { id, edited } => {
                let db = self.db.clone();
                let generation = self.state.generation;
                self.bridge.spawn(move |_| async move {
                    Message::Duplicated {
                        generation,
                        result: ops::duplicate_request(&db, id, edited.map(|t| *t))
                            .await
                            .map(Box::new)
                            .map_err(failure),
                    }
                });
            }
            Navigation::SelectCollection(id) => self.switch_collection(id),
            Navigation::CreateRequest { folder, name } => {
                let Some(collection) = self.state.collection else {
                    return;
                };
                let db = self.db.clone();
                self.bridge.spawn(move |_| async move {
                    Message::Created {
                        result: ops::create_request(&db, collection, folder, name)
                            .await
                            .map(Box::new)
                            .map_err(failure),
                    }
                });
            }
            Navigation::CreateCollection(name) => {
                let db = self.db.clone();
                self.bridge.spawn(move |_| async move {
                    let result = db
                        .call(move |tx| {
                            collections::create_collection(tx, &name, "", &Variables::new())
                        })
                        .await;
                    match result {
                        Ok(created) => Message::Collections {
                            select: Some(created.id),
                            result: db
                                .call(|tx| collections::collections(tx))
                                .await
                                .map_err(failure),
                        },
                        Err(error) => Message::Changed {
                            reload_collections: false,
                            result: Err(failure(error)),
                        },
                    }
                });
            }
            Navigation::Close => self.variables_navigate(VarTransition::Close),
            Navigation::Import {
                preview,
                remove_unsupported,
            } => {
                let db = self.db.clone();
                self.bridge.spawn(move |_| async move {
                    let result = transfer::import(&db, *preview, remove_unsupported).await;
                    Message::Imported(result.map_err(failure))
                });
            }
            Navigation::Restore {
                request,
                status,
                response,
            } => {
                self.state.generation += 1;
                self.load_editor(&request);
                if let Some(response) = response {
                    self.state.response.show(*response);
                }
                self.state.screen = Screen::Requests;
                self.request_status(status, Tone::Muted);
            }
        }
    }

    /// Show collection `id`. A saved request from another collection leaves the editor.
    fn switch_collection(&mut self, id: CollectionId) {
        let other = self.state.editor.as_ref().is_some_and(|pane| {
            pane.editor.id().is_some() && pane.editor.identity().collection_id != Some(id)
        });
        if other {
            self.clear_editor();
        }
        if self.state.collection != Some(id) {
            self.state.tree = None;
        }
        self.state.collection = Some(id);
        self.load_tree(id);
        self.remember_selection();
    }

    /// Drop the pending navigation; a cancelled close leaves the window open.
    fn cancel_guard(&mut self) {
        if let Some(guard) = self.state.guard.take()
            && guard.next == Navigation::Close
        {
            self.state.close = CloseState::Open;
        }
    }

    fn guard_choice(&mut self, choice: GuardChoice) {
        if choice == GuardChoice::Cancel {
            return self.cancel_guard();
        }
        let Some(guard) = self.state.guard.take() else {
            return;
        };
        match choice {
            GuardChoice::Cancel => {}
            GuardChoice::Discard => {
                self.discard_editor_changes();
                self.perform(guard.next);
            }
            GuardChoice::Save => {
                self.state.guard = Some(Guard {
                    stage: GuardStage::Saving,
                    ..guard
                });
                self.save_current();
            }
        }
    }

    fn discard_editor_changes(&mut self) {
        let Some(pane) = &self.state.editor else {
            return;
        };
        if let Some(id) = pane.editor.id() {
            self.state.saver.discard(Some(id));
            if let Some(baseline) = self.state.saver.baseline(id).cloned() {
                let tab = pane.tab;
                let mut fresh = EditorPane::new(Editor::load(&baseline));
                fresh.tab = tab;
                self.state.editor = Some(fresh);
            }
        } else {
            self.state.editor = None;
            self.state.response.clear();
        }
        self.state.autosave_due = None;
    }

    fn guard_failed(&mut self, error: String) {
        if let Some(guard) = &mut self.state.guard {
            guard.stage = GuardStage::Asking;
            let request = self
                .state
                .editor
                .as_ref()
                .map_or_else(String::new, |p| p.editor.name().to_owned());
            let draft = self.guard_draft();
            self.state.dialog = Some(Dialog::Guard {
                request,
                draft,
                failed: Some(error),
            });
        } else {
            self.error("Save failed", error);
        }
    }

    /// After a write of `id`, continue the guard if it was waiting for exactly that.
    fn check_guard_save(&mut self, id: RequestId) {
        let waiting = self
            .state
            .guard
            .as_ref()
            .is_some_and(|g| g.stage == GuardStage::Saving)
            && self.state.editor.as_ref().and_then(|p| p.editor.id()) == Some(id);
        if !waiting {
            return;
        }
        match self.state.saver.state(id) {
            SaveState::Saved => {
                let guard = self.state.guard.take().expect("checked above");
                self.perform(guard.next);
            }
            SaveState::Failed => {
                let error = self
                    .state
                    .saver
                    .failure(id)
                    .unwrap_or("Saving failed")
                    .to_owned();
                self.guard_failed(error);
            }
            SaveState::Dirty | SaveState::Saving => {}
        }
    }

    // ---- saving ---------------------------------------------------------------------------

    /// Explicit save of the editor: a draft is named and created, a request is written now.
    fn save_current(&mut self) {
        let Some(pane) = &self.state.editor else {
            return;
        };
        let Ok(request) = pane.editor.to_request() else {
            self.cancel_guard();
            return self.error(
                "Save failed",
                "Fix the highlighted request settings before saving.",
            );
        };
        if let Some(id) = request.id {
            self.state.saver.update(&request);
            self.state.autosave_due = None;
            self.pump_saves(Some(id));
            self.check_guard_save(id);
            return;
        }
        let Some(collection) = self.state.collection else {
            self.cancel_guard();
            self.state.dialog = Some(Dialog::info(
                "Save draft",
                "Select or create a collection first. Your draft is retained.",
            ));
            return;
        };
        let collection_name = self
            .state
            .collections
            .iter()
            .find(|c| c.id == collection)
            .map_or_else(String::new, |c| c.name.clone());
        if let Some(guard) = &mut self.state.guard {
            guard.stage = GuardStage::NamingDraft;
        }
        self.state.dialog = Some(Dialog::name(
            "Save to collection",
            &format!("Request name in {collection_name}:"),
            request.template.name.as_str(),
            NamePurpose::SaveDraft(collection),
        ));
    }

    fn save_draft(&mut self, collection: CollectionId, name: String) {
        let Some(Ok(request)) = self.state.editor.as_ref().map(|p| p.editor.to_request()) else {
            return;
        };
        if let Some(guard) = &mut self.state.guard {
            guard.stage = GuardStage::Saving;
        }
        let generation = self.state.generation;
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            Message::DraftSaved {
                generation,
                result: ops::save_draft(&db, request, collection, name)
                    .await
                    .map(Box::new)
                    .map_err(failure),
            }
        });
    }

    fn draft_saved(&mut self, generation: u64, result: Result<Box<Request>, Failure>) {
        match result.map(|saved| *saved) {
            Ok(saved) => {
                if generation == self.state.generation
                    && let Some(pane) = &mut self.state.editor
                {
                    pane.editor.adopt_identity(&saved);
                    if let Ok(current) = pane.editor.to_request() {
                        self.state.saver.remember(&saved);
                        self.state.saver.update(&current);
                    }
                    self.state.selected = saved.id.map(NodeId::Request);
                    self.write_ui_state(DRAFT_KEY, Value::Null);
                    self.remember_selection();
                }
                self.reload_tree();
                if self
                    .state
                    .guard
                    .as_ref()
                    .is_some_and(|g| g.stage == GuardStage::Saving)
                {
                    let guard = self.state.guard.take().expect("checked above");
                    self.perform(guard.next);
                }
            }
            Err(error) => self.guard_failed(error),
        }
    }

    /// Start the next serialized write: only `only`, or any pending request.
    fn pump_saves(&mut self, only: Option<RequestId>) {
        if let Some((id, snapshot)) = self.state.saver.next_write(only) {
            let template = snapshot.template;
            self.task(
                move |db| collections::update_request(db, id, &template),
                move |result| Message::Saved {
                    id,
                    result: result.map_err(failure),
                },
            );
        }
    }

    fn saved(&mut self, id: RequestId, result: Result<(), Failure>) {
        self.state.saver.finish(result);
        // The tree shows each request's method and name; refresh it when a save changed them.
        let stale = match (
            self.state.tree.as_ref().and_then(|t| t.request(id)),
            self.state.saver.baseline(id),
        ) {
            (Some(shown), Some(saved)) => {
                shown.method != saved.template.method || shown.name != saved.template.name
            }
            _ => false,
        };
        if stale && self.state.saver.state(id) != SaveState::Failed {
            self.reload_tree();
        }
        if self.state.guard.is_some() {
            if self.state.saver.state(id) == SaveState::Dirty {
                self.pump_saves(Some(id));
            }
            self.check_guard_save(id);
        } else if self.state.saver.has_pending() && self.state.saver.failures().next().is_none() {
            self.pump_saves(None);
        }
    }

    /// The short save state next to the Save button, a longer explanation for its hover text,
    /// whether saving is possible now, and its tone.
    pub fn save_label(&self) -> Option<SaveStatus<'static>> {
        let pane = self.state.editor.as_ref()?;
        let (word, detail, enabled) = if !pane.invalid.is_empty() {
            (
                SaveWord::Invalid,
                "Fix the highlighted fields to save",
                false,
            )
        } else if let Some(id) = pane.editor.id() {
            match self.state.saver.state(id) {
                SaveState::Saving => (SaveWord::Saving, "Saving the request", false),
                SaveState::Failed => (
                    SaveWord::Failed,
                    "The request changes could not be saved; retry or discard them",
                    true,
                ),
                SaveState::Dirty => (SaveWord::Unsaved, "Unsaved request changes", true),
                SaveState::Saved if self.state.settings.autosave => {
                    (SaveWord::Saved, "Changes are saved automatically", false)
                }
                SaveState::Saved => (SaveWord::Saved, "The request is saved", false),
            }
        } else {
            (SaveWord::Draft, "Not saved to a collection yet", true)
        };
        Some(SaveStatus {
            word,
            detail,
            enabled,
        })
    }

    pub fn save_button_text(&self) -> &'static str {
        match self.state.editor.as_ref().map(|p| {
            (
                p.editor.id(),
                p.editor.id().map(|id| self.state.saver.state(id)),
            )
        }) {
            Some((None, _)) => "Save to collection…",
            Some((_, Some(SaveState::Failed))) => "Retry save",
            _ => "Save request",
        }
    }

    // ---- editing ----------------------------------------------------------------------------

    /// Replace what the open request sends with a pasted curl command, keeping its identity,
    /// name, description and place: an ordinary unsaved edit (the URL field it is pasted into
    /// exists only while a request is open).
    fn paste_curl(&mut self, command: &str, now: f64) {
        let Ok(parsed) = imposter_app::core::curl::parse(command) else {
            return;
        };
        let Some(pane) = &mut self.state.editor else {
            return;
        };
        let mut request = pane.editor.identity().clone();
        let kept = &request.template;
        request.template = imposter_app::core::model::RequestTemplate {
            name: kept.name.clone(),
            description: pane.editor.description.clone(),
            position: kept.position,
            checks: pane.editor.checks.clone(),
            captures: pane.editor.captures.clone(),
            ..parsed.template
        };
        pane.editor = Editor::load(&request);
        pane.custom_method = None;
        self.edited(false, false, now);
        self.load_snapshot();
        self.request_status("Replaced from curl — not saved yet", Tone::Muted);
        if !parsed.warnings.is_empty() {
            self.state.dialog = Some(Dialog::info(
                "Some curl options were not imported",
                parsed.warnings.join("\n"),
            ));
        }
    }

    fn edited(&mut self, url: bool, params: bool, now: f64) {
        let Some(pane) = &mut self.state.editor else {
            return;
        };
        if url {
            pane.editor.url_changed();
        }
        if params {
            pane.editor.params_changed();
        }
        match pane.editor.to_request() {
            Ok(request) => {
                pane.invalid.clear();
                if request.id.is_some() {
                    self.state.saver.update(&request);
                    if self.state.settings.autosave {
                        self.state.autosave_due = Some(now + AUTOSAVE_DELAY);
                    }
                }
            }
            Err(InvalidOptions(errors)) => pane.invalid = errors,
        }
    }

    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let Some(due) = self.state.autosave_due else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if now >= due {
            self.state.autosave_due = None;
            self.pump_saves(None);
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(due - now));
        }
    }

    fn file_picked(&mut self, target: FileRequest, path: &std::path::Path) {
        let path = path.display().to_string();
        if let FileRequest::Setting(field) = target {
            let screen = &mut self.state.settings_screen;
            if let Some(value) = field.value(&mut screen.form) {
                *value = path;
                screen.refresh_status();
            }
            return;
        }
        let Some(pane) = &mut self.state.editor else {
            return;
        };
        match target {
            FileRequest::Binary => pane.editor.binary_path = path,
            FileRequest::MultipartNew => pane.editor.multipart.push(FormItem::file("file", path)),
            FileRequest::Setting(_) => return,
        }
        self.state.actions.push(Action::Edited {
            url: false,
            params: false,
        });
    }

    // ---- sending ------------------------------------------------------------------------------

    pub fn is_executing(&self) -> bool {
        self.state.execution.is_some()
    }

    fn send(&mut self) {
        if let Some(execution) = &self.state.execution {
            self.bridge.cancel(execution.task);
            return;
        }
        if self.state.close != CloseState::Open {
            return;
        }
        let Some(pane) = &self.state.editor else {
            return;
        };
        let request = match pane.editor.to_request() {
            Ok(request) => request,
            Err(InvalidOptions(errors)) => {
                if let Some(pane) = &mut self.state.editor {
                    pane.invalid = errors;
                    pane.tab = EditorTab::Settings;
                }
                self.state.response.notice = Some((
                    "Fix the highlighted request settings before sending".into(),
                    Tone::Error,
                ));
                return;
            }
        };
        let generation = self.state.generation;
        let db = self.db.clone();
        let task = self.bridge.spawn(move |cancel| async move {
            let result = executor::execute(&db, request, cancel).await;
            Message::Executed {
                generation,
                result: result.map(Box::new).map_err(NotSent::from),
            }
        });
        self.state.execution = Some(Execution {
            task,
            shown_at: None,
        });
        self.state.response.blocked = None;
        self.state.response.pending = true;
        // The pane says "Sending…" with the elapsed time while it runs.
        self.state.response.notice = None;
    }

    fn executed(&mut self, generation: u64, mut result: Result<Box<Executed>, NotSent>) {
        self.state.execution = None;
        self.state.response.pending = false;
        // Captured values are written even when the user has moved on: a login's token must
        // not depend on staying on the login request.
        let kept = match &mut result {
            Ok(executed) => self.write_captures(std::mem::take(&mut executed.writes)),
            Err(_) => None,
        };
        if self.smoke.as_ref().is_some_and(|s| s.sent) {
            let outcome = match &result {
                Ok(executed) if executed.response.outcome == ExchangeOutcome::Response => {
                    Ok(executed.response.status)
                }
                Ok(executed) => Err(format!(
                    "{:?} {}: {}",
                    executed.response.outcome,
                    executed.response.error_kind,
                    executed.response.reason
                )),
                Err(error) => Err(error.message.clone()),
            };
            return self.finish_smoke(outcome);
        }
        if self.state.history.loaded {
            self.load_history();
        }
        if generation != self.state.generation {
            if let Some(kept) = kept {
                self.status_for(&kept, Tone::Warning, 2.0 * STATUS_SECONDS);
            }
            return;
        }
        match result {
            Ok(executed) => {
                let response = executed.response;
                let (summary, tone) = if response.history_error.is_empty() {
                    crate::viewer::outcome_summary(&response)
                } else {
                    (response.history_error.clone(), Tone::Warning)
                };
                self.request_status(&summary, tone);
                self.state.response.shown_run = executed.record;
                self.state.response.show(response);
                self.state.response.sent = Some(
                    crate::viewer::sent_request_of(&executed.sent, executed.attach_cookies)
                        .as_text(),
                );
                if let Some(kept) = kept {
                    self.state.response.notice = Some((kept, Tone::Warning));
                }
                if let Some(id) = self.state.editor.as_ref().and_then(|p| p.editor.id()) {
                    self.load_runs(id);
                }
            }
            Err(not_sent) => {
                // The response pane says why and what to do; the status bar only that
                // nothing went out.
                self.request_status("Not sent", Tone::Error);
                self.state.response.notice = None;
                self.state.response.blocked = Some(not_sent);
            }
        }
    }

    fn show_run(&mut self, id: ExecutionId) {
        self.state.response.shown_run = Some(id);
        let generation = self.state.generation;
        self.task(
            move |db| executions::execution(db, id),
            move |result| Message::Run {
                generation,
                result: result.map(|record| record.map(Box::new)).map_err(failure),
            },
        );
    }

    // ---- tree and collection operations ---------------------------------------------------------

    fn named(&mut self, purpose: NamePurpose, name: String) {
        let reload = |result: Result<(), StoreError>| Message::Changed {
            reload_collections: false,
            result: result.map_err(failure),
        };
        match purpose {
            NamePurpose::NewCollection => self.navigate(Navigation::CreateCollection(name)),
            NamePurpose::NewRequest { folder } => {
                self.navigate(Navigation::CreateRequest { folder, name });
            }
            NamePurpose::NewFolder { parent } => {
                let Some(collection) = self.state.collection else {
                    return;
                };
                let db = self.db.clone();
                self.bridge.spawn(move |_| async move {
                    let result = ops::create_folder(&db, collection, parent, name)
                        .await
                        .map(drop);
                    reload(result)
                });
            }
            NamePurpose::RenameCollection(id) => self.task(
                move |db| collections::rename_collection(db, id, &name),
                |result| Message::Changed {
                    reload_collections: true,
                    result: result.map_err(failure),
                },
            ),
            NamePurpose::Rename(NodeId::Folder(id)) => {
                self.task(move |db| collections::rename_folder(db, id, &name), reload);
            }
            NamePurpose::Rename(NodeId::Request(id)) => {
                let new_name = name.clone();
                if let Some(pane) = &mut self.state.editor
                    && pane.editor.id() == Some(id)
                {
                    let folder = pane.editor.identity().folder_id;
                    pane.editor.adopt_metadata(&new_name, folder);
                }
                self.state
                    .saver
                    .adopt_metadata(id, |request| request.template.name.clone_from(&new_name));
                self.task(move |db| collections::rename_request(db, id, &name), reload);
            }
            NamePurpose::SaveDraft(collection) => self.save_draft(collection, name),
        }
    }

    /// Create or rename an environment and set its color, in one command.
    fn environment_named(
        &mut self,
        id: Option<EnvironmentId>,
        name: String,
        color: Option<EnvColor>,
    ) {
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = db
                .call(move |tx| {
                    let id = match id {
                        Some(id) => {
                            environments::rename_environment(tx, id, &name)?;
                            id
                        }
                        None => environments::create_environment(tx, &name, &Variables::new())?.id,
                    };
                    environments::set_environment_color(tx, id, color)?;
                    Ok((
                        id,
                        environments::environments(tx)?,
                        environments::active_environment(tx)?,
                    ))
                })
                .await;
            match result {
                Ok((id, list, active)) => Message::Environments {
                    select: Some(Scope::Environment(id)),
                    result: Ok((list, active)),
                },
                Err(error) => Message::Environments {
                    select: None,
                    result: Err(failure(error)),
                },
            }
        });
    }

    fn confirmed(&mut self, purpose: ConfirmPurpose) {
        match purpose {
            ConfirmPurpose::DeleteCollection(id) => {
                if self.state.editor.as_ref().is_some_and(|p| {
                    p.editor.identity().collection_id == Some(id) && p.editor.id().is_some()
                }) {
                    self.clear_editor();
                }
                self.state.collection = None;
                self.state.tree = None;
                self.task(
                    move |db| collections::delete_collection(db, id),
                    |result| Message::Changed {
                        reload_collections: true,
                        result: result.map_err(failure),
                    },
                );
            }
            ConfirmPurpose::Delete(node) => {
                let open = self.state.editor.as_ref().and_then(|p| p.editor.id());
                let affected = match node {
                    NodeId::Request(id) => open == Some(id),
                    NodeId::Folder(folder) => open.is_some_and(|open| {
                        self.state
                            .tree
                            .as_ref()
                            .is_some_and(|t| t.requests_under(folder).contains(&open))
                    }),
                };
                if affected {
                    self.clear_editor();
                }
                let reload = |result: Result<(), StoreError>| Message::Changed {
                    reload_collections: false,
                    result: result.map_err(failure),
                };
                match node {
                    NodeId::Folder(id) => {
                        self.task(move |db| collections::delete_folder(db, id), reload);
                    }
                    NodeId::Request(id) => {
                        self.task(move |db| collections::delete_request(db, id), reload);
                    }
                }
            }
            ConfirmPurpose::DeleteEnvironment(id) => {
                let select =
                    (self.state.variables.scope == Scope::Environment(id)).then_some(Scope::Global);
                self.task(
                    move |db| {
                        environments::delete_environment(db, id)?;
                        Ok((
                            environments::environments(db)?,
                            environments::active_environment(db)?,
                        ))
                    },
                    move |result| Message::Environments {
                        select,
                        result: result.map_err(failure),
                    },
                );
            }
            ConfirmPurpose::ClearHistory => self.task(
                |db| executions::clear(db),
                |result| match result {
                    Ok(()) => Message::HistoryPage {
                        generation: 0,
                        result: Ok(Vec::new()),
                    },
                    Err(error) => Message::HistoryPage {
                        generation: 0,
                        result: Err(failure(error)),
                    },
                },
            ),
            ConfirmPurpose::ClearVariables(scope) => {
                let screen = &mut self.state.variables;
                if screen.scope == scope {
                    screen.rows.clear();
                    screen.secrets.clear();
                    screen.status = None;
                }
            }
            ConfirmPurpose::RevertVariables(scope) => {
                if self.state.variables.scope == scope {
                    self.revert_variables();
                }
            }
            ConfirmPurpose::DiscardOnClose => {
                self.state.saver.discard(None);
                self.state.close = CloseState::Ready;
            }
            ConfirmPurpose::PostmanExport => self.pick_postman_destination(),
            // Through the actions, which carry the time the run starts at.
            ConfirmPurpose::StartLoad => {
                self.state
                    .actions
                    .push(Action::Load(crate::load_screen::LoadAction::Start {
                        confirmed: true,
                    }));
            }
            ConfirmPurpose::StartRun => {
                self.run_action(crate::run_screen::RunAction::Start { confirmed: true });
            }
        }
    }

    fn move_node(&mut self, node: NodeId, target: Option<FolderId>, index: usize) {
        if let (NodeId::Folder(id), Some(target)) = (node, target)
            && id == target
        {
            return;
        }
        let reload = |result: Result<(), StoreError>| Message::Changed {
            reload_collections: false,
            result: result.map_err(failure),
        };
        if let NodeId::Request(id) = node {
            if let Some(pane) = &mut self.state.editor
                && pane.editor.id() == Some(id)
            {
                let name = pane.editor.name().to_owned();
                pane.editor.adopt_metadata(&name, target);
            }
            self.state
                .saver
                .adopt_metadata(id, |request| request.folder_id = target);
        }
        self.task(
            move |db| collections::place(db, node, target, index),
            reload,
        );
    }

    // ---- actions ----------------------------------------------------------------------------------

    pub fn handle(&mut self, action: Action, now: f64) {
        match action {
            Action::SelectCollection(id) if Some(id) != self.state.collection => {
                self.navigate(Navigation::SelectCollection(id));
            }
            Action::SelectCollection(_) => {}
            Action::Select(node) => self.state.selected = Some(node),
            Action::OpenRequest(id)
                if self.state.editor.as_ref().and_then(|p| p.editor.id()) == Some(id) => {}
            Action::OpenRequest(id) => self.navigate(Navigation::Open(id)),
            Action::DuplicateRequest(id) => {
                let edited = if let Some(pane) = &mut self.state.editor
                    && pane.editor.id() == Some(id)
                {
                    match pane.editor.to_request() {
                        Ok(request) => Some(Box::new(request.template)),
                        Err(InvalidOptions(errors)) => {
                            pane.invalid = errors;
                            pane.tab = EditorTab::Settings;
                            return;
                        }
                    }
                } else {
                    None
                };
                self.navigate(Navigation::Duplicate { id, edited });
            }
            Action::Prompt(dialog) => {
                if matches!(dialog, Dialog::Palette(_)) {
                    self.load_request_index();
                }
                self.state.dialog = Some(dialog);
            }
            Action::OpenIn {
                collection,
                request,
            } => {
                self.state.screen = Screen::Requests;
                if self.state.editor.as_ref().and_then(|p| p.editor.id()) != Some(request) {
                    self.navigate(Navigation::OpenIn {
                        collection,
                        request,
                    });
                }
            }
            Action::PasteCurl(command) => self.paste_curl(&command, now),
            Action::FormatBody => {
                let formatted = self.state.editor.as_mut().and_then(|pane| {
                    let editor = &mut pane.editor;
                    let json = editor.body_type == imposter_app::core::model::BodyType::Json;
                    let formatted = crate::viewer::format_json_body(&editor.body_text)
                        .filter(|f| json && *f != editor.body_text)?;
                    editor.body_text = formatted;
                    pane.tab = EditorTab::Body;
                    Some(())
                });
                if formatted.is_some() {
                    self.edited(false, false, now);
                }
            }
            Action::AddCheck(check) => {
                if let Some(pane) = &mut self.state.editor {
                    pane.editor.checks.push(check);
                    pane.tab = EditorTab::Tests;
                    self.edited(false, false, now);
                }
            }
            Action::AddCapture(capture) => {
                if let Some(pane) = &mut self.state.editor {
                    pane.editor.captures.push(capture);
                    pane.tab = EditorTab::Captures;
                    self.edited(false, false, now);
                }
            }
            Action::ToggleSidebar => {
                self.state.layout.sidebar_visible = !self.state.layout.sidebar_visible;
            }
            Action::SetAppearance(appearance) => {
                self.state.appearance = appearance;
                let stored = serde_json::to_value(appearance).unwrap_or(Value::Null);
                self.write_ui_state(APPEARANCE_KEY, stored);
            }
            Action::ToggleLayout => {
                self.state.layout.orientation = self.state.layout.orientation.switched();
            }
            Action::Move {
                node,
                target,
                index,
            } => self.move_node(node, target, index),
            Action::Save => match self.state.screen {
                Screen::Variables => self.save_variables(),
                Screen::Settings => self.save_settings(),
                Screen::Mocks => self.save_mocks(),
                Screen::Requests if self.state.guard.is_none() => self.save_current(),
                Screen::Requests | Screen::History | Screen::Load | Screen::Run => {}
            },
            Action::Mock(action) => self.mock_action(action),
            Action::Load(action) => self.load_action(action, now),
            Action::LoadRequest(id) => {
                if let Some(target) = crate::load_screen::Target::from_tree(&self.state, id) {
                    self.handle(Action::LoadTarget(target), now);
                }
            }
            Action::LoadTarget(target) => {
                if self.state.load.run.is_none() {
                    self.state.load.target = Some(target);
                    self.state.load.error = None;
                }
                self.state.screen = Screen::Load;
            }
            Action::Run(action) => self.run_action(action),
            Action::RunFolder(folder) => {
                if let Some(target) = crate::run_screen::RunTarget::from_tree(&self.state, folder) {
                    self.handle(Action::RunTarget(target), now);
                }
            }
            Action::RunTarget(target) => {
                if self.state.run.run.is_none() {
                    self.state.run.target = Some(target);
                }
                self.state.screen = Screen::Run;
            }
            Action::SettingsEdited => self.state.settings_screen.refresh_status(),
            Action::CheckPath(path) => {
                self.state.settings_screen.paths.insert(path.clone(), None);
                self.bridge.spawn(move |_| async move {
                    let exists = transfer::is_file(PathBuf::from(&path)).await;
                    Message::PathChecked { path, exists }
                });
            }
            Action::CopyCurl => self.copy_curl(),
            Action::Send => self.send(),
            Action::PickFile(target) => self.pick(
                FileDialog::Open {
                    title: "Choose upload file",
                    filter: None,
                },
                move |paths| Message::FilePicked {
                    target,
                    path: paths.into_iter().next(),
                },
            ),
            Action::Import => self.pick(
                FileDialog::Open {
                    title: "Import collection",
                    filter: Some(("Collections and OpenAPI", &["json", "yaml", "yml"])),
                },
                |paths| Message::ImportPicked(paths.into_iter().next()),
            ),
            Action::Export => self.prompt_export(),
            Action::SaveBody { bytes, name, cut } => self.pick(
                FileDialog::Save {
                    title: "Save response body",
                    name,
                    filter: None,
                },
                move |paths| Message::BodySavePicked {
                    path: paths.into_iter().next(),
                    bytes,
                    cut,
                },
            ),
            Action::ShowRun(id) => self.show_run(id),
            Action::Edited { url, params } => self.edited(url, params, now),
            Action::Screen(screen) => {
                self.state.screen = screen;
                if screen == Screen::Mocks && !self.state.mocks.loaded() {
                    self.load_mocks();
                }
                if screen == Screen::History && !self.state.history.loaded {
                    self.load_history();
                }
                if screen == Screen::Variables && !self.state.variables.loaded {
                    let scope = self.state.variables.scope;
                    self.load_variables(scope);
                }
                if screen == Screen::Settings {
                    // Files may have come or gone since the paths were last checked.
                    self.state.settings_screen.paths.clear();
                }
                if screen == Screen::Settings && !self.state.settings_screen.is_loaded() {
                    self.load_settings();
                }
            }
            Action::SetEnvironment(id) => self.set_environment(id),
            Action::ManageEnvironments => {
                let scope = self
                    .state
                    .active_environment
                    .map_or(Scope::Global, Scope::Environment);
                if self.state.variables.scope == scope && self.state.variables.loaded {
                    // Already there, unsaved edits included.
                    self.state.screen = Screen::Variables;
                } else {
                    self.variables_navigate(VarTransition::Open(scope));
                }
            }
            Action::OpenDefinition(scope, name) => {
                self.variables_navigate(VarTransition::OpenDefinition(scope, name));
            }
            Action::DefineVariable(scope, name) => {
                self.variables_navigate(VarTransition::Define(scope, name));
            }
            Action::ChooseValue { scope, name, value } => self.choose_value(scope, name, value),
            Action::Variables(action) => self.variables_action(action),
            Action::History(action) => self.history_action(action),
        }
    }

    fn dialog_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Open => {}
            Outcome::Closed => {
                self.state.dialog = None;
                if self
                    .state
                    .guard
                    .as_ref()
                    .is_some_and(|g| g.stage == GuardStage::NamingDraft)
                {
                    self.cancel_guard();
                }
            }
            Outcome::Name(purpose, name) => {
                self.state.dialog = None;
                self.named(purpose, name);
            }
            Outcome::Environment { id, name, color } => {
                self.state.dialog = None;
                self.environment_named(id, name, color);
            }
            Outcome::Confirmed(purpose) => {
                self.state.dialog = None;
                self.confirmed(purpose);
            }
            Outcome::Guard(choice) => {
                self.state.dialog = None;
                self.guard_choice(choice);
            }
            Outcome::VariablesGuard(choice) => {
                self.state.dialog = None;
                self.variables_choice(choice);
            }
            Outcome::MocksGuard(choice) => {
                self.state.dialog = None;
                self.mocks_choice(choice);
            }
            Outcome::SettingsGuard(choice) => {
                self.state.dialog = None;
                self.settings_choice(choice);
            }
            Outcome::Move(node, target) => {
                self.state.dialog = None;
                self.move_node(node, target, usize::MAX);
            }
            Outcome::Companions(include) => {
                let Some(Dialog::Companions { path }) = self.state.dialog.take() else {
                    return;
                };
                if include {
                    self.pick(
                        FileDialog::OpenMany {
                            title: "Select companion files",
                        },
                        move |companions| Message::CompanionsPicked { path, companions },
                    );
                } else {
                    self.read_import(path, Vec::new());
                }
            }
            Outcome::Import => {
                if let Some(Dialog::Import {
                    preview,
                    remove_unsupported,
                }) = self.state.dialog.take()
                {
                    self.navigate(Navigation::Import {
                        preview,
                        remove_unsupported,
                    });
                }
            }
            Outcome::Palette => {
                if let Some(Dialog::Palette(palette)) = self.state.dialog.take()
                    && let Some(action) = palette.chosen
                {
                    self.state.actions.push(action);
                }
            }
            Outcome::Scripts => {
                if let Some(Dialog::Scripts {
                    target, scripts, ..
                }) = self.state.dialog.take()
                {
                    self.save_scripts(target, scripts);
                }
            }
            Outcome::Curl => {
                if let Some(Dialog::Curl { text }) = self.state.dialog.take()
                    && let Ok(parsed) = imposter_app::core::curl::parse(&text)
                {
                    let request = Request {
                        template: parsed.template,
                        ..Request::default()
                    };
                    self.navigate(Navigation::Restore {
                        request: Box::new(request),
                        status: "Opened the curl command as a draft",
                        response: None,
                    });
                }
            }
            Outcome::Export => {
                if let Some(Dialog::Export {
                    environments,
                    globals,
                    postman,
                }) = self.state.dialog.take()
                {
                    let chosen = environments
                        .into_iter()
                        .filter(|(_, _, include)| *include)
                        .map(|(id, _, _)| id)
                        .collect();
                    self.export(chosen, globals, postman);
                }
            }
        }
    }

    // ---- import and export ----------------------------------------------------------------------

    /// The open request as curl, resolved the way Send would resolve it now.
    fn copy_curl(&mut self) {
        let Some(pane) = &self.state.editor else {
            return;
        };
        let Ok(request) = pane.editor.to_request() else {
            return self.error(
                "Copy as curl failed",
                "Fix the highlighted request settings first.",
            );
        };
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = executor::curl_with_identity_notice(&db, &request).await;
            Message::Curl(result.map_err(failure))
        });
    }

    fn load_request_index(&mut self) {
        self.task(
            |db| collections::request_index(db),
            |result| Message::RequestIndex(result.map_err(failure)),
        );
    }

    fn read_import(&mut self, path: PathBuf, companions: Vec<PathBuf>) {
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = transfer::read_preview(&db, path, companions).await;
            Message::ImportPreviewed(result.map(Box::new).map_err(failure))
        });
    }

    /// Show the imported collection and reload the variables it may have added.
    fn imported(&mut self, id: CollectionId) {
        self.reload_collections(Some(id));
        let screen = &self.state.variables;
        let select = (screen.loaded && !screen.dirty()).then_some(screen.scope);
        self.task(
            |db| {
                Ok((
                    environments::environments(db)?,
                    environments::active_environment(db)?,
                ))
            },
            move |result| Message::Environments {
                select,
                result: result.map_err(failure),
            },
        );
        self.status("Collection imported");
    }

    fn prompt_export(&mut self) {
        if self.state.collection.is_none() {
            self.state.dialog = Some(Dialog::info(
                "Export collection",
                "Select a collection first",
            ));
            return;
        }
        self.state.dialog = Some(Dialog::Export {
            environments: self
                .state
                .environments
                .iter()
                .map(|e| (e.id, e.name.clone(), false))
                .collect(),
            globals: false,
            postman: false,
        });
    }

    /// Export the selected collection as the editor shows it: unsaved edits of the open
    /// request are included, and pending autosaves are written too.
    fn export(&mut self, environments: Vec<EnvironmentId>, globals: bool, postman: bool) {
        let Some(collection) = self.state.collection else {
            return;
        };
        let draft = self
            .state
            .editor
            .as_ref()
            .and_then(|pane| pane.editor.to_request().ok());
        if self.state.settings.autosave {
            self.state.autosave_due = None;
            self.pump_saves(None);
        }
        let scope = ExportScope {
            collection: Some(collection),
            environments,
            globals,
            draft,
        };
        if postman {
            let db = self.db.clone();
            self.bridge.spawn(move |_| async move {
                let result =
                    transfer::postman_export(&db, scope)
                        .await
                        .map(|(document, data, warnings)| {
                            Box::new(PostmanExport {
                                document,
                                data,
                                warnings,
                            })
                        });
                Message::PostmanReady(result.map_err(failure))
            });
        } else {
            let scope = Box::new(scope);
            self.pick(native_export_dialog(), move |paths| {
                Message::NativeExportPicked {
                    scope,
                    path: paths.into_iter().next(),
                }
            });
        }
    }

    /// A bundle with environments or globals goes to a new directory, a lone collection to a
    /// file.
    fn pick_postman_destination(&mut self) {
        let Some(export) = &self.state.postman else {
            return;
        };
        let bundle =
            !export.document.environments.is_empty() || !export.document.globals.is_empty();
        let dialog = if bundle {
            FileDialog::Save {
                title: "New destination directory",
                name: "collection-export",
                filter: None,
            }
        } else {
            FileDialog::Save {
                title: "Export Postman collection",
                name: "collection.postman_collection.json",
                filter: Some(("Postman", "json")),
            }
        };
        self.pick(dialog, |paths| {
            Message::PostmanExportPicked(paths.into_iter().next())
        });
    }

    // ---- variables ------------------------------------------------------------------------------

    /// Reload what the open request's variables resolve to (after a load, an environment switch
    /// or saved variables). Uses the executor's own snapshot code.
    fn load_snapshot(&mut self) {
        let Some(request) = self.state.editor.as_ref().map(|p| {
            p.editor
                .to_request()
                .unwrap_or_else(|_| p.editor.identity().clone())
        }) else {
            self.state.snapshot = None;
            return;
        };
        self.state.snapshot_generation += 1;
        let generation = self.state.snapshot_generation;
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = executor::inputs(&db, &request)
                .await
                .map(|inputs| Box::new(inputs.snapshot))
                .map_err(failure);
            Message::Snapshot { generation, result }
        });
    }

    fn set_environment(&mut self, id: Option<EnvironmentId>) {
        if id == self.state.active_environment {
            return;
        }
        self.state.active_environment = id;
        self.task(
            move |db| {
                environments::set_active_environment(db, id)?;
                Ok((
                    environments::environments(db)?,
                    environments::active_environment(db)?,
                ))
            },
            |result| Message::Environments {
                select: None,
                result: result.map_err(failure),
            },
        );
        let name = id
            .and_then(|id| self.state.environments.iter().find(|e| e.id == id))
            .map_or("Global (no environment)".to_owned(), |e| e.name.clone());
        self.status(&format!("Environment: {name}"));
    }

    fn load_variables(&mut self, scope: Scope) {
        self.state.variables.scope = scope;
        self.state.variables.loaded = false;
        self.task(
            move |db| {
                let (variables, saved) = scopes::read(db, scope)?;
                let secrets = match SecretScope::of_scope(scope) {
                    Some(scope) => secrets::read(db, scope)?,
                    None => Variables::new(),
                };
                Ok((variables, saved, secrets))
            },
            move |result| Message::VariablesLoaded {
                scope,
                result: result.map_err(failure),
            },
        );
    }

    fn variables_loaded(
        &mut self,
        scope: Scope,
        result: Result<(Variables, SavedValues, Variables), Failure>,
    ) {
        let screen = &mut self.state.variables;
        if scope != screen.scope {
            return;
        }
        match result {
            Ok((variables, saved, secrets)) => {
                screen.rows = vars::rows(&variables, &saved);
                screen.baseline = vars::entries(&screen.rows);
                screen.secrets = vars::rows(&secrets, &SavedValues::default());
                screen.secret_baseline = vars::entries(&screen.secrets);
                screen.loaded = true;
                screen.status = None;
                if let Some(name) = screen.define.take() {
                    // "Define {{secret:x}}" adds a secret; any other name, a variable.
                    let (rows, row_name) = match imposter_app::core::variables::secret_name(&name) {
                        Some(secret) if SecretScope::of_scope(scope).is_some() => {
                            (&mut screen.secrets, secret.to_owned())
                        }
                        _ => (&mut screen.rows, name.clone()),
                    };
                    if !rows.iter().any(|row| row.name == row_name) {
                        rows.push(vars::Row::new(row_name, ""));
                    }
                    screen.highlight = Some(name);
                }
            }
            Err(error) => {
                screen.status = Some(SaveWord::Unavailable);
                self.request_status(
                    &format!("Variables could not be loaded: {error}"),
                    Tone::Error,
                );
            }
        }
    }

    /// Run `transition` now if the Variables screen has no unsaved rows, otherwise ask first.
    fn variables_navigate(&mut self, transition: VarTransition) {
        if self.state.variables.dirty() {
            self.state.variables.pending = Some(transition);
            self.state.dialog = Some(Dialog::VariablesGuard);
        } else {
            self.variables_perform(transition);
        }
    }

    fn variables_perform(&mut self, transition: VarTransition) {
        match transition {
            VarTransition::Scope(scope) => self.load_variables(scope),
            VarTransition::NewEnvironment => {
                self.state.dialog = Some(Dialog::new_environment());
            }
            VarTransition::RenameEnvironment(id) => {
                let color = self
                    .state
                    .environments
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.color);
                self.state.dialog = Some(Dialog::rename_environment(
                    id,
                    &self.environment_name(id),
                    color,
                ));
            }
            VarTransition::DeleteEnvironment(id) => {
                let current = self.environment_name(id);
                let variables = self
                    .state
                    .environments
                    .iter()
                    .find(|e| e.id == id)
                    .map_or(0, |e| e.variables.len());
                self.state.dialog = Some(Dialog::delete_environment(id, &current, variables));
            }
            VarTransition::Open(scope) => {
                self.state.screen = Screen::Variables;
                self.load_variables(scope);
            }
            VarTransition::OpenDefinition(scope, name) => {
                self.state.screen = Screen::Variables;
                self.state.variables.highlight = Some(name);
                self.load_variables(scope);
            }
            VarTransition::Define(scope, name) => {
                self.state.screen = Screen::Variables;
                self.state.variables.define = Some(name);
                self.load_variables(scope);
            }
            VarTransition::Close => self.settings_close(),
        }
    }

    /// A variables scope in a sentence: "Global", "the Dev environment", "the API collection".
    fn scope_phrase(&self, scope: Scope) -> String {
        match scope {
            Scope::Global => "Global".to_owned(),
            Scope::Environment(id) => format!("the {} environment", self.environment_name(id)),
            Scope::Collection(id) => self
                .state
                .collections
                .iter()
                .find(|c| c.id == id)
                .map_or_else(
                    || "the collection".to_owned(),
                    |c| format!("the {} collection", c.name),
                ),
        }
    }

    fn environment_name(&self, id: EnvironmentId) -> String {
        self.state
            .environments
            .iter()
            .find(|e| e.id == id)
            .map_or_else(String::new, |e| e.name.clone())
    }

    fn variables_choice(&mut self, choice: GuardChoice) {
        let Some(transition) = self.state.variables.pending.take() else {
            return;
        };
        match choice {
            GuardChoice::Save => {
                self.state.variables.pending = Some(transition);
                self.save_variables();
            }
            GuardChoice::Discard => {
                self.revert_variables();
                self.variables_perform(transition);
            }
            GuardChoice::Cancel => {
                if transition == VarTransition::Close {
                    self.state.close = CloseState::Open;
                }
            }
        }
    }

    fn revert_variables(&mut self) {
        let screen = &mut self.state.variables;
        let rows = |baseline: &[vars::Entry]| -> Vec<vars::Row> {
            baseline
                .iter()
                .map(|(name, value, saved)| {
                    vars::Row::with_saved(name.clone(), value.clone(), saved.clone())
                })
                .collect()
        };
        screen.rows = rows(&screen.baseline);
        screen.secrets = rows(&screen.secret_baseline);
        screen.status = None;
    }

    /// Swap a variable's current value from the inspector. The Variables screen shows the new
    /// value at once when it is on that scope; with unsaved edits there the inspector does not
    /// offer the swap, so a later Save cannot quietly undo it.
    fn choose_value(&mut self, scope: Scope, name: String, value: String) {
        let screen = &self.state.variables;
        if screen.dirty() && screen.scope == scope {
            return;
        }
        let chosen = name.clone();
        self.task(
            move |db| scopes::choose_value(db, scope, &chosen, &value),
            move |result| Message::ValueChosen {
                scope,
                name,
                result: result.map_err(failure),
            },
        );
    }

    /// Write captured values in the background. Values for the scope the Variables screen
    /// has unsaved edits to are not written, so its Save cannot quietly undo them, nor they
    /// the edits; the returned message says which.
    fn write_captures(
        &mut self,
        writes: Vec<imposter_app::core::capture::Write>,
    ) -> Option<String> {
        let screen = &self.state.variables;
        let blocked = screen.dirty().then_some(screen.scope);
        let (kept, writes): (Vec<_>, Vec<_>) = writes
            .into_iter()
            .partition(|w| w.scope.is_some() && w.scope == blocked);
        if !writes.is_empty() {
            // The pages whose rows change, secrets included, so a clean one reloads.
            let mut scopes: Vec<Scope> = Vec::new();
            for scope in writes.iter().filter_map(|w| w.scope) {
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
            // Names only: `secret:token` says a secret was stored without its value.
            let names: Vec<String> = writes
                .iter()
                .map(imposter_app::core::capture::Write::key)
                .collect();
            let db = self.db.clone();
            self.bridge.spawn(move |_| async move {
                Message::CapturesWritten {
                    scopes,
                    names,
                    result: executor::write_captures(&db, writes).await.map_err(failure),
                }
            });
        }
        (!kept.is_empty()).then(|| {
            let names: Vec<String> = kept
                .iter()
                .map(imposter_app::core::capture::Write::key)
                .collect();
            format!(
                "Not set: {} — the Variables screen has unsaved changes to that scope",
                names.join(", ")
            )
        })
    }

    fn captures_written(
        &mut self,
        scopes: &[Scope],
        names: &[String],
        result: Result<(), Failure>,
    ) {
        match result {
            // Values can be secrets, so the status names the variables alone.
            Ok(()) => {
                let set = format!("set {}", names.join(", "));
                let (generation, screen) = (self.state.generation, self.state.screen);
                // Beside the send's result when that still shows, so neither hides the other.
                match &mut self.state.status {
                    Some(line) if line.lasting == (Lasting::While { generation, screen }) => {
                        line.text = format!("{} · {set}", line.text);
                    }
                    _ => self.status(&format!("Set {}", names.join(", "))),
                }
                let screen = &self.state.variables;
                if screen.loaded && scopes.contains(&screen.scope) && !screen.dirty() {
                    self.load_variables(screen.scope);
                }
                self.load_snapshot();
            }
            Err(error) => self.error("Captured values were not saved", error),
        }
    }

    fn value_chosen(&mut self, scope: Scope, name: &str, result: Result<(), Failure>) {
        match result {
            // Values appear only in the inspector, so the status names the variable alone.
            Ok(()) => {
                self.status(&format!("{name}: saved value chosen"));
                let screen = &self.state.variables;
                if screen.loaded && screen.scope == scope && !screen.dirty() {
                    self.load_variables(scope);
                }
                self.load_snapshot();
            }
            Err(error) => self.error("Value not changed", error),
        }
    }

    fn save_variables(&mut self) {
        let screen = &mut self.state.variables;
        if !screen.loaded || screen.saving {
            return;
        }
        let checked = vars::to_scope(&screen.rows)
            .map_err(|e| e.to_string())
            .and_then(|scope| Ok((scope, vars::to_secrets(&screen.secrets)?)));
        let ((variables, saved), secret_values) = match checked {
            Ok(checked) => checked,
            Err(error) => {
                screen.status = Some(SaveWord::Invalid);
                if screen.pending.take() == Some(VarTransition::Close) {
                    self.state.close = CloseState::Open;
                }
                self.request_status(&error, Tone::Error);
                return;
            }
        };
        screen.saving = true;
        let scope = screen.scope;
        self.task(
            move |db| {
                scopes::write(db, scope, &variables, &saved)?;
                match SecretScope::of_scope(scope) {
                    Some(scope) => secrets::write(db, scope, &secret_values),
                    None => Ok(()),
                }
            },
            move |result| Message::VariablesSaved {
                scope,
                result: result.map_err(failure),
            },
        );
    }

    fn variables_saved(&mut self, scope: Scope, result: Result<(), Failure>) {
        let screen = &mut self.state.variables;
        screen.saving = false;
        match result {
            Ok(()) => {
                if scope == screen.scope {
                    screen.baseline = vars::entries(&screen.rows);
                    screen.secret_baseline = vars::entries(&screen.secrets);
                    screen.status = None;
                }
                if let Some(transition) = screen.pending.take() {
                    self.variables_perform(transition);
                }
                self.load_snapshot();
            }
            Err(error) => {
                screen.status = Some(SaveWord::Failed);
                if screen.pending.take() == Some(VarTransition::Close) {
                    self.state.close = CloseState::Open;
                }
                self.request_status(
                    &format!("Variables not saved, changes kept: {error}"),
                    Tone::Error,
                );
            }
        }
    }

    fn variables_action(&mut self, action: VariablesAction) {
        match action {
            VariablesAction::SelectScope(scope) => {
                if scope != self.state.variables.scope || !self.state.variables.loaded {
                    self.variables_navigate(VarTransition::Scope(scope));
                }
            }
            VariablesAction::Save => self.save_variables(),
            VariablesAction::Revert => {
                let scope = self.state.variables.scope;
                if self.state.variables.dirty() {
                    self.state.dialog =
                        Some(Dialog::revert_variables(scope, &self.scope_phrase(scope)));
                }
            }
            VariablesAction::ClearAll => {
                let scope = self.state.variables.scope;
                let count = self.state.variables.rows.len() + self.state.variables.secrets.len();
                if count > 0 {
                    self.state.dialog = Some(Dialog::clear_variables(
                        scope,
                        &self.scope_phrase(scope),
                        count,
                    ));
                }
            }
            VariablesAction::Edited => {
                self.state.variables.status = None;
            }
            VariablesAction::NewEnvironment => {
                self.variables_navigate(VarTransition::NewEnvironment);
            }
            VariablesAction::RenameEnvironment(id) => {
                self.variables_navigate(VarTransition::RenameEnvironment(id));
            }
            VariablesAction::DeleteEnvironment(id) => {
                self.variables_navigate(VarTransition::DeleteEnvironment(id));
            }
        }
    }

    pub fn variables_dirty(&self) -> bool {
        self.state.variables.dirty()
    }

    // ---- settings -------------------------------------------------------------------------------

    fn load_settings(&mut self) {
        self.load_usage();
        let screen = &mut self.state.settings_screen;
        if screen.loading {
            return;
        }
        screen.loading = true;
        self.task(
            |db| kv::app_settings(db),
            |result| Message::SettingsLoaded(result.map_err(failure)),
        );
    }

    fn settings_loaded(&mut self, result: Result<AppSettings, Failure>) {
        let screen = &mut self.state.settings_screen;
        screen.loading = false;
        match result {
            Ok(settings) => {
                screen.form = SettingsForm::new(&settings);
                screen.baseline = Some(screen.form.clone());
                screen.base = settings;
                screen.refresh_status();
            }
            Err(error) => {
                screen.status = Some(SaveWord::Unavailable);
                self.request_status(
                    &format!("Settings could not be loaded: {error}"),
                    Tone::Error,
                );
            }
        }
    }

    /// Save the form; history retention applies to the new limits at once.
    fn save_settings(&mut self) {
        let screen = &mut self.state.settings_screen;
        if !screen.is_loaded() || screen.saving {
            return;
        }
        let form = screen.form.clone();
        let settings = form.apply(&screen.base);
        if let Err(error) = settings.validate() {
            screen.status = Some(SaveWord::Invalid);
            self.request_status(&format!("Settings not saved: {error}"), Tone::Error);
            self.settings_close_failed();
            return;
        }
        screen.saving = true;
        self.task(
            move |db| {
                kv::set_app_settings(db, &settings)?;
                executions::enforce_retention(
                    db,
                    settings.history_max_entries,
                    settings.history_max_bytes,
                )?;
                Ok(settings)
            },
            move |result| Message::SettingsSaved {
                form,
                result: result.map_err(failure),
            },
        );
    }

    fn settings_saved(&mut self, form: SettingsForm, result: Result<AppSettings, Failure>) {
        let screen = &mut self.state.settings_screen;
        screen.saving = false;
        match result {
            Ok(settings) => {
                screen.baseline = Some(form);
                screen.base = settings.clone();
                screen.status = None;
                if !settings.autosave {
                    self.state.autosave_due = None;
                }
                self.state.settings = settings;
                // Retention may have dropped runs.
                if self.state.history.loaded {
                    self.load_history();
                } else {
                    self.load_usage();
                }
                if std::mem::take(&mut self.state.settings_screen.closing) {
                    self.mocks_transition(crate::mock_screen::Transition::Close);
                }
            }
            Err(error) => {
                screen.status = Some(SaveWord::Failed);
                self.request_status(&format!("Settings not saved: {error}"), Tone::Error);
                self.settings_close_failed();
            }
        }
    }

    /// Resolve settings before the mock configuration guard and final UI-state write.
    fn settings_close(&mut self) {
        if self.state.settings_screen.dirty() {
            self.state.settings_screen.closing = true;
            self.state.dialog = Some(Dialog::SettingsGuard);
        } else {
            self.mocks_transition(crate::mock_screen::Transition::Close);
        }
    }

    fn settings_close_failed(&mut self) {
        if std::mem::take(&mut self.state.settings_screen.closing) {
            self.state.close = CloseState::Open;
        }
    }

    fn settings_choice(&mut self, choice: GuardChoice) {
        match choice {
            GuardChoice::Save => self.save_settings(),
            GuardChoice::Discard => {
                let screen = &mut self.state.settings_screen;
                if let Some(baseline) = &screen.baseline {
                    screen.form = baseline.clone();
                }
                screen.refresh_status();
                screen.closing = false;
                self.mocks_transition(crate::mock_screen::Transition::Close);
            }
            GuardChoice::Cancel => self.settings_close_failed(),
        }
    }

    fn load_mocks(&mut self) {
        if self.state.mocks.loading || self.state.mocks.saving {
            return;
        }
        self.state.mocks.loading = true;
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            Message::MocksLoaded(imposter_app::mock::load(&db).await.map_err(failure))
        });
    }

    fn mocks_loaded(&mut self, result: Result<imposter_app::core::mock::MockConfig, Failure>) {
        let screen = &mut self.state.mocks;
        screen.loading = false;
        match result {
            Ok(config) => {
                screen.form = config.clone();
                screen.baseline = Some(config);
                screen.error = None;
            }
            Err(error) => screen.error = Some(error),
        }
    }

    fn save_mocks(&mut self) {
        let screen = &mut self.state.mocks;
        if !screen.loaded() || screen.saving {
            return;
        }
        let config = screen.form.clone();
        if let Err(error) = config.validate() {
            screen.error = Some(error.to_string());
            self.mocks_cancel_transition();
            return;
        }
        screen.saving = true;
        screen.error = None;
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = imposter_app::mock::save(&db, config.clone())
                .await
                .map_err(failure);
            Message::MocksSaved { config, result }
        });
    }

    fn mocks_saved(
        &mut self,
        config: imposter_app::core::mock::MockConfig,
        result: Result<(), Failure>,
    ) {
        let screen = &mut self.state.mocks;
        screen.saving = false;
        match result {
            Ok(()) => {
                screen.baseline = Some(config);
                screen.error = None;
                if let Some(next) = screen.pending.take() {
                    self.mocks_transition(next);
                }
            }
            Err(error) => {
                screen.error = Some(error);
                self.mocks_cancel_transition();
            }
        }
    }

    fn mock_action(&mut self, action: crate::mock_screen::MockAction) {
        use crate::mock_screen::MockAction;
        match action {
            MockAction::Edited => self.state.mocks.error = None,
            MockAction::Reload => self.mocks_transition(crate::mock_screen::Transition::Reload),
            MockAction::Stop => {
                if let Some(run) = &mut self.state.mocks.run {
                    run.stopping = true;
                    self.bridge.cancel(run.task);
                }
            }
            MockAction::Start => {
                let screen = &mut self.state.mocks;
                if self.state.close != CloseState::Open
                    || !screen.loaded()
                    || screen.dirty()
                    || screen.saving
                    || screen.loading
                    || screen.run.is_some()
                {
                    return;
                }
                let config = screen
                    .baseline
                    .clone()
                    .expect("loaded mock screen has a baseline");
                screen.generation += 1;
                let generation = screen.generation;
                let notify = self.bridge.notifier();
                // The log is called from the server's threads; the notifier is not `Sync`.
                let log = std::sync::Mutex::new(self.bridge.notifier());
                let run_config = config.clone();
                let task = self.bridge.spawn(move |cancel| async move {
                    let result = imposter_app::mock::run(
                        run_config,
                        move |address| {
                            notify(Message::MockListening {
                                generation,
                                address: format!("http://{address}"),
                            });
                        },
                        move |entry| {
                            let notify = log
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            notify(Message::MockLog {
                                generation,
                                entry: Box::new(entry),
                            });
                        },
                        cancel,
                    )
                    .await
                    .map_err(failure);
                    Message::MockStopped { generation, result }
                });
                screen.notice = None;
                screen.log.clear();
                screen.run = Some(crate::mock_screen::Run {
                    task,
                    config,
                    address: None,
                    stopping: false,
                });
            }
        }
    }

    // ---- load tests ------------------------------------------------------------------------

    fn load_action(&mut self, action: crate::load_screen::LoadAction, now: f64) {
        use crate::load_screen::LoadAction;
        match action {
            LoadAction::Choose => {
                let palette =
                    crate::palette::Palette::open(&self.state, crate::palette::Mode::LoadTarget);
                self.state.dialog = Some(Dialog::Palette(Box::new(palette)));
                self.load_request_index();
            }
            LoadAction::Show(index) => {
                if index < self.state.load.runs.len() {
                    self.state.load.shown = index;
                }
            }
            LoadAction::Stop => {
                if let Some(run) = &mut self.state.load.run {
                    run.stopping = true;
                    self.bridge.cancel(run.task);
                }
            }
            LoadAction::Start { confirmed } => self.load_start(confirmed, now),
        }
    }

    fn load_start(&mut self, confirmed: bool, now: f64) {
        if self.state.close != CloseState::Open || self.state.load.run.is_some() {
            return;
        }
        let Some(target) = self.state.load.target.clone() else {
            return;
        };
        let plan = match self.state.load.form.plan() {
            Ok(plan) => plan,
            Err(reason) => {
                self.state.load.error = Some(reason);
                return;
            }
        };
        // Many requests to production are worth a second look.
        let environment = self
            .state
            .active_environment
            .and_then(|id| self.state.environments.iter().find(|e| e.id == id));
        if !confirmed
            && let Some(environment) = environment.filter(|e| e.color == Some(EnvColor::Red))
        {
            self.state.dialog = Some(Dialog::start_load(
                &environment.name,
                plan.requests(),
                &target.name,
            ));
            return;
        }
        // What Send would send now when it is open; otherwise as stored.
        let open = if let Some(pane) = &self.state.editor
            && pane.editor.id() == Some(target.id)
        {
            if let Ok(request) = pane.editor.to_request() {
                Some(request)
            } else {
                self.state.load.error =
                    Some("Fix the request's settings on its Settings tab first".to_owned());
                return;
            }
        } else {
            None
        };
        let screen = &mut self.state.load;
        screen.generation += 1;
        screen.error = None;
        let generation = screen.generation;
        let progress = Arc::new(imposter_app::load::Progress::default());
        let shared = progress.clone();
        let db = self.db.clone();
        let id = target.id;
        let task = self.bridge.spawn(move |cancel| async move {
            let request = match open {
                Some(request) => request,
                None => match db.call(move |db| collections::request(db, id)).await {
                    Ok(Some(request)) => request,
                    Ok(None) => {
                        return Message::LoadFinished {
                            generation,
                            result: Err(NotSent {
                                message: "The request no longer exists".to_owned(),
                                unresolved: Vec::new(),
                            }),
                        };
                    }
                    Err(error) => {
                        return Message::LoadFinished {
                            generation,
                            result: Err(ExecuteError::Store(error).into()),
                        };
                    }
                },
            };
            let result = imposter_app::load::run(&db, request, plan, cancel, shared).await;
            Message::LoadFinished {
                generation,
                result: result.map_err(NotSent::from),
            }
        });
        screen.run = Some(crate::load_screen::Running {
            task,
            progress,
            started: now,
            plan,
            target,
            stopping: false,
        });
        let form = serde_json::to_value(&self.state.load.form).unwrap_or(Value::Null);
        self.write_ui_state(LOAD_KEY, form);
    }

    fn load_finished(
        &mut self,
        generation: u64,
        result: Result<imposter_app::load::Report, NotSent>,
    ) {
        let screen = &mut self.state.load;
        if generation != screen.generation {
            return;
        }
        let Some(run) = screen.run.take() else {
            return;
        };
        match result {
            Ok(report) => {
                let text = format!(
                    "Load test finished · {}",
                    crate::load_screen::rate_text(report.summary.per_second)
                );
                screen.finished(crate::load_screen::Finished {
                    target: run.target,
                    plan: run.plan,
                    report,
                    when: crate::viewer::clock_time(executor::now()),
                });
                self.status(&text);
            }
            Err(not_sent) => screen.error = Some(not_sent.message),
        }
    }

    fn run_action(&mut self, action: crate::run_screen::RunAction) {
        use crate::run_screen::RunAction;
        let screen = &mut self.state.run;
        match action {
            RunAction::Choose(target) => {
                if screen.run.is_none() {
                    screen.target = Some(target);
                }
            }
            RunAction::Show(index) => {
                if index < screen.runs.len() && screen.run.is_none() {
                    screen.shown = index;
                    screen.selected = None;
                    screen.pane = ResponsePane::default();
                }
            }
            RunAction::Select(index) => screen.select(index),
            RunAction::Stop => {
                if let Some(run) = &mut screen.run {
                    run.stopping = true;
                    self.bridge.cancel(run.task);
                }
            }
            RunAction::Start { confirmed } => self.run_start(confirmed),
        }
    }

    fn run_start(&mut self, confirmed: bool) {
        use crate::run_screen::EnvironmentChoice;
        if self.state.close != CloseState::Open || self.state.run.run.is_some() {
            return;
        }
        let Some(target) = self.state.run.target.clone() else {
            return;
        };
        let chosen = match self.state.run.environment {
            EnvironmentChoice::Active => self.state.active_environment,
            EnvironmentChoice::Chosen(id) => id,
        };
        let environment = chosen.and_then(|id| self.state.environments.iter().find(|e| e.id == id));
        // Red marks production: a whole run against it is worth a second look.
        if !confirmed
            && let Some(environment) = environment.filter(|e| e.color == Some(EnvColor::Red))
        {
            self.state.dialog = Some(Dialog::start_run(&environment.name, &target.label));
            return;
        }
        let environment_name =
            environment.map_or_else(|| "No environment".to_owned(), |e| e.name.clone());
        let screen = &mut self.state.run;
        screen.generation += 1;
        screen.selected = None;
        screen.pane = ResponsePane::default();
        let generation = screen.generation;
        let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
        let pushed = shared.clone();
        let db = self.db.clone();
        let keep_going = screen.keep_going;
        let (collection, folder) = (target.collection, target.folder);
        let task = self.bridge.spawn(move |cancel| async move {
            let requests =
                match imposter_app::profile::select_under(&db, collection, folder, chosen).await {
                    Ok(requests) => requests,
                    Err(error) => {
                        return Message::RunFinished {
                            generation,
                            result: Err(error.to_string()),
                        };
                    }
                };
            let options = imposter_app::runner::Options {
                keep_going,
                record: Some((db, imposter_app::core::execution::Via::App)),
            };
            imposter_app::runner::run(requests, options, cancel, |step| {
                if let Ok(mut steps) = pushed.lock() {
                    steps.push(step.clone());
                }
            })
            .await;
            Message::RunFinished {
                generation,
                result: Ok(()),
            }
        });
        screen.run = Some(crate::run_screen::Running {
            task,
            shared,
            steps: Vec::new(),
            target,
            environment: environment_name,
            stopping: false,
        });
    }

    fn run_finished(&mut self, generation: u64, result: Result<(), String>) {
        let screen = &mut self.state.run;
        if generation != screen.generation {
            return;
        }
        let Some(mut run) = screen.run.take() else {
            return;
        };
        run.drain();
        let ran_any = !run.steps.is_empty();
        let finished = crate::run_screen::FinishedRun {
            target: run.target,
            environment: run.environment,
            steps: run.steps,
            when: crate::viewer::clock_time(executor::now()),
            error: result.err(),
        };
        let text = match &finished.error {
            Some(error) => format!("Run not started · {error}"),
            None => format!(
                "Run finished · {}",
                crate::run_screen::summary(&finished.steps).1
            ),
        };
        screen.finished(finished);
        if ran_any {
            // Each request sent is in History now.
            if self.state.screen == Screen::History {
                self.load_history();
            }
            if let Some(id) = self.state.editor.as_ref().and_then(|pane| pane.editor.id()) {
                self.load_runs(id);
            }
        }
        self.status(&text);
    }

    fn mocks_transition(&mut self, next: crate::mock_screen::Transition) {
        if self.state.mocks.dirty() {
            self.state.mocks.pending = Some(next);
            self.state.dialog = Some(Dialog::MocksGuard);
        } else {
            match next {
                crate::mock_screen::Transition::Reload => self.load_mocks(),
                crate::mock_screen::Transition::Close => self.finish_close(),
            }
        }
    }

    fn mocks_cancel_transition(&mut self) {
        if self.state.mocks.pending.take() == Some(crate::mock_screen::Transition::Close) {
            self.state.close = CloseState::Open;
        }
    }

    fn mocks_choice(&mut self, choice: GuardChoice) {
        match choice {
            GuardChoice::Save => self.save_mocks(),
            GuardChoice::Cancel => self.mocks_cancel_transition(),
            GuardChoice::Discard => {
                let screen = &mut self.state.mocks;
                if let Some(base) = &screen.baseline {
                    screen.form = base.clone();
                }
                screen.error = None;
                if let Some(next) = screen.pending.take() {
                    self.mocks_transition(next);
                }
            }
        }
    }

    fn finish_close(&mut self) {
        self.state.close = CloseState::Flushing;
        self.flush_ui_state();
    }

    // ---- history --------------------------------------------------------------------------------

    /// Read how much history holds, after anything that may change it.
    fn load_usage(&mut self) {
        self.task(
            |db| executions::usage(db),
            |result| Message::HistoryUsage(result.map_err(failure)),
        );
    }

    fn load_history(&mut self) {
        self.load_usage();
        let history = &mut self.state.history;
        history.generation += 1;
        let generation = history.generation;
        let offset = history.offset;
        let filter = if history.only_current {
            // No open saved request: an id that matches nothing, so the list is empty.
            Some(
                self.state
                    .editor
                    .as_ref()
                    .and_then(|p| p.editor.id())
                    .unwrap_or(RequestId(-1)),
            )
        } else {
            None
        };
        history.status = Some(("Loading history…".into(), Tone::Muted));
        let query = imposter_app::filter::Query::parse_with(
            &history.filter,
            imposter_app::filter::HISTORY_FIELDS,
        );
        self.task(
            move |db| {
                if query.is_all() {
                    executions::page(db, filter, offset, HISTORY_PAGE)
                } else {
                    executions::search(db, filter, |row| query.matches(row), offset, HISTORY_PAGE)
                }
            },
            move |result| Message::HistoryPage {
                generation,
                result: result.map_err(failure),
            },
        );
    }

    fn history_page(&mut self, generation: u64, result: Result<Vec<ExecutionSummary>, Failure>) {
        if generation == 0 {
            self.load_usage();
        }
        let history = &mut self.state.history;
        // Generation 0 answers "clear all history", which always applies.
        if generation != 0 && generation != history.generation {
            return;
        }
        if generation == 0 {
            history.offset = 0;
        }
        match result {
            Ok(rows) => {
                let filtered = !history.filter.trim().is_empty();
                history.status = Some(if rows.is_empty() && filtered {
                    ("No matching executions".to_owned(), Tone::Muted)
                } else if rows.is_empty() {
                    ("No executions yet".to_owned(), Tone::Muted)
                } else {
                    (
                        format!(
                            "{} {}-{}",
                            if filtered { "Matches" } else { "Executions" },
                            history.offset + 1,
                            history.offset + rows.len() as u64
                        ),
                        Tone::Muted,
                    )
                });
                history.rows = rows;
                history.loaded = true;
                if history
                    .selected
                    .is_some_and(|id| !history.rows.iter().any(|r| r.id == id))
                {
                    history.selected = None;
                    history.record = None;
                    history.request = None;
                    history.response = ResponsePane::default();
                }
            }
            Err(error) => {
                history.status = Some((format!("History unavailable: {error}"), Tone::Error));
            }
        }
    }

    fn history_action(&mut self, action: HistoryAction) {
        match action {
            HistoryAction::Refresh | HistoryAction::Filtered => {
                self.state.history.offset = 0;
                self.load_history();
            }
            HistoryAction::Newer => {
                self.state.history.offset = self.state.history.offset.saturating_sub(HISTORY_PAGE);
                self.load_history();
            }
            HistoryAction::Older => {
                if self.state.history.rows.len() as u64 == HISTORY_PAGE {
                    self.state.history.offset += HISTORY_PAGE;
                    self.load_history();
                }
            }
            HistoryAction::OnlyCurrent(only) => {
                self.state.history.only_current = only;
                self.state.history.offset = 0;
                self.load_history();
            }
            HistoryAction::Select(id) => {
                self.state.history.selected = Some(id);
                self.state.history.record = None;
                self.state.history.opened = None;
                self.task(
                    move |db| executions::execution(db, id),
                    move |result| Message::HistoryRecord {
                        id,
                        result: result.map(|r| r.map(Box::new)).map_err(failure),
                    },
                );
            }
            HistoryAction::Restore => {
                if let Some(record) = &self.state.history.record {
                    let draft = record.restore();
                    self.navigate(Navigation::Restore {
                        request: Box::new(draft),
                        status: "Restored a draft from History",
                        response: Some(Box::new(record.response.clone())),
                    });
                }
            }
            HistoryAction::CopyCurl => {
                if let Some(record) = &self.state.history.record {
                    match executor::recorded_curl(record) {
                        Ok(command) => {
                            self.state.clipboard = Some(command);
                            self.state.history.status = Some((
                                if record.settings.host_identities.is_empty() {
                                    "Copied as curl"
                                } else {
                                    executor::CURL_IDENTITY_NOTICE
                                }
                                .into(),
                                Tone::Ok,
                            ));
                        }
                        Err(error) => self.error("Copy as curl failed", error.to_string()),
                    }
                }
            }
            HistoryAction::Clear => {
                let executions = self.state.history.usage.map(|u| u.entries);
                self.state.dialog = Some(Dialog::clear_history(executions));
            }
            HistoryAction::OpenResult => {
                // From the palette: show where the result will appear.
                self.state.screen = Screen::History;
                if !self.state.history.loaded {
                    self.load_history();
                }
                self.pick(
                    FileDialog::Open {
                        title: "Open result file",
                        filter: Some(("Imposter results", &["json"])),
                    },
                    |paths| Message::ResultPicked(paths.into_iter().next()),
                );
            }
            HistoryAction::CloseResult => {
                let history = &mut self.state.history;
                history.opened = None;
                history.request = None;
                history.response = ResponsePane::default();
            }
        }
    }

    // ---- closing --------------------------------------------------------------------------

    /// Begin the close sequence: stop the running request, let background work finish,
    /// resolve unsaved changes, write UI state, then close.
    pub fn request_close(&mut self) {
        if self.state.close != CloseState::Open {
            return;
        }
        self.mock_action(crate::mock_screen::MockAction::Stop);
        self.load_action(crate::load_screen::LoadAction::Stop, 0.0);
        self.run_action(crate::run_screen::RunAction::Stop);
        self.state.close = CloseState::Draining;
        self.state.autosave_due = None;
        if let Some(execution) = &self.state.execution {
            self.bridge.cancel(execution.task);
        }
    }

    fn close_tick(&mut self, ctx: &egui::Context) {
        match self.state.close {
            CloseState::Draining if self.bridge.running() == 0 => {
                // Unsaved or failed edits are resolved by the same guard as any navigation.
                self.state.close = CloseState::Guarding;
                self.navigate(Navigation::Close);
            }
            CloseState::Draining => ctx.request_repaint_after(Duration::from_millis(30)),
            CloseState::Ready => ctx.send_viewport_cmd(ViewportCommand::Close),
            CloseState::Open | CloseState::Guarding | CloseState::Flushing => {}
        }
    }

    fn remember_window(&mut self, ctx: &egui::Context) {
        let (outer, inner, maximized) = ctx.input(|i| {
            (
                i.viewport().outer_rect,
                i.viewport().inner_rect,
                i.viewport().maximized,
            )
        });
        if let (Some(outer), Some(inner)) = (outer, inner) {
            let maximized = maximized.unwrap_or(false);
            // Keep the normal size while maximized, so restoring un-maximizes to it.
            if !maximized || self.window.is_none() {
                self.window = Some(WindowState {
                    x: outer.min.x,
                    y: outer.min.y,
                    width: inner.width(),
                    height: inner.height(),
                    maximized,
                });
            } else if let Some(window) = &mut self.window {
                window.maximized = true;
            }
        }
    }

    fn flush_ui_state(&mut self) {
        let layout = self.state.layout.to_stored();
        let last = serde_json::to_value(Last {
            collection_id: self.state.collection,
            request_id: self.state.editor.as_ref().and_then(|pane| pane.editor.id()),
        })
        .unwrap_or(Value::Null);
        let draft = self
            .state
            .editor
            .as_ref()
            .filter(|pane| pane.editor.is_draft())
            .and_then(|pane| pane.editor.to_request().ok())
            .map_or(Value::Null, |request| {
                serde_json::to_value(request).unwrap_or(Value::Null)
            });
        let window = self
            .window
            .and_then(|w| serde_json::to_value(w).ok())
            .unwrap_or(Value::Null);
        let db = self.db.clone();
        self.bridge.spawn(move |_| async move {
            let result = db
                .call(move |tx| {
                    kv::set(tx, Table::UiState, LAYOUT_KEY, &layout)?;
                    kv::set(tx, Table::UiState, LAST_KEY, &last)?;
                    kv::set(tx, Table::UiState, DRAFT_KEY, &draft)?;
                    if !window.is_null() {
                        kv::set(tx, Table::UiState, WINDOW_KEY, &window)?;
                    }
                    Ok(())
                })
                .await;
            Message::Flushed(result.map_err(failure))
        });
    }
}

impl ImposterApp {
    fn pane(&mut self, kind: PaneKind) -> &mut ResponsePane {
        match kind {
            PaneKind::Request => &mut self.state.response,
            PaneKind::History => &mut self.state.history.response,
            PaneKind::Run => &mut self.state.run.pane,
        }
    }

    /// Start the filter runs the response viewers need; parsing large bodies stays off the UI
    /// thread.
    fn start_filters(&mut self) {
        for kind in [PaneKind::Request, PaneKind::History, PaneKind::Run] {
            let Some(job) = self.pane(kind).filter_job() else {
                continue;
            };
            self.bridge.spawn(move |_| async move {
                let FilterJob {
                    shown,
                    query,
                    source,
                } = job;
                let asked = query.clone();
                let done = match imposter_app::runtime::compute(move || source.run(&query)).await {
                    Ok((document, result)) => Filtered {
                        shown,
                        query: asked,
                        document: Some(document),
                        result,
                    },
                    Err(error) => Filtered {
                        shown,
                        query: asked,
                        document: None,
                        result: Err(format!("Filtering failed: {error}")),
                    },
                };
                Message::Filtered(kind, Box::new(done))
            });
        }
    }
}

impl eframe::App for ImposterApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for message in self.bridge.drain() {
            self.apply(message);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.state.close != CloseState::Ready {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.request_close();
        }
        self.remember_window(ctx);
        self.autosave_tick(ctx);
        self.close_tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.applied != Some(self.state.appearance) {
            theme::apply(
                ui.ctx(),
                self.state.appearance.theme,
                self.state.appearance.density,
            );
            self.applied = Some(self.state.appearance);
        }
        theme::begin_frame(ui.ctx());
        crate::workspace::draw(self, ui);
        let now = ui.input(|i| i.time);
        for action in std::mem::take(&mut self.state.actions) {
            self.handle(action, now);
        }
        self.start_filters();
        if let Some(dialog) = &mut self.state.dialog {
            let outcome = dialog.show(ui.ctx());
            self.dialog_outcome(outcome);
        }
        if let Some(text) = self.state.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        if let Some(started) = self.first_frame.take() {
            tracing::info!(elapsed_ms = started.elapsed().as_millis(), "first frame");
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.bridge.shutdown(SHUTDOWN_GRACE) {
            tracing::warn!("background work did not finish before exit");
        }
        if let Some(store) = self.store.take()
            && let Err(error) = store.close()
        {
            tracing::error!(%error, "closing the database failed");
        }
    }
}

fn native_export_dialog() -> FileDialog {
    FileDialog::Save {
        title: "Export collection",
        name: "collection.imposter.json",
        filter: Some(("Native", "json")),
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn line(lasting: Lasting) -> StatusLine {
        StatusLine {
            text: "x".into(),
            tone: Tone::Muted,
            lasting,
        }
    }

    #[test]
    fn passing_messages_fade_from_when_they_are_first_shown() {
        let mut passing = line(Lasting::For {
            seconds: STATUS_SECONDS,
            shown_at: None,
        });
        assert!(passing.applies(1, Screen::Requests, 100.0));
        assert!(
            passing.applies(9, Screen::History, 105.9),
            "screens do not matter"
        );
        assert!(!passing.applies(9, Screen::History, 106.0));
    }

    #[test]
    fn request_messages_last_while_the_request_and_screen_do() {
        let mut about = line(Lasting::While {
            generation: 3,
            screen: Screen::Requests,
        });
        assert!(about.applies(3, Screen::Requests, 1e9), "no time limit");
        assert!(!about.applies(4, Screen::Requests, 0.0), "another request");
        assert!(!about.applies(3, Screen::Mocks, 0.0), "another screen");
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn json(body: &str) -> Response {
        Response {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
            ..Response::default()
        }
    }

    fn finish(job: FilterJob) -> Filtered {
        let (document, result) = job.source.run(&job.query);
        Filtered {
            shown: job.shown,
            query: job.query,
            document: Some(document),
            result,
        }
    }

    #[test]
    fn one_run_at_a_time_and_the_newest_query_follows() {
        let mut pane = ResponsePane::default();
        pane.show(json(r#"{"a":1,"b":2}"#));
        assert!(pane.filter_job().is_none(), "closed");
        pane.filter.open();
        assert!(pane.filter_job().is_none(), "no query yet");
        pane.filter.query = "$.a".into();
        let first = pane.filter_job().unwrap();
        assert!(matches!(
            first.source,
            crate::viewer::FilterSource::Body { .. }
        ));
        pane.filter.query = "$.b".into();
        assert!(pane.filter_job().is_none(), "one run in flight");
        assert!(pane.filter.stale());

        pane.filtered(finish(first));
        assert_eq!(pane.filter.result.as_ref().unwrap().0, "$.a");
        let second = pane.filter_job().unwrap();
        assert_eq!(second.query, "$.b");
        assert!(
            matches!(second.source, crate::viewer::FilterSource::Parsed(_)),
            "the body is parsed once"
        );
        pane.filtered(finish(second));
        assert!(!pane.filter.stale());
        assert!(pane.filter_job().is_none(), "nothing left to do");
        assert_eq!(
            pane.filter.shown().unwrap().as_ref().unwrap().text,
            "[\n  2\n]"
        );
    }

    #[test]
    fn the_results_tab_gives_way_when_a_response_has_no_checks() {
        use imposter_app::core::check::{Check, CheckResult, Op, Verdict};
        let mut pane = ResponsePane::default();
        let mut checked = json("{}");
        checked.checks = vec![CheckResult {
            check: Check::new(
                imposter_app::core::select::Selector::Status,
                Op::Equals,
                "200",
            ),
            verdict: Verdict::Passed,
            actual: Some("200".into()),
        }];
        pane.show(checked.clone());
        pane.tab = ResponseTab::Tests;
        pane.show(checked);
        assert_eq!(pane.tab, ResponseTab::Tests);
        pane.show(json("{}"));
        assert_eq!(pane.tab, ResponseTab::Body);
    }

    #[test]
    fn a_result_for_an_earlier_response_is_dropped() {
        let mut pane = ResponsePane::default();
        pane.show(json(r#"{"a":1}"#));
        pane.filter.open();
        pane.filter.query = "$.a".into();
        let old = pane.filter_job().unwrap();
        pane.show(json(r#"{"a":2}"#));
        let current = pane.filter_job().unwrap();
        pane.filtered(finish(old));
        assert!(
            pane.filter.result.is_none(),
            "the old body's result is ignored"
        );
        pane.filtered(finish(current));
        assert_eq!(
            pane.filter.shown().unwrap().as_ref().unwrap().text,
            "[\n  2\n]"
        );

        pane.filter.close();
        assert!(pane.filter.shown().is_none());
        assert!(pane.filter_job().is_none());
    }

    #[test]
    fn a_failed_run_keeps_its_query_and_parses_again_next_time() {
        let mut pane = ResponsePane::default();
        pane.show(json(r#"{"a":1}"#));
        pane.filter.open();
        pane.filter.query = "$.a".into();
        let job = pane.filter_job().unwrap();
        pane.filtered(Filtered {
            shown: job.shown,
            query: job.query,
            document: None,
            result: Err("Filtering failed: boom".into()),
        });
        assert!(
            pane.filter_job().is_none(),
            "the same query is not retried in a loop"
        );
        pane.filter.query = "$".into();
        assert!(matches!(
            pane.filter_job().unwrap().source,
            crate::viewer::FilterSource::Body { .. }
        ));
    }
}

#[cfg(test)]
mod branding_tests {
    use super::*;

    #[test]
    fn native_exports_suggest_the_new_extension() {
        let FileDialog::Save { name, filter, .. } = native_export_dialog() else {
            panic!("native export needs a save dialog");
        };
        assert_eq!(name, "collection.imposter.json");
        assert_eq!(filter, Some(("Native", "json")));
    }
}
