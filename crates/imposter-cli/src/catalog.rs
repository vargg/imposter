//! `imposter list`, `show` and `envs`: what an agent or a person reads before running
//! anything. They read the profile (or a `--file` or `--project`) and write nothing; output
//! is JSON, or
//! readable text with `--text`.

use std::fmt::Write as _;
use std::path::PathBuf;

use imposter_app::catalog::{self, AuthView, Environments, Listing, RequestView};
use imposter_app::document::RequestPath;
use imposter_app::formats::native::{self, NativeDocument};
use imposter_app::paths;
use imposter_app::store::Store;
use imposter_core::run_result::Source;

use crate::send::DEFAULT_MAX_BODY;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    List,
    Show,
    Envs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogArgs {
    pub kind: Kind,
    /// `list`: the filter, in the sidebar's language; `show`: the request's path.
    pub target: String,
    pub environment: Option<String>,
    pub file: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub max_body: usize,
    pub text: bool,
}

impl Default for CatalogArgs {
    fn default() -> Self {
        Self {
            kind: Kind::List,
            target: String::new(),
            environment: None,
            file: None,
            project: None,
            max_body: DEFAULT_MAX_BODY,
            text: false,
        }
    }
}

/// Where the definitions are read: a file, a collections folder, or the profile (closed
/// when dropped).
enum Opened {
    File(Box<NativeDocument>),
    Project(Box<imposter_app::project::ProjectFolder>),
    Profile(Store),
}

/// Open the file `--file` names, the folder `--project` names, or the personal profile.
pub(crate) async fn open(
    file: Option<&std::path::Path>,
    project: Option<&std::path::Path>,
    var: &dyn Fn(&str) -> Option<String>,
) -> Result<OpenedSource, String> {
    if file.is_none()
        && let Some(dir) = project
    {
        let dir = dir.to_owned();
        let folder = tokio::task::spawn_blocking(move || imposter_app::project::read(&dir))
            .await
            .map_err(|_| "The collections folder could not be read".to_owned())?
            .map_err(|e| e.to_string())?;
        return Ok(OpenedSource(Opened::Project(Box::new(folder))));
    }
    if let Some(file) = file {
        let file = file.to_owned();
        let bytes = tokio::task::spawn_blocking(move || native::read_document(&file))
            .await
            .map_err(|_| "The file could not be read".to_owned())?
            .map_err(|e| e.to_string())?;
        return Ok(OpenedSource(Opened::File(Box::new(
            native::decode(&bytes).map_err(|e| e.to_string())?,
        ))));
    }
    let explicit = var(paths::DATA_DIR_VARIABLE).map(std::ffi::OsString::from);
    let store = tokio::task::spawn_blocking(move || {
        paths::open_existing_profile(explicit, std::env::home_dir())
    })
    .await
    .map_err(|_| "The profile could not be opened".to_owned())?
    .map_err(|e| e.to_string())?;
    Ok(OpenedSource(Opened::Profile(store)))
}

/// A source the commands read from.
pub(crate) struct OpenedSource(Opened);

impl OpenedSource {
    /// Close the profile, waiting for its database thread; a file needs nothing.
    async fn close(self) {
        if let Opened::Profile(store) = self.0 {
            let _ = tokio::task::spawn_blocking(move || store.close()).await;
        }
    }
}

/// Run `args` and return what to print, or why nothing could be read.
pub async fn run(
    args: &CatalogArgs,
    var: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let source = open(args.file.as_deref(), args.project.as_deref(), var).await?;
    let output = render(args, &source.0).await;
    source.close().await;
    output
}

async fn render(args: &CatalogArgs, source: &Opened) -> Result<String, String> {
    match args.kind {
        Kind::List => {
            let listing = match source {
                Opened::File(document) => catalog::list_file(document),
                Opened::Project(folder) => {
                    let mut listing = catalog::list_file(&folder.documents[0]);
                    for document in &folder.documents[1..] {
                        listing
                            .requests
                            .extend(catalog::list_file(document).requests);
                    }
                    listing.source = Source::Project;
                    listing
                }
                Opened::Profile(store) => catalog::list_profile(&store.handle())
                    .await
                    .map_err(|e| e.to_string())?,
            }
            .filtered(&args.target);
            Ok(output(args.text, &listing, list_text))
        }
        Kind::Show => {
            let path = RequestPath::parse(&args.target).map_err(|e| e.to_string())?;
            let environment = args.environment.as_deref();
            let view = match source {
                Opened::File(document) => {
                    catalog::show_file(document, &path, environment, args.max_body)
                        .map_err(|e| e.to_string())?
                }
                Opened::Project(folder) => {
                    let document = folder.document(&path).map_err(|e| e.to_string())?;
                    let mut view = catalog::show_file(document, &path, environment, args.max_body)
                        .map_err(|e| e.to_string())?;
                    view.source = Source::Project;
                    view
                }
                Opened::Profile(store) => {
                    catalog::show_profile(&store.handle(), &path, environment, args.max_body)
                        .await
                        .map_err(|e| e.to_string())?
                }
            };
            Ok(output(args.text, &view, show_text))
        }
        Kind::Envs => {
            let environments = match source {
                Opened::File(document) => catalog::environments_file(document),
                Opened::Project(folder) => {
                    let mut environments = catalog::environments_file(&folder.documents[0]);
                    for document in &folder.documents[1..] {
                        environments
                            .collections
                            .extend(catalog::environments_file(document).collections);
                    }
                    environments.source = Source::Project;
                    environments
                }
                Opened::Profile(store) => catalog::environments_profile(&store.handle())
                    .await
                    .map_err(|e| e.to_string())?,
            };
            Ok(output(args.text, &environments, envs_text))
        }
    }
}

fn output<T: serde::Serialize>(text: bool, value: &T, as_text: fn(&T) -> String) -> String {
    if text {
        as_text(value)
    } else {
        let mut json = serde_json::to_string_pretty(value).unwrap_or_default();
        json.push('\n');
        json
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

fn list_text(listing: &Listing) -> String {
    let width = listing
        .requests
        .iter()
        .map(|r| r.path.chars().count())
        .max()
        .unwrap_or(0);
    let methods = listing
        .requests
        .iter()
        .map(|r| r.method.chars().count())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for request in &listing.requests {
        let mut notes = Vec::new();
        if request.checks > 0 {
            notes.push(plural(request.checks, "check"));
        }
        if request.captures > 0 {
            notes.push(plural(request.captures, "capture"));
        }
        if request.scripts {
            notes.push("scripts".to_owned());
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("  ({})", notes.join(", "))
        };
        // Writing to a String cannot fail.
        let _ = writeln!(
            text,
            "{:<methods$}  {:<width$}  {}{notes}",
            request.method, request.path, request.url
        );
    }
    if listing.requests.is_empty() {
        text.push_str("No saved requests match.\n");
    }
    text
}

fn rows(text: &mut String, title: &str, rows: &[catalog::Item], separator: &str) {
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(text, "{title}:");
    for row in rows {
        let off = if row.enabled { "" } else { "  (off)" };
        let _ = writeln!(text, "  {}{separator}{}{off}", row.name, row.value);
    }
}

fn show_text(view: &RequestView) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{} {}", view.method, view.url);
    let _ = writeln!(text, "Path: {}", view.path);
    let _ = writeln!(
        text,
        "Environment: {}",
        view.environment
            .as_deref()
            .unwrap_or("none (globals and collection only)")
    );
    if !view.description.is_empty() {
        let _ = writeln!(text, "Description: {}", view.description);
    }
    rows(&mut text, "Query", &view.query, "=");
    rows(&mut text, "Headers", &view.headers, ": ");
    rows(&mut text, "Cookies", &view.cookies, "=");
    match &view.auth {
        AuthView::None => {}
        AuthView::Basic { username, password } => {
            let _ = writeln!(text, "Auth: Basic {username} / {password}");
        }
        AuthView::Digest { username, password } => {
            let _ = writeln!(text, "Auth: Digest {username} / {password}");
        }
        AuthView::Bearer { token } => {
            let _ = writeln!(text, "Auth: Bearer {token}");
        }
    }
    if view.body.bytes > 0 {
        let kind = serde_json::to_value(view.body.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = writeln!(text, "Body ({kind}, {} bytes):", view.body.bytes);
        match &view.body.text {
            Some(body) => {
                for line in body.lines() {
                    let _ = writeln!(text, "  {line}");
                }
                if view.body.cut {
                    text.push_str("  …\n");
                }
            }
            None => text.push_str("  (not text)\n"),
        }
    }
    if !view.options.is_empty() {
        let options: Vec<String> = view
            .options
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        let _ = writeln!(text, "Settings: {}", options.join(", "));
    }
    if !view.checks.is_empty() {
        text.push_str("Checks:\n");
        for check in &view.checks {
            let off = if check.check.enabled { "" } else { "  (off)" };
            let _ = writeln!(text, "  {}{off}", check.describe);
        }
    }
    if !view.captures.is_empty() {
        text.push_str("Captures:\n");
        for capture in &view.captures {
            let _ = writeln!(
                text,
                "  {} <- {}",
                capture.variable,
                capture.selector.describe()
            );
        }
    }
    for (level, scripts) in view
        .inherited_scripts
        .iter()
        .map(|l| (l.level.as_str(), &l.scripts))
        .chain([("This request", &view.scripts)])
    {
        for (phase, script) in [
            ("before sending", &scripts.before),
            ("after the response", &scripts.after),
        ] {
            if !script.is_empty() {
                let _ = writeln!(text, "{level}, {phase}:");
                for line in script.lines() {
                    let _ = writeln!(text, "  {line}");
                }
            }
        }
    }
    if !view.variables.is_empty() {
        text.push_str("Variables:\n");
        let width = view
            .variables
            .iter()
            .map(|v| v.name.chars().count())
            .max()
            .unwrap_or(0);
        for variable in &view.variables {
            let overrides = if variable.overrides.is_empty() {
                String::new()
            } else {
                format!(" (overrides {})", variable.overrides.join(", "))
            };
            let _ = writeln!(
                text,
                "  {:<width$}  {}{overrides}",
                variable.name, variable.defined_in
            );
        }
    }
    text
}

fn scope_line(text: &mut String, name: &str, variables: &[String]) {
    let names = if variables.is_empty() {
        "no variables".to_owned()
    } else {
        variables.join(", ")
    };
    let _ = writeln!(text, "  {name}: {names}");
}

/// Secret names under their scope's line; nothing when there are none.
fn secret_line(text: &mut String, secrets: &[String]) {
    if !secrets.is_empty() {
        let _ = writeln!(text, "    secrets: {}", secrets.join(", "));
    }
}

fn envs_text(environments: &Environments) -> String {
    let mut text = String::from("Environments:\n");
    if environments.environments.is_empty() {
        text.push_str("  none\n");
    }
    for environment in &environments.environments {
        let name = match environment.color {
            Some(imposter_app::core::model::EnvColor::Red) => {
                format!("{} (red: production)", environment.name)
            }
            Some(color) => format!(
                "{} ({})",
                environment.name,
                serde_json::to_value(color)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ),
            None => environment.name.clone(),
        };
        scope_line(&mut text, &name, &environment.variables);
        secret_line(&mut text, &environment.secrets);
    }
    text.push_str("Globals:\n");
    scope_line(&mut text, "Global", &environments.globals);
    secret_line(&mut text, &environments.global_secrets);
    text.push_str("Collections:\n");
    for collection in &environments.collections {
        scope_line(&mut text, &collection.name, &collection.variables);
    }
    text
}
