//! What the command line shows about saved requests without running them: a listing, one
//! request's definition, and the environments. Templates are shown as stored, with their
//! `{{placeholders}}`; variable and environment values never are, and a stored credential is
//! shown only when it is nothing but placeholders (`redact::shown`). Nothing is written.

use imposter_core::capture::Capture;
use imposter_core::check::Check;
use imposter_core::model::{
    Auth, BodyType, EnvColor, ParamItem, RequestTemplate, Scripts, Variables,
};
use imposter_core::redact::{SECRET_HEADERS, shown};
use imposter_core::run_result::Source;
use imposter_core::variables::{self, SecretScope, Source as Defined};
use imposter_store::{StoreError, StoreHandle, collections, environments};
use imposter_transfer::native::NativeDocument;
use serde::Serialize;

use crate::document::{self, DocumentError, RequestPath};
use crate::executor::Inputs;
use crate::filter::{Candidate, Query};
use crate::profile::{self, ProfileError, Purpose};

/// One saved request in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listed {
    pub path: String,
    pub method: String,
    pub url: String,
    pub checks: usize,
    pub captures: usize,
    /// The request, its folders or its collection have scripts.
    pub scripts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listing {
    pub source: Source,
    pub requests: Vec<Listed>,
}

impl Listing {
    /// The requests `filter` matches, in the sidebar's language; the collection's name is
    /// part of the text a word can match.
    #[must_use]
    pub fn filtered(mut self, filter: &str) -> Self {
        let query = Query::parse(filter);
        self.requests.retain(|request| {
            let shown_path = request.path.replace('/', " / ");
            let name = request.path.rsplit('/').next().unwrap_or_default();
            query.matches(&Candidate {
                path: &shown_path,
                name,
                method: &request.method,
                url: &request.url,
            })
        });
        self
    }
}

fn listed(path: &RequestPath, template: &RequestTemplate, inherited: bool) -> Listed {
    Listed {
        path: path.to_string(),
        method: template.method.clone(),
        url: template.url.clone(),
        checks: template.checks.len(),
        captures: template.captures.len(),
        scripts: inherited || !template.scripts.is_empty(),
    }
}

/// Every request in `document`, in the file's order.
pub fn list_file(document: &NativeDocument) -> Listing {
    let collection_scripts = !document.collection.scripts.is_empty();
    Listing {
        source: Source::File,
        requests: document::all(document)
            .into_iter()
            .map(|(path, found)| {
                let inherited =
                    collection_scripts || found.folders.iter().any(|f| !f.scripts.is_empty());
                let mut place = vec![document.collection.name.as_str()];
                place.extend(path.parts().iter().map(String::as_str));
                let name = place.pop().unwrap_or_default();
                listed(&RequestPath::new(&place, name), found.template, inherited)
            })
            .collect(),
    }
}

/// Every saved request, collection by collection, read in one database command.
pub async fn list_profile(store: &StoreHandle) -> Result<Listing, StoreError> {
    store
        .call(|db| {
            let mut requests = Vec::new();
            let mut collections_seen: std::collections::HashMap<_, _> =
                std::collections::HashMap::new();
            for entry in collections::request_index(db)? {
                let Some(request) = collections::request(db, entry.id)? else {
                    continue;
                };
                if !collections_seen.contains_key(&entry.collection_id)
                    && let Some(collection) = collections::collection(db, entry.collection_id)?
                {
                    collections_seen.insert(entry.collection_id, collection);
                }
                let inherited = match collections_seen.get(&entry.collection_id) {
                    Some(collection) => {
                        crate::executor::script_levels(db, collection, request.folder_id)?
                            .0
                            .iter()
                            .any(|level| !level.scripts.is_empty())
                    }
                    None => false,
                };
                let mut place = vec![entry.collection.as_str()];
                place.extend(entry.folders.iter().map(String::as_str));
                requests.push(listed(
                    &RequestPath::new(&place, &entry.name),
                    &request.template,
                    inherited,
                ));
            }
            Ok(Listing {
                source: Source::Profile,
                requests,
            })
        })
        .await
}

/// A header, query parameter or cookie as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

fn items(rows: &[ParamItem], secret: impl Fn(&str) -> bool) -> Vec<Item> {
    rows.iter()
        .map(|row| Item {
            name: row.name.clone(),
            value: if secret(&row.name) {
                shown(&row.value)
            } else {
                row.value.clone()
            },
            enabled: row.is_enabled,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthView {
    None,
    Basic { username: String, password: String },
    Digest { username: String, password: String },
    Bearer { token: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyView {
    #[serde(rename = "type")]
    pub kind: BodyType,
    /// The body as stored (a form's fields as JSON, a binary body's file path); `None` when
    /// it is not text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub bytes: usize,
    /// The text is shorter than the body.
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckView {
    pub describe: String,
    #[serde(flatten)]
    pub check: Check,
}

/// Scripts of a level that runs around the request: its collection or a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LevelView {
    pub level: String,
    #[serde(flatten)]
    pub scripts: Scripts,
}

/// A variable the request uses and where its value would come from, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VariableView {
    pub name: String,
    /// The scope in effect (`Dev`, `Global`, a collection, `Input`), `Dev secrets` or
    /// `Global secrets` for a stored secret, `generated` for a generated value, or
    /// `undefined`.
    pub defined_in: String,
    /// Scopes that also define it, overridden, highest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<String>,
}

/// One request as stored, for reading rather than running.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestView {
    pub source: Source,
    pub path: String,
    /// The environment variable sources were read for; `None`: globals and collection.
    pub environment: Option<String>,
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub query: Vec<Item>,
    pub headers: Vec<Item>,
    pub cookies: Vec<Item>,
    pub auth: AuthView,
    pub body: BodyView,
    /// The settings this request sets itself; the rest come from the application.
    pub options: serde_json::Map<String, serde_json::Value>,
    pub checks: Vec<CheckView>,
    pub captures: Vec<Capture>,
    pub scripts: Scripts,
    /// The levels whose scripts also run, outer first.
    pub inherited_scripts: Vec<LevelView>,
    pub variables: Vec<VariableView>,
}

fn auth(template: &RequestTemplate) -> AuthView {
    match template.auth.active() {
        Ok(Some(Auth::Basic(c))) => AuthView::Basic {
            username: c.username.clone(),
            password: shown(&c.password),
        },
        Ok(Some(Auth::Digest(c))) => AuthView::Digest {
            username: c.username.clone(),
            password: shown(&c.password),
        },
        Ok(Some(Auth::Bearer(t))) => AuthView::Bearer {
            token: shown(&t.token),
        },
        Ok(None) | Err(_) => AuthView::None,
    }
}

fn body(template: &RequestTemplate, max: usize) -> BodyView {
    let bytes = template.body.len();
    let text = std::str::from_utf8(&template.body).ok().map(|text| {
        let mut end = text.len().min(max);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_owned()
    });
    BodyView {
        kind: template.body_type,
        cut: text.as_ref().is_some_and(|t| t.len() < bytes),
        text,
        bytes,
    }
}

fn options(template: &RequestTemplate) -> serde_json::Map<String, serde_json::Value> {
    let mut map = match serde_json::to_value(&template.options) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.retain(|_, value| !value.is_null());
    if let Some(password) = &template.options.password {
        map.insert("password".into(), shown(password).into());
    }
    map
}

fn label(source: &Defined) -> String {
    match source {
        Defined::Captured => "captured collection (deleted)".to_owned(),
        Defined::Secret { .. } => format!("{} secrets", source.label()),
        other => other.label().to_owned(),
    }
}

fn view(
    source: Source,
    path: &RequestPath,
    environment: Option<&str>,
    template: &RequestTemplate,
    inputs: &Inputs,
    max_body: usize,
) -> RequestView {
    let variables = variables::used(template)
        .into_iter()
        .map(|name| {
            let definitions = inputs.snapshot.definitions.get(&name);
            let (defined_in, overrides) = match definitions.and_then(|d| d.split_first()) {
                Some((first, rest)) => (
                    label(&first.source),
                    rest.iter().map(|d| label(&d.source)).collect(),
                ),
                None if imposter_core::generate::is_generated(&name) => {
                    ("generated".to_owned(), Vec::new())
                }
                None => ("undefined".to_owned(), Vec::new()),
            };
            VariableView {
                name,
                defined_in,
                overrides,
            }
        })
        .collect();
    let secret_header = |name: &str| SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str());
    RequestView {
        source,
        path: path.to_string(),
        environment: environment.map(str::to_owned),
        method: template.method.clone(),
        url: template.url.clone(),
        description: template.description.clone(),
        query: items(&template.queries, |_| false),
        headers: items(&template.headers, secret_header),
        cookies: items(&template.cookies, |_| true),
        auth: auth(template),
        body: body(template, max_body),
        options: options(template),
        checks: template
            .checks
            .iter()
            .map(|check| CheckView {
                describe: check.describe(),
                check: check.clone(),
            })
            .collect(),
        captures: template.captures.clone(),
        scripts: template.scripts.clone(),
        inherited_scripts: inputs
            .scripts
            .0
            .iter()
            .filter(|level| !level.scripts.is_empty())
            .map(|level| LevelView {
                level: level.name.clone(),
                scripts: level.scripts.clone(),
            })
            .collect(),
        variables,
    }
}

/// The request at `path` in `document`, with variable sources for `environment`.
pub fn show_file(
    document: &NativeDocument,
    path: &RequestPath,
    environment: Option<&str>,
    max_body: usize,
) -> Result<RequestView, DocumentError> {
    let selected = document::select(document, path, environment, &Variables::new())?;
    let mut place = vec![document.collection.name.as_str()];
    let parts: Vec<&str> = selected_parts(&selected.path, document.collection.name.as_str());
    place.extend(&parts[..parts.len().saturating_sub(1)]);
    let full = RequestPath::new(&place, parts.last().copied().unwrap_or_default());
    Ok(view(
        Source::File,
        &full,
        environment,
        &selected.request.template,
        &selected.inputs,
        max_body,
    ))
}

/// The parts of `path` after the collection's name, which a file path may leave out.
fn selected_parts<'a>(path: &'a RequestPath, collection: &str) -> Vec<&'a str> {
    let parts: Vec<&str> = path.parts().iter().map(String::as_str).collect();
    match parts.split_first() {
        Some((first, rest)) if *first == collection && !rest.is_empty() => rest.to_vec(),
        _ => parts,
    }
}

/// The saved request at `path`, with variable sources for `environment`. Production
/// environments are allowed: only names are shown.
pub async fn show_profile(
    store: &StoreHandle,
    path: &RequestPath,
    environment: Option<&str>,
    max_body: usize,
) -> Result<RequestView, ProfileError> {
    let selected = profile::select(
        store,
        path,
        environment,
        &Variables::new(),
        Purpose::Inspect,
    )
    .await?;
    Ok(view(
        Source::Profile,
        path,
        environment,
        &selected.request.template,
        &selected.inputs,
        max_body,
    ))
}

/// A variable scope, by its variable names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scope {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<EnvColor>,
    pub variables: Vec<String>,
    /// Its secrets' names (`{{secret:name}}`), never their values; a file has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
}

/// The environments, the globals and each collection's variables, by name only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Environments {
    pub source: Source,
    pub environments: Vec<Scope>,
    pub globals: Vec<String>,
    /// The global secrets' names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub global_secrets: Vec<String>,
    pub collections: Vec<Scope>,
}

fn names(variables: &Variables) -> Vec<String> {
    variables.keys().cloned().collect()
}

pub fn environments_file(document: &NativeDocument) -> Environments {
    Environments {
        source: Source::File,
        environments: document
            .environments
            .iter()
            .map(|e| Scope {
                name: e.name.as_str().to_owned(),
                color: e.color,
                variables: names(&e.variables),
                secrets: Vec::new(),
            })
            .collect(),
        globals: names(&document.globals),
        global_secrets: Vec::new(),
        collections: vec![Scope {
            name: document.collection.name.as_str().to_owned(),
            color: None,
            variables: names(&document.collection.variables),
            secrets: Vec::new(),
        }],
    }
}

pub async fn environments_profile(store: &StoreHandle) -> Result<Environments, StoreError> {
    store
        .call(|db| {
            let secret_names = imposter_store::secrets::names(db)?;
            let secrets_of = |scope: SecretScope| -> Vec<String> {
                secret_names
                    .iter()
                    .filter(|(s, _)| *s == scope)
                    .map(|(_, name)| name.clone())
                    .collect()
            };
            Ok(Environments {
                source: Source::Profile,
                environments: environments::environments(db)?
                    .into_iter()
                    .map(|e| Scope {
                        secrets: secrets_of(SecretScope::Environment(e.id)),
                        name: e.name,
                        color: e.color,
                        variables: names(&e.variables),
                    })
                    .collect(),
                globals: names(&environments::globals(db)?),
                global_secrets: secrets_of(SecretScope::Global),
                collections: collections::collections(db)?
                    .into_iter()
                    .map(|c| Scope {
                        name: c.name,
                        color: None,
                        variables: names(&c.variables),
                        secrets: Vec::new(),
                    })
                    .collect(),
            })
        })
        .await
}
