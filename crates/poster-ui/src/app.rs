//! The eframe application: owns all UI state and turns user actions into background work.
//!
//! Widgets never touch the database or network. They emit [`Action`]s while drawing; after the
//! frame, [`PosterApp::handle`] applies them. Background results come back as [`Message`]s,
//! drained at the start of each frame. Results carry the selection generation they were
//! started under, so a late result never replaces another request's state.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use egui::ViewportCommand;
use poster_app::core::execution::{ExecutionRecord, Outcome as ExchangeOutcome, Response};
use poster_app::core::ids::{CollectionId, EnvironmentId, ExecutionId, FolderId, RequestId};
use poster_app::core::model::{FormItem, Request, Variables};
use poster_app::core::settings::AppSettings;
use poster_app::core::variables::Snapshot;
use poster_app::executor::{self, ExecuteError, Executed};
use poster_app::formats::native::NativeDocument;
use poster_app::runtime::{Bridge, TaskId};
use poster_app::saver::{Saver, State as SaveState};
use poster_app::store::collections::{self, Collection, Tree};
use poster_app::store::environments::{self, Environment};
use poster_app::store::executions::{self, ExecutionSummary};
use poster_app::store::kv::{self, Table};
use poster_app::store::{Store, StoreError, StoreHandle};
use poster_app::transfer::{self, ExportScope, ImportPreview};
use poster_app::{ops, store};
use serde_json::Value;

use crate::dialogs::{ConfirmPurpose, Dialog, GuardChoice, NamePurpose, Outcome};
use crate::editor::{Editor, InvalidOptions, OptionField};
use crate::layout::WorkspaceLayout;
use crate::settings_screen::SettingsForm;
use crate::theme::{self, Tone};
use crate::tree::{NodeId, TreeView};
use crate::vars::{self, Scope};
use crate::widgets::{FileRequest, Inspection, SaveStatus, ViewerState};

/// How long shutdown waits for background work before aborting it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// Autosave waits this long after the last edit.
pub const AUTOSAVE_DELAY: f64 = 0.5;

const LAST_KEY: &str = "last";
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
    layout: Value,
    draft: Option<Request>,
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
        result: Result<Box<Executed>, Failure>,
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
        result: Result<Variables, Failure>,
    },
    VariablesSaved {
        scope: Scope,
        result: Result<(), Failure>,
    },
    HistoryPage {
        generation: u64,
        result: Result<Vec<ExecutionSummary>, Failure>,
    },
    HistoryRecord {
        id: ExecutionId,
        result: Result<Option<Box<ExecutionRecord>>, Failure>,
    },
    Flushed(Result<(), Failure>),
    /// Background work with nothing to apply (it logs its own failures).
    Done,
    FilePicked {
        target: FileRequest,
        path: Option<PathBuf>,
    },
    ImportPicked(Option<PathBuf>),
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
    PostmanReady(Result<Box<PostmanExport>, Failure>),
    PostmanExportPicked(Option<PathBuf>),
    Exported(Result<PathBuf, Failure>),
    Curl(Result<String, Failure>),
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
/// [`PosterApp::script_file_dialogs`]).
#[derive(Debug, Clone)]
enum FileDialog {
    Open {
        title: &'static str,
        filter: Option<(&'static str, &'static str)>,
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
    Move {
        node: NodeId,
        target: Option<FolderId>,
    },
    Save,
    Send,
    PickFile(FileRequest),
    ShowRun(ExecutionId),
    Screen(Screen),
    /// Switch the active execution environment (`None`: globals only).
    SetEnvironment(Option<EnvironmentId>),
    /// The inspector's "Open definition": edit `name` in `scope`.
    OpenDefinition(Scope, String),
    Variables(VariablesAction),
    History(HistoryAction),
    /// Pick a collection file and review its import.
    Import,
    /// Choose what to export from the selected collection.
    Export,
    /// A Settings screen field changed.
    SettingsEdited,
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
}

/// The Settings screen: the form, the saved baseline it is compared with, and save state.
#[derive(Debug)]
pub struct SettingsScreen {
    pub form: SettingsForm,
    /// The stored settings, including those the screen does not show.
    base: AppSettings,
    baseline: Option<SettingsForm>,
    pub status: Option<(String, Tone)>,
    saving: bool,
    loading: bool,
    /// Saving or discarding resolves an app close.
    closing: bool,
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
        self.status = Some(if self.dirty() {
            ("Unsaved".into(), Tone::Warning)
        } else {
            ("Saved".into(), Tone::Muted)
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    Refresh,
    Newer,
    Older,
    OnlyCurrent(bool),
    Select(ExecutionId),
    Restore,
    /// Copy the selected execution's request as curl.
    CopyCurl,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryTab {
    #[default]
    Request,
    Response,
}

pub const HISTORY_PAGE: u64 = 50;

/// The History screen: pages of executions and the details of the selected one.
#[derive(Default)]
pub struct HistoryScreen {
    pub only_current: bool,
    pub offset: u64,
    pub rows: Vec<ExecutionSummary>,
    pub selected: Option<ExecutionId>,
    pub record: Option<ExecutionRecord>,
    /// The selected execution's request as sent.
    pub request: Option<crate::viewer::SentRequest>,
    pub request_tab: ResponseTab,
    pub request_body_view: ViewerState,
    pub request_headers_view: ViewerState,
    pub response: ResponsePane,
    pub tab: HistoryTab,
    pub status: Option<(String, Tone)>,
    pub loaded: bool,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariablesAction {
    SelectScope(Scope),
    Save,
    /// Back to the stored rows, without asking.
    Revert,
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
    Close,
}

/// The Variables screen: one scope's rows, edited independently of requests.
#[derive(Debug, Default)]
pub struct VariablesScreen {
    pub scope: Scope,
    pub rows: Vec<vars::Row>,
    baseline: Vec<(String, String)>,
    pub status: Option<(String, Tone)>,
    pub loaded: bool,
    pub confirm_clear: bool,
    /// A name to highlight after "Open definition".
    pub highlight: Option<String>,
    pending: Option<VarTransition>,
    saving: bool,
}

impl VariablesScreen {
    pub fn dirty(&self) -> bool {
        self.loaded && vars::entries(&self.rows) != self.baseline
    }
}

/// Navigations that replace the editor go through the unsaved-changes guard.
#[derive(Debug, Clone, PartialEq)]
enum Navigation {
    Open(RequestId),
    Duplicate {
        id: RequestId,
        edited: Option<Box<poster_app::core::model::RequestTemplate>>,
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
}

impl EditorTab {
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::Body, "Body"),
        (Self::Auth, "Auth"),
        (Self::Params, "Query"),
        (Self::Headers, "Headers"),
        (Self::Cookies, "Cookies"),
        (Self::Settings, "Settings"),
        (Self::Docs, "Docs"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseTab {
    #[default]
    Body,
    Headers,
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
    /// The formatted preview of `response`, computed once per response.
    pub preview: Option<(crate::viewer::Preview, String)>,
}

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
            preview: None,
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
        self.response = Some(response);
    }

    fn clear(&mut self, notice: &str) {
        *self = Self {
            notice: Some((notice.to_owned(), Tone::Muted)),
            ..Self::default()
        };
    }
}

struct Execution {
    task: TaskId,
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
    pub status: String,
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
    /// The header logo, uploaded on first use.
    pub logo: Option<egui::TextureHandle>,
    pub actions: Vec<Action>,
}

pub struct PosterApp {
    bridge: Bridge<Message>,
    store: Option<Store>,
    db: StoreHandle,
    pub state: State,
    window: Option<WindowState>,
    first_frame: Option<std::time::Instant>,
    /// Scripted file dialog answers (tests); `None` opens native dialogs.
    file_answers: Option<VecDeque<Option<PathBuf>>>,
    smoke: Option<SmokeTest>,
}

impl PosterApp {
    pub fn new(ctx: &egui::Context, store: Store) -> std::io::Result<Self> {
        theme::apply(ctx);
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
                status: "Starting…".into(),
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
                logo: None,
                actions: Vec::new(),
            },
            window: None,
            first_frame: Some(std::time::Instant::now()),
            file_answers: None,
            smoke: None,
        };
        app.state.response.clear("No request selected");
        app.task(
            |db| {
                Ok(Startup {
                    settings: kv::app_settings(db)?,
                    collections: collections::collections(db)?,
                    environments: environments::environments(db)?,
                    active_environment: environments::active_environment(db)?,
                    last: kv::get(db, Table::UiState, LAST_KEY)?.unwrap_or_default(),
                    layout: kv::get(db, Table::UiState, LAYOUT_KEY)?.unwrap_or(Value::Null),
                    draft: kv::get(db, Table::UiState, DRAFT_KEY)?.flatten(),
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
                    if let Some((label, extension)) = filter {
                        builder = builder.add_filter(label, &[extension]);
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
                        Ok(Some(record)) => self.state.response.show(record.response),
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
            Message::HistoryPage { generation, result } => self.history_page(generation, result),
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
            Message::FilePicked { target, path } => {
                if let Some(path) = path {
                    self.file_picked(target, &path);
                }
            }
            Message::ImportPicked(path) => {
                if let Some(path) = path {
                    if path.to_string_lossy().ends_with(".poster.json") {
                        self.read_import(path, Vec::new());
                    } else {
                        self.state.dialog = Some(Dialog::Companions { path });
                    }
                }
            }
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
            Message::Curl(Ok(command)) => {
                self.state.clipboard = Some(command);
                self.status("Copied as curl");
            }
            Message::Curl(Err(error)) => self.error("Copy as curl failed", error),
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
        state.status = "Ready".into();
        state.environments = startup.environments;
        state.active_environment = startup.active_environment;
        if let Some(draft) = startup.draft {
            self.load_editor(&draft);
            self.status(
                "Restored unsaved draft — Send to execute, Save to add to the selected collection",
            );
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

    fn status(&mut self, text: &str) {
        text.clone_into(&mut self.state.status);
    }

    fn error(&mut self, title: &str, message: impl Into<String>) {
        self.state.dialog = Some(Dialog::error(title, message));
    }

    // ---- loading --------------------------------------------------------------------------

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
        self.state.response.clear(if id.is_some() {
            "No response yet"
        } else {
            "Unsaved draft"
        });
        self.state.inspect = None;
        self.load_snapshot();
        if let Some(id) = id {
            self.load_runs(id);
            self.remember_selection();
        }
    }

    fn clear_editor(&mut self) {
        if let Some(id) = self.state.editor.as_ref().and_then(|pane| pane.editor.id()) {
            self.state.saver.forget(id);
        }
        self.state.editor = None;
        self.state.selected = None;
        self.state.generation += 1;
        self.state.response.clear("No request selected");
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
                self.state.dialog = Some(Dialog::Guard {
                    request,
                    failed: None,
                });
            }
        }
    }

    fn perform(&mut self, next: Navigation) {
        match next {
            Navigation::Open(id) => self.open(id),
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
            Navigation::Restore { request, status } => {
                self.state.generation += 1;
                self.load_editor(&request);
                self.state.screen = Screen::Requests;
                self.status(status);
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
            self.state.response.clear("No request selected");
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
            self.state.dialog = Some(Dialog::Guard {
                request,
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
        let (text, detail, enabled, tone) = if !pane.invalid.is_empty() {
            (
                "Invalid fields",
                "Fix the highlighted fields to save",
                false,
                Tone::Error,
            )
        } else if let Some(id) = pane.editor.id() {
            match self.state.saver.state(id) {
                SaveState::Saving => ("Saving…", "Saving the request", false, Tone::Muted),
                SaveState::Failed => (
                    "Save failed",
                    "The request changes could not be saved; retry or discard them",
                    true,
                    Tone::Error,
                ),
                SaveState::Dirty => ("Unsaved", "Unsaved request changes", true, Tone::Warning),
                SaveState::Saved if self.state.settings.autosave => (
                    "Saved",
                    "Changes are saved automatically",
                    false,
                    Tone::Muted,
                ),
                SaveState::Saved => ("Saved", "The request is saved", false, Tone::Muted),
            }
        } else {
            (
                "Draft",
                "Not saved to a collection yet",
                true,
                Tone::Warning,
            )
        };
        Some(SaveStatus {
            text,
            detail,
            tone,
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
        let Some(pane) = &mut self.state.editor else {
            return;
        };
        let path = path.display().to_string();
        match target {
            FileRequest::Binary => pane.editor.binary_path = path,
            FileRequest::MultipartNew => pane.editor.multipart.push(FormItem::file("file", path)),
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
                result: result.map(Box::new).map_err(|error| match error {
                    ExecuteError::Invalid(message) => format!("Request failed: {message}"),
                    ExecuteError::Store(error) => format!("Request failed: {error}"),
                }),
            }
        });
        self.state.execution = Some(Execution { task });
        let previous = self.state.response.response.is_some();
        self.state.response.pending = true;
        self.state.response.notice = Some((
            if previous {
                "Request in progress · showing previous run"
            } else {
                "Request in progress"
            }
            .into(),
            Tone::Muted,
        ));
    }

    fn executed(&mut self, generation: u64, result: Result<Box<Executed>, Failure>) {
        self.state.execution = None;
        self.state.response.pending = false;
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
                Err(error) => Err(error.clone()),
            };
            return self.finish_smoke(outcome);
        }
        if self.state.history.loaded {
            self.load_history();
        }
        if generation != self.state.generation {
            return;
        }
        match result {
            Ok(executed) => {
                let response = executed.response;
                let summary = if response.history_error.is_empty() {
                    format!(
                        "{:?} in {}",
                        response.outcome,
                        crate::viewer::format_duration(response.elapsed)
                            .trim_start_matches("Time: ")
                    )
                } else {
                    response.history_error.clone()
                };
                self.state.status = summary;
                self.state.response.shown_run = executed.record;
                self.state.response.show(response);
                if let Some(id) = self.state.editor.as_ref().and_then(|p| p.editor.id()) {
                    self.load_runs(id);
                }
            }
            Err(message) => {
                self.state.response.notice = Some((message, Tone::Error));
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
            NamePurpose::NewEnvironment => {
                let db = self.db.clone();
                self.bridge.spawn(move |_| async move {
                    let result = db
                        .call(move |tx| {
                            let created =
                                environments::create_environment(tx, &name, &Variables::new())?;
                            Ok((
                                created.id,
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
            NamePurpose::RenameEnvironment(id) => self.task(
                move |db| {
                    environments::rename_environment(db, id, &name)?;
                    Ok((
                        environments::environments(db)?,
                        environments::active_environment(db)?,
                    ))
                },
                |result| Message::Environments {
                    select: None,
                    result: result.map_err(failure),
                },
            ),
        }
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
            ConfirmPurpose::DiscardOnClose => {
                self.state.saver.discard(None);
                self.state.close = CloseState::Ready;
            }
            ConfirmPurpose::PostmanExport => self.pick_postman_destination(),
        }
    }

    fn move_node(&mut self, node: NodeId, target: Option<FolderId>) {
        if let (NodeId::Folder(id), Some(target)) = (node, target)
            && id == target
        {
            return;
        }
        let reload = |result: Result<(), StoreError>| Message::Changed {
            reload_collections: false,
            result: result.map_err(failure),
        };
        match node {
            NodeId::Folder(id) => {
                self.task(move |db| collections::move_folder(db, id, target), reload);
            }
            NodeId::Request(id) => {
                if let Some(pane) = &mut self.state.editor
                    && pane.editor.id() == Some(id)
                {
                    let name = pane.editor.name().to_owned();
                    pane.editor.adopt_metadata(&name, target);
                }
                self.state
                    .saver
                    .adopt_metadata(id, |request| request.folder_id = target);
                self.task(move |db| collections::move_request(db, id, target), reload);
            }
        }
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
            Action::Prompt(dialog) => self.state.dialog = Some(dialog),
            Action::Move { node, target } => self.move_node(node, target),
            Action::Save => match self.state.screen {
                Screen::Variables => self.save_variables(),
                Screen::Settings => self.save_settings(),
                Screen::Requests if self.state.guard.is_none() => self.save_current(),
                Screen::Requests | Screen::History => {}
            },
            Action::SettingsEdited => self.state.settings_screen.refresh_status(),
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
                    filter: Some(("Collections", "json")),
                },
                |paths| Message::ImportPicked(paths.into_iter().next()),
            ),
            Action::Export => self.prompt_export(),
            Action::ShowRun(id) => self.show_run(id),
            Action::Edited { url, params } => self.edited(url, params, now),
            Action::Screen(screen) => {
                self.state.screen = screen;
                if screen == Screen::History && !self.state.history.loaded {
                    self.load_history();
                }
                if screen == Screen::Variables && !self.state.variables.loaded {
                    let scope = self.state.variables.scope;
                    self.load_variables(scope);
                }
                if screen == Screen::Settings && !self.state.settings_screen.is_loaded() {
                    self.load_settings();
                }
            }
            Action::SetEnvironment(id) => self.set_environment(id),
            Action::OpenDefinition(scope, name) => {
                self.variables_navigate(VarTransition::OpenDefinition(scope, name));
            }
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
            Outcome::SettingsGuard(choice) => {
                self.state.dialog = None;
                self.settings_choice(choice);
            }
            Outcome::Move(node, target) => {
                self.state.dialog = None;
                self.move_node(node, target);
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
            Outcome::Curl => {
                if let Some(Dialog::Curl { text }) = self.state.dialog.take()
                    && let Ok(parsed) = poster_app::core::curl::parse(&text)
                {
                    let request = Request {
                        template: parsed.template,
                        ..Request::default()
                    };
                    self.navigate(Navigation::Restore {
                        request: Box::new(request),
                        status: "Opened from curl as an unsaved draft — Save to add it to a collection",
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
            let result = executor::curl_command(&db, &request).await;
            Message::Curl(result.map_err(failure))
        });
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
            self.pick(
                FileDialog::Save {
                    title: "Export collection",
                    name: "collection.poster.json",
                    filter: Some(("Poster", "json")),
                },
                move |paths| Message::NativeExportPicked {
                    scope,
                    path: paths.into_iter().next(),
                },
            );
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
            move |db| match scope {
                Scope::Global => environments::globals(db),
                Scope::Environment(id) => environments::environment(db, id)?
                    .map(|e| e.variables)
                    .ok_or(StoreError::NotFound("environment")),
                Scope::Collection(id) => collections::collection(db, id)?
                    .map(|c| c.variables)
                    .ok_or(StoreError::NotFound("collection")),
            },
            move |result| Message::VariablesLoaded {
                scope,
                result: result.map_err(failure),
            },
        );
    }

    fn variables_loaded(&mut self, scope: Scope, result: Result<Variables, Failure>) {
        let screen = &mut self.state.variables;
        if scope != screen.scope {
            return;
        }
        match result {
            Ok(variables) => {
                screen.rows = vars::rows(&variables);
                screen.baseline = vars::entries(&screen.rows);
                screen.loaded = true;
                screen.status = None;
            }
            Err(error) => {
                screen.status = Some((
                    format!("Variables could not be loaded: {error}"),
                    Tone::Error,
                ));
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
                self.state.dialog = Some(Dialog::name(
                    "New Environment",
                    "Environment Name:",
                    "",
                    NamePurpose::NewEnvironment,
                ));
            }
            VarTransition::RenameEnvironment(id) => {
                let current = self.environment_name(id);
                self.state.dialog = Some(Dialog::name(
                    "Rename Environment",
                    "Name:",
                    &current,
                    NamePurpose::RenameEnvironment(id),
                ));
            }
            VarTransition::DeleteEnvironment(id) => {
                let current = self.environment_name(id);
                self.state.dialog = Some(Dialog::Confirm {
                    title: "Delete Environment".into(),
                    message: format!("Delete the environment \"{current}\" and its variables?"),
                    confirm: "Delete".into(),
                    purpose: ConfirmPurpose::DeleteEnvironment(id),
                });
            }
            VarTransition::OpenDefinition(scope, name) => {
                self.state.screen = Screen::Variables;
                self.state.variables.highlight = Some(name);
                self.load_variables(scope);
            }
            VarTransition::Close => self.settings_close(),
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
        screen.rows = screen
            .baseline
            .iter()
            .map(|(name, value)| vars::Row::new(name.clone(), value.clone()))
            .collect();
        screen.status = None;
    }

    fn save_variables(&mut self) {
        let screen = &mut self.state.variables;
        if !screen.loaded || screen.saving {
            return;
        }
        let variables = match vars::to_variables(&screen.rows) {
            Ok(variables) => variables,
            Err(error) => {
                screen.status = Some((error.to_string(), Tone::Error));
                if screen.pending.take() == Some(VarTransition::Close) {
                    self.state.close = CloseState::Open;
                }
                return;
            }
        };
        screen.saving = true;
        let scope = screen.scope;
        self.task(
            move |db| match scope {
                Scope::Global => environments::set_globals(db, &variables),
                Scope::Environment(id) => {
                    environments::set_environment_variables(db, id, &variables)
                }
                Scope::Collection(id) => collections::set_collection_variables(db, id, &variables),
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
                    screen.status = Some(("Saved".into(), Tone::Muted));
                }
                if let Some(transition) = screen.pending.take() {
                    self.variables_perform(transition);
                }
                self.load_snapshot();
            }
            Err(error) => {
                screen.status = Some((
                    format!("Save failed, changes retained: {error}"),
                    Tone::Error,
                ));
                if screen.pending.take() == Some(VarTransition::Close) {
                    self.state.close = CloseState::Open;
                }
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
            VariablesAction::Revert => self.revert_variables(),
            VariablesAction::Edited => {
                let screen = &mut self.state.variables;
                screen.status = screen
                    .dirty()
                    .then(|| ("Unsaved".to_owned(), Tone::Warning));
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
            Err(_) => {
                screen.status = Some(("Settings could not be loaded".into(), Tone::Error));
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
            screen.status = Some((format!("Failed to save settings: {error}"), Tone::Error));
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
                screen.status = Some(if screen.dirty() {
                    ("Unsaved".into(), Tone::Warning)
                } else {
                    ("Saved".into(), Tone::Muted)
                });
                if !settings.autosave {
                    self.state.autosave_due = None;
                }
                self.state.settings = settings;
                if self.state.history.loaded {
                    self.load_history();
                }
                if std::mem::take(&mut self.state.settings_screen.closing) {
                    self.finish_close();
                }
            }
            Err(error) => {
                screen.status = Some((format!("Failed to save settings: {error}"), Tone::Error));
                self.settings_close_failed();
            }
        }
    }

    /// The last close step before writing UI state: unsaved settings ask first.
    fn settings_close(&mut self) {
        if self.state.settings_screen.dirty() {
            self.state.settings_screen.closing = true;
            self.state.dialog = Some(Dialog::SettingsGuard);
        } else {
            self.finish_close();
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
                self.finish_close();
            }
            GuardChoice::Cancel => self.settings_close_failed(),
        }
    }

    fn finish_close(&mut self) {
        self.state.close = CloseState::Flushing;
        self.flush_ui_state();
    }

    // ---- history --------------------------------------------------------------------------------

    fn load_history(&mut self) {
        let history = &mut self.state.history;
        history.generation += 1;
        let generation = history.generation;
        let offset = history.offset;
        let filter = if history.only_current {
            // No open saved request: an id that matches nothing, as the Python app did.
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
        self.task(
            move |db| executions::page(db, filter, offset, HISTORY_PAGE),
            move |result| Message::HistoryPage {
                generation,
                result: result.map_err(failure),
            },
        );
    }

    fn history_page(&mut self, generation: u64, result: Result<Vec<ExecutionSummary>, Failure>) {
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
                history.status = Some(if rows.is_empty() {
                    ("No executions yet".to_owned(), Tone::Muted)
                } else {
                    (
                        format!(
                            "Executions {}-{}",
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
            HistoryAction::Refresh => {
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
                        status: "Restored unsaved draft — Send to execute, Save to add to the selected collection",
                    });
                }
            }
            HistoryAction::CopyCurl => {
                if let Some(record) = &self.state.history.record {
                    match executor::recorded_curl(record) {
                        Ok(command) => {
                            self.state.clipboard = Some(command);
                            self.state.history.status = Some(("Copied as curl".into(), Tone::Ok));
                        }
                        Err(error) => self.error("Copy as curl failed", error.to_string()),
                    }
                }
            }
            HistoryAction::Clear => {
                self.state.dialog = Some(Dialog::Confirm {
                    title: "Clear all history".into(),
                    message: "Delete all execution history? Saved requests are not affected."
                        .into(),
                    confirm: "Delete history".into(),
                    purpose: ConfirmPurpose::ClearHistory,
                });
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

impl eframe::App for PosterApp {
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
        crate::workspace::draw(self, ui);
        let now = ui.input(|i| i.time);
        for action in std::mem::take(&mut self.state.actions) {
            self.handle(action, now);
        }
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
