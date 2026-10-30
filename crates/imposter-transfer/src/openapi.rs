//! OpenAPI 3.0, 3.1 and 3.2, in JSON or YAML: one request per operation, generated once into
//! a new collection. Generating loses nothing a request already did, so every gap is a
//! warning and no consent is needed.

use std::collections::HashSet;

use imposter_core::model::{
    BodyType, FormItem, ParamItem, RequestAuth, RequestTemplate, Variables, is_method_token,
};
use imposter_core::url;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::native::{Name, NativeDocument, TransferCollection, TransferFolder};

/// Path item fields that are operations, in import order (`query` is new in 3.2).
const METHODS: [&str; 9] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
];

/// The collection variable every generated URL starts with.
pub const BASE_URL: &str = "baseUrl";

/// Example bodies stop at this nesting depth and this many values, so a large or deeply
/// nested schema cannot produce a body nobody would send.
const MAX_DEPTH: usize = 10;
const MAX_VALUES: usize = 5_000;

/// `$ref` chains longer than this are treated as broken, which also stops `A -> B -> A`.
const MAX_REF_HOPS: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    #[error("Not valid JSON or YAML: {0}")]
    Yaml(String),
    #[error("Swagger 2.0 files are not supported; convert the file to OpenAPI 3 first")]
    Swagger,
    #[error("Only OpenAPI 3.0, 3.1 and 3.2 are supported, not {0:?}")]
    Version(String),
    #[error("The OpenAPI file has no operations to import")]
    NoOperations,
}

/// Whether `data` declares itself an OpenAPI (or Swagger) document; anything else is left to
/// the other importers.
pub fn is_openapi(data: &Value) -> bool {
    data.get("openapi").is_some() || data.get("swagger").is_some()
}

/// Parse YAML into JSON values. Node and alias limits stop alias bombs, which would otherwise
/// expand a small file into gigabytes; the import size cap already bounds the input.
pub fn parse_yaml(data: &[u8]) -> Result<Value, OpenApiError> {
    let mut budget = serde_saphyr::Budget::default();
    // Without aliases a node takes at least two bytes, so a document may expand to about its
    // own size. The default limits suit config files, not GitHub's or Stripe's specs.
    let nodes = (data.len() / 2).clamp(250_000, 10_000_000);
    budget.max_nodes = nodes;
    budget.max_events = nodes.saturating_mul(2);
    let options = serde_saphyr::options! { budget: Some(budget) };
    serde_saphyr::from_slice_with_options(data, options)
        .map_err(|error| OpenApiError::Yaml(error.to_string()))
}

/// Convert an OpenAPI document into a native document and its warnings.
pub fn decode(data: &Value) -> Result<(NativeDocument, Vec<String>), OpenApiError> {
    if data.get("swagger").is_some() {
        return Err(OpenApiError::Swagger);
    }
    let version = text(data.get("openapi"));
    if !["3.0", "3.1", "3.2"]
        .iter()
        .any(|v| version == *v || version.starts_with(&format!("{v}.")))
    {
        return Err(OpenApiError::Version(version));
    }
    let mut walk = Walk::new(data);
    let base = walk.root_server();
    walk.variables.insert(BASE_URL.to_owned(), base);
    let mut tagged: IndexMap<String, Vec<RequestTemplate>> = IndexMap::new();
    let mut untagged = Vec::new();
    for (path, item) in data
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        for (tag, request) in walk.path_item(path, item) {
            match tag {
                Some(tag) => tagged.entry(tag).or_default().push(request),
                None => untagged.push(request),
            }
        }
    }
    if tagged.is_empty() && untagged.is_empty() {
        return Err(OpenApiError::NoOperations);
    }
    if data.get("webhooks").is_some_and(truthy) {
        walk.warn("Webhooks are not imported".to_owned());
    }
    walk.missing_path_values();
    let info = data.get("info");
    let name = collection_name(info);
    let folders = folders(data, tagged);
    let offset = folders.len();
    let collection = TransferCollection {
        name,
        description: text(info.and_then(|i| i.get("description"))),
        variables: walk.variables,
        folders,
        requests: positioned(untagged, offset),
    };
    Ok((NativeDocument::new(collection), walk.warnings))
}

fn collection_name(info: Option<&Value>) -> Name {
    let title = text(info.and_then(|i| i.get("title")));
    Name::new(title.trim())
        .or_else(|| Name::new("Imported API"))
        .expect("the fallback collection name is not empty")
}

/// Folders in the order the document declares its tags, then undeclared tags as they appear.
fn folders(
    data: &Value,
    mut tagged: IndexMap<String, Vec<RequestTemplate>>,
) -> Vec<TransferFolder> {
    let mut ordered = Vec::new();
    for tag in data
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = text(tag.get("name"));
        if let Some(requests) = tagged.shift_remove(&name) {
            ordered.push((name, text(tag.get("description")), requests));
        }
    }
    ordered.extend(
        tagged
            .into_iter()
            .map(|(name, requests)| (name, String::new(), requests)),
    );
    ordered
        .into_iter()
        .enumerate()
        .filter_map(|(index, (name, description, requests))| {
            Some(TransferFolder {
                name: Name::new(name)?,
                description,
                position: position(index),
                folders: Vec::new(),
                requests: positioned(requests, 0),
            })
        })
        .collect()
}

fn position(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}

fn positioned(mut requests: Vec<RequestTemplate>, offset: usize) -> Vec<RequestTemplate> {
    for (index, request) in requests.iter_mut().enumerate() {
        request.position = position(offset + index);
    }
    requests
}

/// A string, number or boolean as text; anything else is empty.
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Number(_) => true,
    }
}

/// A value as a form field or query value: scalars as text, structures as compact JSON.
fn field_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
        other => text(Some(other)),
    }
}

/// `/pets/{petId}` becomes `/pets/{{petId}}`, and the names it contains.
fn path_template(path: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(path.len() + 8);
    let mut names = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else {
            break;
        };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..close];
        out.push_str("{{");
        out.push_str(name);
        out.push_str("}}");
        names.push(name.to_owned());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    (out, names)
}

/// What kind of body a media type becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Media {
    Json,
    Urlencoded,
    Multipart,
    Text,
    Binary,
}

impl Media {
    fn of(media: &str) -> Self {
        let media = media
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if media == "application/json" || media.ends_with("+json") || media.ends_with("/json") {
            Self::Json
        } else if media == "application/x-www-form-urlencoded" {
            Self::Urlencoded
        } else if media == "multipart/form-data" {
            Self::Multipart
        } else if media.starts_with("text/")
            || media.ends_with("/xml")
            || media.ends_with("+xml")
            || media.ends_with("yaml")
            || media == "application/graphql"
        {
            Self::Text
        } else {
            Self::Binary
        }
    }

    /// Lower is preferred when an operation accepts several media types.
    fn rank(self) -> u8 {
        match self {
            Self::Json => 0,
            Self::Urlencoded => 1,
            Self::Multipart => 2,
            Self::Text => 3,
            Self::Binary => 4,
        }
    }
}

/// How a scheme is used by requests: through the auth slot or as a row.
#[derive(Debug, Clone, PartialEq)]
enum Credential {
    Auth(RequestAuth),
    Header(String, String),
    Query(String, String),
    Cookie(String, String),
}

struct Walk<'a> {
    root: &'a Value,
    warnings: Vec<String>,
    seen: HashSet<String>,
    variables: Variables,
    /// Path parameters with no example, which stay undefined so a send stops on them.
    missing: Vec<String>,
    location: String,
}

impl<'a> Walk<'a> {
    fn new(root: &'a Value) -> Self {
        Self {
            root,
            warnings: Vec::new(),
            seen: HashSet::new(),
            variables: Variables::new(),
            missing: Vec::new(),
            location: String::new(),
        }
    }

    /// Warnings are deduplicated: a scheme or schema used by many operations warns once.
    fn warn(&mut self, message: String) {
        if self.seen.insert(message.clone()) {
            self.warnings.push(message);
        }
    }

    fn define(&mut self, name: &str, value: &str) {
        if !self.variables.contains_key(name) {
            self.variables.insert(name.to_owned(), value.to_owned());
        }
    }

    /// Follow `$ref` to its target. Only references inside the document are followed.
    fn resolve(&mut self, value: &'a Value) -> Option<&'a Value> {
        let mut current = value;
        for _ in 0..MAX_REF_HOPS {
            let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
                return Some(current);
            };
            current = self.lookup(reference)?;
        }
        let location = self.location.clone();
        self.warn(format!(
            "{location}: a reference chain is too long or loops"
        ));
        None
    }

    fn lookup(&mut self, reference: &str) -> Option<&'a Value> {
        let Some(fragment) = reference.strip_prefix('#') else {
            self.warn(format!(
                "External reference {reference} is not followed; its values stay empty"
            ));
            return None;
        };
        let pointer = url::unquote_plus(&fragment.replace('+', "%2B"));
        let found = self.root.pointer(&pointer);
        if found.is_none() {
            self.warn(format!("Reference {reference} does not exist"));
        }
        found
    }

    /// The first root server, with its variables set to their defaults.
    fn root_server(&mut self) -> String {
        let servers = self.root.get("servers").and_then(Value::as_array);
        let Some(first) = servers.and_then(|s| s.first()) else {
            self.warn(format!(
                "No server URL; set the {BASE_URL} collection variable before sending"
            ));
            return String::new();
        };
        let url = server_url(first);
        let others: Vec<String> = servers
            .into_iter()
            .flatten()
            .skip(1)
            .map(server_url)
            .collect();
        if !others.is_empty() {
            self.warn(format!(
                "Using server {url}; to use another ({}), change the {BASE_URL} collection variable",
                others.join(", ")
            ));
        }
        self.relative_server(&url);
        url
    }

    fn relative_server(&mut self, url: &str) {
        if !url.contains("://") {
            self.warn(format!(
                "Server URL {:?} is relative; put the full address in the {BASE_URL} collection variable",
                if url.is_empty() { "/" } else { url }
            ));
        }
    }

    /// Every operation of a path item, with the tag that names its folder.
    fn path_item(&mut self, path: &str, item: &'a Value) -> Vec<(Option<String>, RequestTemplate)> {
        path.clone_into(&mut self.location);
        let Some(item) = self.resolve(item).and_then(Value::as_object) else {
            return Vec::new();
        };
        let mut operations: Vec<(String, &'a Value)> = METHODS
            .iter()
            .filter_map(|method| Some((method.to_ascii_uppercase(), item.get(*method)?)))
            .collect();
        operations.extend(
            item.get("additionalOperations")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(method, operation)| (method.clone(), operation)),
        );
        let shared = item.get("parameters");
        let prefix = self.override_server(item.get("servers"));
        let mut out = Vec::new();
        for (method, operation) in operations {
            self.location = format!("{method} {path}");
            if !is_method_token(&method) {
                let location = self.location.clone();
                self.warn(format!(
                    "{location}: {method:?} is not an HTTP method; skipped"
                ));
                continue;
            }
            let Some(operation) = self.resolve(operation).and_then(Value::as_object) else {
                continue;
            };
            let prefix = self
                .override_server(operation.get("servers"))
                .or_else(|| prefix.clone());
            out.push(self.operation(&method, path, operation, shared, prefix));
        }
        out
    }

    /// A path or operation `servers` override, used literally instead of `{{baseUrl}}`.
    fn override_server(&mut self, servers: Option<&Value>) -> Option<String> {
        let url = server_url(servers?.as_array()?.first()?);
        self.relative_server(&url);
        Some(url)
    }

    fn operation(
        &mut self,
        method: &str,
        path: &str,
        operation: &'a Map<String, Value>,
        shared: Option<&'a Value>,
        prefix: Option<String>,
    ) -> (Option<String>, RequestTemplate) {
        let summary = text(operation.get("summary"));
        let summary = summary.lines().next().unwrap_or_default().trim();
        let name = if summary.is_empty() {
            let id = text(operation.get("operationId"));
            if id.trim().is_empty() {
                self.location.clone()
            } else {
                id.trim().to_owned()
            }
        } else {
            summary.to_owned()
        };
        let mut description = text(operation.get("description"));
        if operation.get("deprecated").is_some_and(truthy) {
            description = if description.is_empty() {
                "Deprecated.".to_owned()
            } else {
                format!("Deprecated.\n\n{description}")
            };
        }
        let (template, names) = path_template(path);
        let prefix = prefix.unwrap_or_else(|| format!("{{{{{BASE_URL}}}}}"));
        let mut request = RequestTemplate {
            name,
            description,
            method: method.to_owned(),
            url: format!("{prefix}{template}"),
            body_type: BodyType::None,
            ..RequestTemplate::default()
        };
        self.parameters(shared, operation.get("parameters"), &mut request);
        for name in names {
            self.path_value(&name, None);
        }
        let security = operation
            .get("security")
            .or_else(|| self.root.get("security"));
        self.security(security, &mut request);
        self.body(operation.get("requestBody"), &mut request);
        if operation.get("callbacks").is_some_and(truthy) {
            let location = self.location.clone();
            self.warn(format!("{location}: callbacks are not imported"));
        }
        let tag = operation
            .get("tags")
            .and_then(Value::as_array)
            .and_then(|tags| tags.first())
            .map(|tag| text(Some(tag)))
            .filter(|tag| !tag.is_empty());
        (tag, request)
    }

    /// Path-level parameters, replaced by operation parameters with the same name and place.
    fn parameters(
        &mut self,
        shared: Option<&'a Value>,
        own: Option<&'a Value>,
        request: &mut RequestTemplate,
    ) {
        let mut merged: IndexMap<(String, String), &'a Map<String, Value>> = IndexMap::new();
        for list in [shared, own] {
            for parameter in list.and_then(Value::as_array).into_iter().flatten() {
                if let Some(parameter) = self.resolve(parameter).and_then(Value::as_object) {
                    let key = (text(parameter.get("name")), text(parameter.get("in")));
                    merged.insert(key, parameter);
                }
            }
        }
        for ((name, place), parameter) in merged {
            if name.is_empty() {
                continue;
            }
            let required = parameter.get("required").is_some_and(truthy);
            let example = self.parameter_example(parameter);
            match place.as_str() {
                "path" => self.path_value(&name, example.as_ref()),
                "query" => request.queries.extend(query_items(
                    &name,
                    parameter,
                    example.as_ref(),
                    required,
                )),
                "header" if is_reserved_header(&name) => {}
                "header" => request.headers.push(ParamItem {
                    is_enabled: required,
                    ..ParamItem::new(name, example.as_ref().map(simple).unwrap_or_default())
                }),
                "cookie" => request.cookies.push(ParamItem {
                    is_enabled: required,
                    ..ParamItem::new(name, example.as_ref().map(simple).unwrap_or_default())
                }),
                other => {
                    let location = self.location.clone();
                    self.warn(format!(
                        "{location}: parameter {name:?} in {other:?} is not imported"
                    ));
                }
            }
        }
    }

    /// Path parameters become collection variables, defined only when the document gives a
    /// value: an empty one would send `DELETE /pets/` instead of stopping on the variable.
    fn path_value(&mut self, name: &str, example: Option<&Value>) {
        match example {
            Some(value) => {
                self.define(name, &simple(value));
                self.missing.retain(|missing| missing != name);
            }
            None if !self.variables.contains_key(name)
                && !self.missing.iter().any(|m| m == name) =>
            {
                self.missing.push(name.to_owned());
            }
            None => {}
        }
    }

    fn missing_path_values(&mut self) {
        if !self.missing.is_empty() {
            let names = self.missing.join(", ");
            self.warn(format!(
                "Path parameters without an example need collection variables before sending: {names}"
            ));
        }
    }

    /// The value the document gives a parameter, if any; types alone give none.
    fn parameter_example(&mut self, parameter: &'a Map<String, Value>) -> Option<Value> {
        if let Some(example) = parameter.get("example") {
            return Some(example.clone());
        }
        if let Some(example) = self.first_example(parameter.get("examples")) {
            return Some(example);
        }
        if let Some(media) = parameter
            .get("content")
            .and_then(Value::as_object)
            .and_then(|content| content.values().next())
        {
            if let Some(example) = media.get("example") {
                return Some(example.clone());
            }
            if let Some(example) = self.first_example(media.get("examples")) {
                return Some(example);
            }
        }
        let schema = self.resolve(parameter.get("schema")?)?;
        schema_example(schema)
    }

    /// The first `value` of an `examples` map; external examples have none.
    fn first_example(&mut self, examples: Option<&'a Value>) -> Option<Value> {
        let example = examples?.as_object()?.values().next()?;
        self.resolve(example)?.get("value").cloned()
    }

    fn security(&mut self, requirement: Option<&'a Value>, request: &mut RequestTemplate) {
        let Some(alternatives) = requirement.and_then(Value::as_array) else {
            return;
        };
        let alternatives: Vec<Vec<String>> = alternatives
            .iter()
            .filter_map(Value::as_object)
            .map(|names| names.keys().cloned().collect())
            .collect();
        // Prefer the first alternative this app can send as is; `{}` means none is required.
        let chosen = alternatives
            .iter()
            .find(|names| names.iter().all(|name| self.native_scheme(name)))
            .or_else(|| alternatives.first());
        let mut slot: Option<String> = None;
        for name in chosen.into_iter().flatten() {
            match self.credential(name) {
                Some(Credential::Auth(auth)) => {
                    if let Some(first) = &slot {
                        let location = self.location.clone();
                        self.warn(format!(
                            "{location}: only one authentication scheme is used ({first}, not {name})"
                        ));
                    } else {
                        request.auth = auth;
                        slot = Some(name.clone());
                    }
                }
                Some(Credential::Header(header, value)) => {
                    request.headers.push(ParamItem::new(header, value));
                }
                Some(Credential::Query(query, value)) => {
                    request.queries.push(ParamItem::new(query, value));
                }
                Some(Credential::Cookie(cookie, value)) => {
                    request.cookies.push(ParamItem::new(cookie, value));
                }
                None => {}
            }
        }
    }

    fn scheme(&mut self, name: &str) -> Option<&'a Map<String, Value>> {
        let scheme = self
            .root
            .get("components")
            .and_then(|c| c.get("securitySchemes"))
            .and_then(|s| s.get(name));
        let Some(scheme) = scheme else {
            self.warn(format!("Security scheme {name:?} is not defined"));
            return None;
        };
        self.resolve(scheme).and_then(Value::as_object)
    }

    /// Whether a scheme needs nothing this app lacks (OAuth flows, client certificates).
    fn native_scheme(&mut self, name: &str) -> bool {
        let Some(scheme) = self.scheme(name) else {
            return false;
        };
        match text(scheme.get("type")).as_str() {
            "http" => matches!(
                text(scheme.get("scheme")).to_ascii_lowercase().as_str(),
                "basic" | "bearer" | "digest"
            ),
            "apiKey" => matches!(
                text(scheme.get("in")).as_str(),
                "header" | "query" | "cookie"
            ),
            _ => false,
        }
    }

    /// What a security scheme adds to a request, with collection variables for its secrets.
    fn credential(&mut self, name: &str) -> Option<Credential> {
        let scheme = self.scheme(name)?;
        let kind = text(scheme.get("type"));
        match kind.as_str() {
            "http" => {
                let method = text(scheme.get("scheme")).to_ascii_lowercase();
                match method.as_str() {
                    "basic" | "digest" => {
                        self.define("username", "");
                        self.define("password", "");
                        Some(Credential::Auth(if method == "basic" {
                            RequestAuth::basic("{{username}}", "{{password}}")
                        } else {
                            RequestAuth::digest("{{username}}", "{{password}}")
                        }))
                    }
                    "bearer" => Some(self.bearer()),
                    other => {
                        self.warn(format!(
                            "Security scheme {name:?}: HTTP {other} authentication is not supported"
                        ));
                        None
                    }
                }
            }
            "apiKey" => {
                let key = text(scheme.get("name"));
                self.define(name, "");
                let value = format!("{{{{{name}}}}}");
                match text(scheme.get("in")).as_str() {
                    "header" => Some(Credential::Header(key, value)),
                    "query" => Some(Credential::Query(key, value)),
                    "cookie" => Some(Credential::Cookie(key, value)),
                    other => {
                        self.warn(format!(
                            "Security scheme {name:?}: API keys in {other:?} are not supported"
                        ));
                        None
                    }
                }
            }
            "oauth2" | "openIdConnect" => {
                self.warn(format!(
                    "Security scheme {name:?}: {kind} flows are not supported; requests send the token collection variable as a bearer token"
                ));
                Some(self.bearer())
            }
            "mutualTLS" => {
                self.warn(format!(
                    "Security scheme {name:?}: set the client certificate in Settings"
                ));
                None
            }
            other => {
                self.warn(format!(
                    "Security scheme {name:?}: type {other:?} is not supported"
                ));
                None
            }
        }
    }

    fn bearer(&mut self) -> Credential {
        self.define("token", "");
        Credential::Auth(RequestAuth::bearer("{{token}}"))
    }

    fn body(&mut self, body: Option<&'a Value>, request: &mut RequestTemplate) {
        let Some(content) = body
            .and_then(|body| self.resolve(body))
            .and_then(|body| body.get("content"))
            .and_then(Value::as_object)
        else {
            return;
        };
        let Some((media_type, entry)) = content
            .iter()
            .min_by_key(|(media, _)| Media::of(media).rank())
        else {
            return;
        };
        let media = Media::of(media_type);
        let specific = !matches!(
            media_type.as_str(),
            "application/json" | "text/plain" | "application/octet-stream" | "*/*"
        );
        let content_type_needed = match media {
            Media::Json | Media::Text | Media::Binary => specific,
            Media::Urlencoded | Media::Multipart => false,
        };
        if content_type_needed {
            request
                .headers
                .push(ParamItem::new("Content-Type", media_type.clone()));
        }
        let schema = entry.get("schema");
        let example = self.media_example(entry);
        match media {
            Media::Json => {
                request.body_type = BodyType::Json;
                let value = example.or_else(|| schema.and_then(|s| self.sample_body(s)));
                request.body = value
                    .map(|v| json_text(&v))
                    .unwrap_or_default()
                    .into_bytes();
            }
            Media::Text => {
                request.body_type = BodyType::Plain;
                if let Some(Value::String(text)) = example {
                    request.body = text.into_bytes();
                }
            }
            Media::Binary => request.body_type = BodyType::Binary,
            // A form with no fields is how some specs (Stripe's GETs) say "no body"; sending
            // it would add a pointless Content-Type.
            Media::Urlencoded => {
                let value = example.or_else(|| schema.and_then(|s| self.sample_body(s)));
                let fields: Vec<ParamItem> = object_entries(value.as_ref())
                    .map(|(name, value)| ParamItem::new(name.clone(), field_text(value)))
                    .collect();
                if !fields.is_empty() {
                    request.body_type = BodyType::Urlencoded;
                    request.set_form_fields(&fields);
                }
            }
            Media::Multipart => {
                let value = example.or_else(|| schema.and_then(|s| self.sample_body(s)));
                let files = schema
                    .map(|s| self.file_fields(s, entry))
                    .unwrap_or_default();
                let fields: Vec<FormItem> = object_entries(value.as_ref())
                    .map(|(name, value)| {
                        if files.contains(name) {
                            FormItem::file(name.clone(), "")
                        } else {
                            FormItem::text(name.clone(), field_text(value))
                        }
                    })
                    .collect();
                if !fields.is_empty() {
                    request.body_type = BodyType::Multipart;
                    request.set_multipart_fields(&fields);
                }
            }
        }
    }

    fn media_example(&mut self, entry: &'a Value) -> Option<Value> {
        if let Some(example) = entry.get("example") {
            return Some(example.clone());
        }
        self.first_example(entry.get("examples"))
    }

    /// Multipart fields sent as files: binary strings, or parts with a non-text encoding.
    fn file_fields(&mut self, schema: &'a Value, entry: &'a Value) -> HashSet<String> {
        let mut files = HashSet::new();
        for (name, property) in self.properties(schema, 0) {
            if self.is_file(property) {
                files.insert(name);
            }
        }
        for (name, encoding) in entry
            .get("encoding")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let content_type = text(encoding.get("contentType"));
            if !content_type.is_empty()
                && !content_type.starts_with("text/")
                && Media::of(&content_type) == Media::Binary
            {
                files.insert(name.clone());
            }
        }
        files
    }

    fn is_file(&mut self, schema: &'a Value) -> bool {
        let Some(schema) = self.resolve(schema) else {
            return false;
        };
        if let Some(items) = schema.get("items") {
            return self.is_file(items);
        }
        text(schema.get("format")) == "binary"
            || (schema.get("contentMediaType").is_some() && schema.get("contentEncoding").is_none())
    }

    /// An object schema's properties, with `allOf` parts merged in.
    fn properties(&mut self, schema: &'a Value, depth: usize) -> Vec<(String, &'a Value)> {
        let Some(schema) = self.resolve(schema) else {
            return Vec::new();
        };
        let mut out: Vec<(String, &'a Value)> = Vec::new();
        if depth < MAX_DEPTH {
            for part in schema
                .get("allOf")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.extend(self.properties(part, depth + 1));
            }
        }
        for (name, property) in schema
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push((name.clone(), property));
        }
        out
    }

    fn sample_body(&mut self, schema: &'a Value) -> Option<Value> {
        let mut sampler = Sampler {
            stack: Vec::new(),
            budget: MAX_VALUES,
            cut: false,
        };
        let value = sampler.sample(self, schema, 0);
        if sampler.cut {
            let location = self.location.clone();
            self.warn(format!(
                "{location}: the example body was cut short; the schema is too large or deep"
            ));
        }
        value
    }
}

/// Generates an example value from a schema: the document's own examples first, then
/// placeholders by type. Recursive schemas are included once.
struct Sampler {
    stack: Vec<String>,
    budget: usize,
    cut: bool,
}

impl Sampler {
    fn sample<'a>(
        &mut self,
        walk: &mut Walk<'a>,
        schema: &'a Value,
        depth: usize,
    ) -> Option<Value> {
        if depth > MAX_DEPTH || self.budget == 0 {
            self.cut = true;
            return None;
        }
        self.budget -= 1;
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if self.stack.iter().any(|r| r == reference) {
                walk.warn(format!(
                    "Schema {reference} is recursive; example bodies include it once"
                ));
                return None;
            }
            let target = walk.lookup(reference)?;
            self.stack.push(reference.to_owned());
            let value = self.sample(walk, target, depth);
            self.stack.pop();
            return value;
        }
        if let Some(example) = schema_example(schema) {
            return Some(example);
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(first) = schema
                .get(key)
                .and_then(Value::as_array)
                .and_then(|options| options.first())
            {
                return self.sample(walk, first, depth);
            }
        }
        if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
            return Some(self.all_of(walk, schema, parts, depth));
        }
        Some(match schema_type(schema).as_str() {
            "object" => Value::Object(self.object(walk, schema, depth)),
            "array" => Value::Array(
                schema
                    .get("items")
                    .and_then(|items| self.sample(walk, items, depth + 1))
                    .into_iter()
                    .collect(),
            ),
            "string" => Value::String(string_placeholder(&text(schema.get("format")))),
            "integer" | "number" => schema
                .get("minimum")
                .filter(|m| m.is_number())
                .cloned()
                .unwrap_or_else(|| Value::from(0)),
            "boolean" => Value::Bool(true),
            _ => Value::Null,
        })
    }

    fn all_of<'a>(
        &mut self,
        walk: &mut Walk<'a>,
        schema: &'a Value,
        parts: &'a [Value],
        depth: usize,
    ) -> Value {
        let mut merged = Map::new();
        let mut other = None;
        for part in parts {
            match self.sample(walk, part, depth) {
                Some(Value::Object(map)) => merged.extend(map),
                Some(value) => other = Some(value),
                None => {}
            }
        }
        merged.extend(self.object(walk, schema, depth));
        match other {
            Some(value) if merged.is_empty() => value,
            _ => Value::Object(merged),
        }
    }

    /// Properties a client sends: `readOnly` ones are the server's.
    fn object<'a>(
        &mut self,
        walk: &mut Walk<'a>,
        schema: &'a Value,
        depth: usize,
    ) -> Map<String, Value> {
        let mut map = Map::new();
        for (name, property) in schema
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let read_only = |s: &Value| s.get("readOnly").is_some_and(truthy);
            if read_only(property) || walk.resolve(property).is_some_and(read_only) {
                continue;
            }
            if let Some(value) = self.sample(walk, property, depth + 1) {
                map.insert(name.clone(), value);
            }
        }
        map
    }
}

/// The example a schema itself gives, if any.
fn schema_example(schema: &Value) -> Option<Value> {
    if let Some(example) = schema.get("example") {
        return Some(example.clone());
    }
    if let Some(first) = schema
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return Some(first.clone());
    }
    for key in ["default", "const"] {
        if let Some(value) = schema.get(key) {
            return Some(value.clone());
        }
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .cloned()
}

/// The schema's type; 3.1 allows a list (`["string", "null"]`), and the type may be implied.
fn schema_type(schema: &Value) -> String {
    match schema.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|kind| *kind != "null")
            .unwrap_or("null")
            .to_owned(),
        _ if schema.get("properties").is_some() => "object".to_owned(),
        _ if schema.get("items").is_some() => "array".to_owned(),
        _ => String::new(),
    }
}

fn string_placeholder(format: &str) -> String {
    match format {
        "date-time" => "2024-01-01T00:00:00Z",
        "date" => "2024-01-01",
        "time" => "00:00:00",
        "email" => "user@example.com",
        "uuid" => "00000000-0000-0000-0000-000000000000",
        "uri" | "url" => "https://example.com",
        "hostname" => "example.com",
        "ipv4" => "192.0.2.1",
        "ipv6" => "2001:db8::1",
        "binary" | "byte" => "",
        _ => "string",
    }
    .to_owned()
}

/// JSON bodies are pretty-printed; a string example that already holds JSON is kept as typed.
fn json_text(value: &Value) -> String {
    if let Value::String(raw) = value
        && serde_json::from_str::<Value>(raw).is_ok()
    {
        return raw.clone();
    }
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn object_entries(value: Option<&Value>) -> impl Iterator<Item = (&String, &Value)> {
    value.and_then(Value::as_object).into_iter().flatten()
}

fn server_url(server: &Value) -> String {
    let mut url = text(server.get("url"));
    for (name, variable) in server
        .get("variables")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        url = url.replace(&format!("{{{name}}}"), &text(variable.get("default")));
    }
    url.trim_end_matches('/').to_owned()
}

/// OpenAPI ignores these as parameters: they come from the body, `Accept` negotiation and
/// security schemes.
fn is_reserved_header(name: &str) -> bool {
    ["accept", "content-type", "authorization"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// `simple` style (path and header values): arrays and objects comma-separated.
fn simple(value: &Value) -> String {
    match value {
        Value::Array(items) => items.iter().map(field_text).collect::<Vec<_>>().join(","),
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| [key.clone(), field_text(value)])
            .collect::<Vec<_>>()
            .join(","),
        other => field_text(other),
    }
}

/// Query rows for a parameter, serialized by its `style` and `explode` (form, exploded, by
/// default). Optional parameters are imported disabled so they change nothing until chosen.
fn query_items(
    name: &str,
    parameter: &Map<String, Value>,
    example: Option<&Value>,
    required: bool,
) -> Vec<ParamItem> {
    let style = text(parameter.get("style"));
    let explode = parameter
        .get("explode")
        .and_then(Value::as_bool)
        .unwrap_or(style.is_empty() || style == "form");
    let row = |name: String, value: String| ParamItem {
        is_enabled: required,
        ..ParamItem::new(name, value)
    };
    match example {
        Some(Value::Array(items)) if explode => items
            .iter()
            .map(|item| row(name.to_owned(), field_text(item)))
            .collect(),
        Some(Value::Array(items)) => {
            let separator = match style.as_str() {
                "spaceDelimited" => " ",
                "pipeDelimited" => "|",
                _ => ",",
            };
            let joined = items
                .iter()
                .map(field_text)
                .collect::<Vec<_>>()
                .join(separator);
            vec![row(name.to_owned(), joined)]
        }
        Some(Value::Object(map)) if style == "deepObject" => map
            .iter()
            .map(|(key, value)| row(format!("{name}[{key}]"), field_text(value)))
            .collect(),
        Some(Value::Object(map)) if explode => map
            .iter()
            .map(|(key, value)| row(key.clone(), field_text(value)))
            .collect(),
        Some(other) => vec![row(name.to_owned(), simple(other))],
        None => vec![row(name.to_owned(), String::new())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_templates_become_variables() {
        assert_eq!(
            path_template("/a/{x}/b/{y-z}.json"),
            (
                "/a/{{x}}/b/{{y-z}}.json".to_owned(),
                vec!["x".to_owned(), "y-z".to_owned()]
            )
        );
        assert_eq!(path_template("/open{"), ("/open{".to_owned(), Vec::new()));
    }

    #[test]
    fn media_types_map_to_body_kinds() {
        for (media, kind) in [
            ("application/json; charset=utf-8", Media::Json),
            ("application/vnd.api+json", Media::Json),
            ("application/x-www-form-urlencoded", Media::Urlencoded),
            ("multipart/form-data", Media::Multipart),
            ("application/xml", Media::Text),
            ("text/csv", Media::Text),
            ("application/x-yaml", Media::Text),
            ("image/png", Media::Binary),
            ("*/*", Media::Binary),
        ] {
            assert_eq!(Media::of(media), kind, "{media}");
        }
    }

    #[test]
    fn string_placeholders_follow_the_format() {
        assert_eq!(string_placeholder("email"), "user@example.com");
        assert_eq!(string_placeholder("binary"), "");
        assert_eq!(string_placeholder("whatever"), "string");
    }
}
