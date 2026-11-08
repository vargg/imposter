//! The collections folder on disk (format `imposter-project`, version 1): one file per
//! request, one per folder, one per collection and one per environment, so an edit is a
//! diff of its own lines.
//!
//! ```text
//! environments/dev.yaml       an environment
//! shop-api/imposter.yaml      a collection
//! shop-api/login.yaml         a request in it
//! shop-api/users/folder.yaml  a folder
//! ```
//!
//! File names come from names (`Shop API` → `shop-api`); identity is the `id` inside each
//! file, and order is its `position`, so a reorder renames nothing. Reading is strict, as
//! the native format is: unknown fields, duplicate ids and unexpected YAML files are
//! refused, naming the file. Secrets, local values, globals and History are never here.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use imposter_core::capture::Capture;
use imposter_core::check::Check;
use imposter_core::model::{
    AuthKind, BearerToken, BodyType, Credentials, EnvColor, FormItem, FormKind, Options, ParamItem,
    RequestAuth, RequestTemplate, SavedValues, Scripts, Variables,
};
use serde::{Deserialize, Serialize};

/// What the folder's collection files say they are.
pub const FORMAT: &str = "imposter-project";
pub const VERSION: u32 = 1;

pub const COLLECTION_FILE: &str = "imposter.yaml";
pub const FOLDER_FILE: &str = "folder.yaml";
pub const ENVIRONMENTS_DIR: &str = "environments";

/// Every collection and environment in a folder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub description: String,
    pub variables: Variables,
    pub saved_values: SavedValues,
    pub scripts: Scripts,
    pub folders: Vec<Folder>,
    pub requests: Vec<Request>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub description: String,
    pub scripts: Scripts,
    pub folders: Vec<Folder>,
    pub requests: Vec<Request>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: String,
    pub template: RequestTemplate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub color: Option<EnvColor>,
    /// Off: the file lists names only, and values stay in the user's profile.
    pub share_values: bool,
    pub variables: Variables,
    pub saved_values: SavedValues,
}

/// A file that could not be read, by its path in the folder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct FormatError {
    pub path: String,
    pub message: String,
}

fn error(path: &str, message: impl Into<String>) -> FormatError {
    FormatError {
        path: path.to_owned(),
        message: message.into(),
    }
}

// ---- file names ------------------------------------------------------------------------

/// A name as a file or directory name: lowercase ASCII letters and digits, `-` for
/// everything else, never two in a row, at most 60 characters; `fallback` when nothing is
/// left.
pub fn slug(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.trim_end_matches('-').chars().take(60).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out
    }
}

/// `base`, or `base-2`, `base-3`… when `taken` (or `reserved`) already has it.
fn unique(base: &str, taken: &mut HashSet<String>, reserved: &[&str]) -> String {
    let mut candidate = base.to_owned();
    let mut n = 2;
    while reserved.contains(&candidate.as_str()) || taken.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

// ---- the files' own shapes -------------------------------------------------------------

fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_true(value: &bool) -> bool {
    *value
}

fn get() -> String {
    "GET".to_owned()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionFile {
    format: String,
    version: u32,
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Variables::is_empty")]
    variables: Variables,
    #[serde(default, skip_serializing_if = "SavedValues::is_empty")]
    saved_values: SavedValues,
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    scripts: Scripts,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderFile {
    id: String,
    name: String,
    #[serde(default)]
    position: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    scripts: Scripts,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentFile {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<EnvColor>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    share_values: bool,
    #[serde(default, skip_serializing_if = "Variables::is_empty")]
    variables: Variables,
    #[serde(default, skip_serializing_if = "SavedValues::is_empty")]
    saved_values: SavedValues,
}

/// A header, query parameter, cookie or form field; `enabled` only when off.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Param {
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
}

impl From<&ParamItem> for Param {
    fn from(item: &ParamItem) -> Self {
        Self {
            name: item.name.clone(),
            value: item.value.clone(),
            enabled: item.is_enabled,
        }
    }
}

impl From<Param> for ParamItem {
    fn from(param: Param) -> Self {
        Self {
            name: param.name,
            value: param.value,
            is_enabled: param.enabled,
        }
    }
}

/// A multipart field: text in `value`, or a file by path in `file`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    content_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    filename: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
}

/// The body by its type, as text a person can read and diff.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
enum Body {
    Json(String),
    Plain(String),
    Text(String),
    FormData(Vec<Param>),
    Urlencoded(Vec<Param>),
    Multipart(Vec<Part>),
    /// A binary body: the path of the file sent.
    File(String),
    /// Bytes that are not text (or a form that does not parse), kept as base64.
    Bytes {
        body_type: BodyType,
        base64: String,
    },
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Auth {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<AuthKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    basic: Option<Credentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    digest: Option<Credentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bearer_token: Option<BearerToken>,
}

impl Auth {
    fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.basic.is_none()
            && self.digest.is_none()
            && self.bearer_token.is_none()
    }
}

/// The request's settings, only those it sets.
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    follow_redirects: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verify_ssl: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attach_cookies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ca_bundle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    certificate_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    download_limit: Option<u64>,
}

impl Settings {
    fn is_empty(&self) -> bool {
        self.to_options().is_empty()
    }

    fn to_options(&self) -> Options {
        Options {
            follow_redirects: self.follow_redirects,
            verify_ssl: self.verify_ssl,
            attach_cookies: self.attach_cookies,
            proxy_url: self.proxy_url.clone(),
            timeout: self.timeout,
            ca_bundle: self.ca_bundle.clone(),
            certificate_path: self.certificate_path.clone(),
            key_file: self.key_file.clone(),
            password: self.password.clone(),
            download_limit: self.download_limit,
        }
    }

    fn from_options(options: &Options) -> Self {
        Self {
            follow_redirects: options.follow_redirects,
            verify_ssl: options.verify_ssl,
            attach_cookies: options.attach_cookies,
            proxy_url: options.proxy_url.clone(),
            timeout: options.timeout,
            ca_bundle: options.ca_bundle.clone(),
            certificate_path: options.certificate_path.clone(),
            key_file: options.key_file.clone(),
            password: options.password.clone(),
            download_limit: options.download_limit,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestFile {
    id: String,
    #[serde(default)]
    position: i64,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default = "get")]
    method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    protocol: String,
    #[serde(default)]
    url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    #[serde(default, skip_serializing_if = "Auth::is_empty")]
    auth: Auth,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    queries: Vec<Param>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    headers: Vec<Param>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cookies: Vec<Param>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Body>,
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    settings: Settings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checks: Vec<Check>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    captures: Vec<Capture>,
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    scripts: Scripts,
}

fn body_of(template: &RequestTemplate) -> Option<Body> {
    use base64::Engine as _;
    let bytes = || Body::Bytes {
        body_type: template.body_type,
        base64: base64::engine::general_purpose::URL_SAFE.encode(&template.body),
    };
    let text = || String::from_utf8(template.body.clone()).ok();
    Some(match template.body_type {
        BodyType::None => return None,
        BodyType::Json => text().map_or_else(bytes, Body::Json),
        BodyType::Plain => text().map_or_else(bytes, Body::Plain),
        BodyType::Text => text().map_or_else(bytes, Body::Text),
        BodyType::Binary => text().map_or_else(bytes, Body::File),
        BodyType::FormData | BodyType::Urlencoded => match template.form_fields() {
            Ok(fields) => {
                let params = fields.iter().map(Param::from).collect();
                if template.body_type == BodyType::FormData {
                    Body::FormData(params)
                } else {
                    Body::Urlencoded(params)
                }
            }
            Err(_) => bytes(),
        },
        BodyType::Multipart => match template.multipart_fields() {
            Ok(fields) => Body::Multipart(
                fields
                    .into_iter()
                    .map(|f| Part {
                        name: f.name,
                        value: (f.kind == FormKind::Text).then(|| f.value.clone()),
                        file: (f.kind == FormKind::File).then_some(f.value),
                        content_type: f.content_type,
                        filename: f.filename,
                        enabled: f.is_enabled,
                    })
                    .collect(),
            ),
            Err(_) => bytes(),
        },
    })
}

fn set_body(
    template: &mut RequestTemplate,
    body: Option<Body>,
    path: &str,
) -> Result<(), FormatError> {
    use base64::Engine as _;
    let Some(body) = body else {
        template.body_type = BodyType::None;
        template.body = Vec::new();
        return Ok(());
    };
    match body {
        Body::Json(text) => {
            (template.body_type, template.body) = (BodyType::Json, text.into_bytes());
        }
        Body::Plain(text) => {
            (template.body_type, template.body) = (BodyType::Plain, text.into_bytes());
        }
        Body::Text(text) => {
            (template.body_type, template.body) = (BodyType::Text, text.into_bytes());
        }
        Body::File(path) => {
            (template.body_type, template.body) = (BodyType::Binary, path.into_bytes());
        }
        Body::FormData(params) => {
            template.body_type = BodyType::FormData;
            let fields: Vec<ParamItem> = params.into_iter().map(ParamItem::from).collect();
            template.set_form_fields(&fields);
        }
        Body::Urlencoded(params) => {
            template.body_type = BodyType::Urlencoded;
            let fields: Vec<ParamItem> = params.into_iter().map(ParamItem::from).collect();
            template.set_form_fields(&fields);
        }
        Body::Multipart(parts) => {
            template.body_type = BodyType::Multipart;
            let mut fields = Vec::with_capacity(parts.len());
            for part in parts {
                let (kind, value) = match (part.value, part.file) {
                    (Some(value), None) => (FormKind::Text, value),
                    (None, Some(file)) => (FormKind::File, file),
                    _ => {
                        return Err(error(
                            path,
                            format!("multipart field {:?} needs one of value or file", part.name),
                        ));
                    }
                };
                fields.push(FormItem {
                    name: part.name,
                    value,
                    is_enabled: part.enabled,
                    kind,
                    content_type: part.content_type,
                    filename: part.filename,
                });
            }
            template.set_multipart_fields(&fields);
        }
        Body::Bytes { body_type, base64 } => {
            template.body_type = body_type;
            template.body = base64::engine::general_purpose::URL_SAFE
                .decode(base64.trim())
                .or_else(|_| base64::engine::general_purpose::STANDARD.decode(base64.trim()))
                .map_err(|_| error(path, "body bytes are not base64"))?;
        }
    }
    Ok(())
}

impl RequestFile {
    fn new(request: &Request) -> Self {
        let t = &request.template;
        Self {
            id: request.id.clone(),
            position: t.position,
            name: t.name.clone(),
            description: t.description.clone(),
            method: t.method.clone(),
            protocol: t.protocol.clone(),
            url: t.url.clone(),
            path: t.path.clone(),
            auth: Auth {
                kind: t.auth.kind,
                basic: t.auth.basic.clone(),
                digest: t.auth.digest.clone(),
                bearer_token: t.auth.bearer_token.clone(),
            },
            queries: t.queries.iter().map(Param::from).collect(),
            headers: t.headers.iter().map(Param::from).collect(),
            cookies: t.cookies.iter().map(Param::from).collect(),
            body: body_of(t),
            settings: Settings::from_options(&t.options),
            checks: t.checks.clone(),
            captures: t.captures.clone(),
            scripts: t.scripts.clone(),
        }
    }

    fn into_request(self, path: &str) -> Result<Request, FormatError> {
        let mut template = RequestTemplate {
            position: self.position,
            name: self.name,
            description: self.description,
            method: self.method,
            protocol: self.protocol,
            url: self.url,
            path: self.path,
            auth: RequestAuth {
                kind: self.auth.kind,
                basic: self.auth.basic,
                digest: self.auth.digest,
                bearer_token: self.auth.bearer_token,
            },
            queries: self.queries.into_iter().map(ParamItem::from).collect(),
            headers: self.headers.into_iter().map(ParamItem::from).collect(),
            cookies: self.cookies.into_iter().map(ParamItem::from).collect(),
            options: self.settings.to_options(),
            checks: self.checks,
            captures: self.captures,
            scripts: self.scripts,
            ..RequestTemplate::default()
        };
        if let Some(timeout) = template.options.timeout
            && !(timeout > 0.0 && timeout.is_finite())
        {
            return Err(error(path, "timeout must be a positive number of seconds"));
        }
        set_body(&mut template, self.body, path)?;
        if template.auth.active().is_err() {
            return Err(error(path, "authentication is missing its configuration"));
        }
        Ok(Request {
            id: self.id,
            template,
        })
    }
}

// ---- rendering -------------------------------------------------------------------------

/// YAML as these files are written: literal blocks for multi-line text, long lines never
/// folded, so a value reads and diffs as typed.
fn yaml<T: Serialize>(value: &T) -> Vec<u8> {
    let options = serde_saphyr::ser_options! {
        folded_wrap_chars: 1_000_000,
    };
    serde_saphyr::to_string_with_options(value, options)
        .expect("the project files are plain data and always serialize")
        .into_bytes()
}

/// Siblings in the order the tree shows them: by position, then id.
fn ordered<T>(items: &[T], key: impl Fn(&T) -> (i64, &str)) -> Vec<&T> {
    let mut sorted: Vec<&T> = items.iter().collect();
    sorted.sort_by(|a, b| key(a).cmp(&key(b)));
    sorted
}

/// Every file of `project`, by its path in the folder (`/`-separated).
pub fn render(project: &Project) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut taken = HashSet::new();
    for environment in ordered(&project.environments, |e| (0, e.id.as_str())) {
        let name = unique(&slug(&environment.name, "environment"), &mut taken, &[]);
        let file = EnvironmentFile {
            id: environment.id.clone(),
            name: environment.name.clone(),
            color: environment.color,
            share_values: environment.share_values,
            variables: if environment.share_values {
                environment.variables.clone()
            } else {
                environment
                    .variables
                    .keys()
                    .map(|k| (k.clone(), String::new()))
                    .collect()
            },
            saved_values: if environment.share_values {
                environment.saved_values.clone()
            } else {
                SavedValues::default()
            },
        };
        files.insert(format!("{ENVIRONMENTS_DIR}/{name}.yaml"), yaml(&file));
    }
    let mut taken = HashSet::new();
    for collection in ordered(&project.collections, |c| (0, c.id.as_str())) {
        let dir = unique(
            &slug(&collection.name, "collection"),
            &mut taken,
            &[ENVIRONMENTS_DIR],
        );
        let file = CollectionFile {
            format: FORMAT.to_owned(),
            version: VERSION,
            id: collection.id.clone(),
            name: collection.name.clone(),
            description: collection.description.clone(),
            variables: collection.variables.clone(),
            saved_values: collection.saved_values.clone(),
            scripts: collection.scripts.clone(),
        };
        files.insert(format!("{dir}/{COLLECTION_FILE}"), yaml(&file));
        render_level(
            &mut files,
            &dir,
            &collection.folders,
            &collection.requests,
            true,
        );
    }
    files
}

fn render_level(
    files: &mut BTreeMap<String, Vec<u8>>,
    dir: &str,
    folders: &[Folder],
    requests: &[Request],
    top: bool,
) {
    let reserved: &[&str] = if top {
        &["imposter", "folder"]
    } else {
        &["folder"]
    };
    let mut taken_files = HashSet::new();
    for request in ordered(requests, |r| (r.template.position, r.id.as_str())) {
        let name = unique(
            &slug(&request.template.name, "request"),
            &mut taken_files,
            reserved,
        );
        files.insert(
            format!("{dir}/{name}.yaml"),
            yaml(&RequestFile::new(request)),
        );
    }
    let mut taken_dirs = HashSet::new();
    for folder in ordered(folders, |f| (f.position, f.id.as_str())) {
        let name = unique(&slug(&folder.name, "folder"), &mut taken_dirs, &[]);
        let path = format!("{dir}/{name}");
        let file = FolderFile {
            id: folder.id.clone(),
            name: folder.name.clone(),
            position: folder.position,
            description: folder.description.clone(),
            scripts: folder.scripts.clone(),
        };
        files.insert(format!("{path}/{FOLDER_FILE}"), yaml(&file));
        render_level(files, &path, &folder.folders, &folder.requests, false);
    }
}

// ---- reading ---------------------------------------------------------------------------

/// The most a project file may hold in YAML nodes, against alias bombs.
fn parse<T: serde::de::DeserializeOwned>(path: &str, bytes: &[u8]) -> Result<T, FormatError> {
    let nodes = (bytes.len() / 2).clamp(10_000, 2_000_000);
    let budget = serde_saphyr::budget! {
        max_nodes: nodes,
        max_events: nodes * 2,
    };
    let options = serde_saphyr::options! {
        budget: budget,
    };
    serde_saphyr::from_slice_with_options(bytes, options).map_err(|e| {
        let text = e.to_string();
        error(
            path,
            text.lines().next().unwrap_or("not valid YAML").to_owned(),
        )
    })
}

/// A folder's files back into a project. `files` holds every `.yaml` file under the
/// folder by its `/`-separated path; anything else in the folder is not part of it.
pub fn read(files: &BTreeMap<String, Vec<u8>>) -> Result<Project, FormatError> {
    let mut project = Project::default();
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut claim = |id: &str, path: &str| -> Result<(), FormatError> {
        if id.trim().is_empty() {
            return Err(error(path, "id must not be empty"));
        }
        if let Some(other) = ids.insert(id.to_owned(), path.to_owned()) {
            return Err(error(path, format!("id {id} is also used by {other}")));
        }
        Ok(())
    };
    // Group by top-level directory: environments, then one per collection.
    let mut collections: BTreeMap<&str, BTreeMap<&str, &[u8]>> = BTreeMap::new();
    for (path, bytes) in files {
        let Some((top, rest)) = path.split_once('/') else {
            // Loose files at the top belong to whatever else shares the folder (a README).
            continue;
        };
        if top == ENVIRONMENTS_DIR {
            if rest.contains('/') {
                return Err(error(
                    path,
                    "environments are files directly in environments/",
                ));
            }
            let file: EnvironmentFile = parse(path, bytes)?;
            claim(&file.id, path)?;
            project.environments.push(Environment {
                id: file.id,
                name: name(path, file.name)?,
                color: file.color,
                share_values: file.share_values,
                variables: file.variables,
                saved_values: file.saved_values,
            });
            continue;
        }
        collections.entry(top).or_default().insert(rest, bytes);
    }
    for (dir, files) in collections {
        // A directory without a collection file is not ours (a repo's other folders).
        let Some(bytes) = files.get(COLLECTION_FILE) else {
            continue;
        };
        let path = format!("{dir}/{COLLECTION_FILE}");
        let file: CollectionFile = parse(&path, bytes)?;
        if file.format != FORMAT {
            return Err(error(&path, format!("format must be {FORMAT:?}")));
        }
        if file.version != VERSION {
            return Err(error(
                &path,
                format!("unsupported version {}", file.version),
            ));
        }
        claim(&file.id, &path)?;
        let mut collection = Collection {
            id: file.id,
            name: name(&path, file.name)?,
            description: file.description,
            variables: file.variables,
            saved_values: file.saved_values,
            scripts: file.scripts,
            ..Collection::default()
        };
        let (folders, requests) = read_level(dir, "", &files, &mut claim)?;
        collection.folders = folders;
        collection.requests = requests;
        project.collections.push(collection);
    }
    Ok(project)
}

fn name(path: &str, name: String) -> Result<String, FormatError> {
    if name.trim().is_empty() {
        Err(error(path, "name must not be empty"))
    } else {
        Ok(name)
    }
}

type Claim<'a> = dyn FnMut(&str, &str) -> Result<(), FormatError> + 'a;

/// The folders and requests directly under `prefix` (`""` for the collection's own).
fn read_level(
    dir: &str,
    prefix: &str,
    files: &BTreeMap<&str, &[u8]>,
    claim: &mut Claim<'_>,
) -> Result<(Vec<Folder>, Vec<Request>), FormatError> {
    let mut requests = Vec::new();
    let mut subdirs: BTreeSet<&str> = BTreeSet::new();
    for (rel, bytes) in files.range(prefix..) {
        let Some(rest) = rel.strip_prefix(prefix) else {
            break;
        };
        let path = format!("{dir}/{rel}");
        match rest.split_once('/') {
            Some((sub, _)) => {
                subdirs.insert(sub);
            }
            None if rest == COLLECTION_FILE && prefix.is_empty() => {}
            None if rest == FOLDER_FILE && !prefix.is_empty() => {}
            None => {
                let file: RequestFile = parse(&path, bytes)?;
                claim(&file.id, &path)?;
                let request = file.into_request(&path)?;
                if request.template.name.trim().is_empty() {
                    return Err(error(&path, "name must not be empty"));
                }
                requests.push(request);
            }
        }
    }
    let mut folders = Vec::new();
    for sub in &subdirs {
        let inner = format!("{prefix}{sub}/");
        let folder_path = format!("{dir}/{inner}{FOLDER_FILE}");
        let Some(bytes) = files.get(format!("{inner}{FOLDER_FILE}").as_str()) else {
            return Err(error(
                &format!("{dir}/{prefix}{sub}"),
                format!("a folder needs its {FOLDER_FILE}"),
            ));
        };
        let file: FolderFile = parse(&folder_path, bytes)?;
        claim(&file.id, &folder_path)?;
        let (children, inner_requests) = read_level(dir, &inner, files, claim)?;
        folders.push(Folder {
            id: file.id,
            name: name(&folder_path, file.name)?,
            position: file.position,
            description: file.description,
            scripts: file.scripts,
            folders: children,
            requests: inner_requests,
        });
    }
    // Siblings in the order the tree shows them, whatever their file names.
    requests.sort_by(|a, b| (a.template.position, &a.id).cmp(&(b.template.position, &b.id)));
    folders.sort_by(|a, b| (a.position, &a.id).cmp(&(b.position, &b.id)));
    Ok((folders, requests))
}

/// Every `.yaml` file under `root` by its `/`-separated path, skipping hidden entries
/// (`.git`). A missing folder reads as empty.
pub fn read_dir(root: &std::path::Path) -> std::io::Result<BTreeMap<String, Vec<u8>>> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut BTreeMap<String, Vec<u8>>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(root, &path, out)?;
            } else if std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("yaml"))
            {
                let rel = path
                    .strip_prefix(root)
                    .map_err(std::io::Error::other)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, std::fs::read(&path)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    if root.exists() {
        walk(root, root, &mut out)?;
    }
    Ok(out)
}
