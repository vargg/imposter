//! Postman Collection v2.1: a supported subset of the native format. Every conversion loss is
//! reported; losses that change what a request sends require explicit consent before import.
//! Behavior follows the Python app (`tests/fixtures/parity/postman.json`).

use std::path::Path;
use std::sync::LazyLock;

use poster_core::model::{
    BodyType, FormItem, FormKind, ParamItem, RequestAuth, RequestTemplate, Variables,
};
use poster_core::url;
use serde_json::{Map, Value, json};

use crate::native::{
    Name, NativeDocument, TransferCollection, TransferEnvironment, TransferFolder, atomic_write,
};

pub const SCHEMA_URL: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// The official v2.1 schema, pinned (see `schemas/README.md`) and validated offline.
pub const SCHEMA: &str = include_str!("../schemas/postman-v2.1.json");

static VALIDATOR: LazyLock<Result<jsonschema::Validator, String>> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(SCHEMA).map_err(|e| e.to_string())?;
    jsonschema::draft4::new(&schema).map_err(|e| e.to_string())
});

#[derive(Debug, thiserror::Error)]
pub enum PostmanError {
    #[error("Not a valid Postman collection: {0}")]
    Schema(String),
    #[error("Only Postman Collection v2.1 is supported")]
    Version,
    #[error("Companion must contain a values array")]
    CompanionValues,
    #[error("Companion must be an environment or globals file")]
    CompanionScope,
    #[error("Collection names must not be empty")]
    EmptyName,
    #[error("Missing authentication credentials")]
    Credentials,
    #[error("Choose a new destination directory for companion exports")]
    DestinationExists,
    #[error("Cannot write the export: {0}")]
    Io(#[from] std::io::Error),
}

pub fn validate(data: &Value) -> Result<(), PostmanError> {
    let validator = VALIDATOR
        .as_ref()
        .map_err(|e| PostmanError::Schema(e.clone()))?;
    validator
        .validate(data)
        .map_err(|error| PostmanError::Schema(error.to_string()))
}

/// Python's `json.dumps(value, ensure_ascii=False)`: `", "` and `": "` separators.
fn py_dumps(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(py_dumps).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}: {}", Value::String(k.clone()), py_dumps(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// Python's `repr()` of a string, as warnings show names.
fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// A JSON value as text: strings as they are, `null` as empty, anything else as JSON.
fn literal(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => py_dumps(other),
    }
}

fn description(value: Option<&Value>) -> String {
    match value {
        Some(Value::Object(map)) => {
            literal(map.get("content").or(Some(&Value::String(String::new()))))
        }
        other => literal(other),
    }
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Remove `{{ $dynamic }}` placeholders (Python `\{\{\s*\$.*?\}\}`); returns whether any was found.
fn strip_dynamic(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut found = false;
    let mut last = 0;
    let mut from = 0;
    while let Some(offset) = text[from..].find("{{") {
        let start = from + offset;
        let after_braces = &text[start + 2..];
        let dollar = after_braces.len() - after_braces.trim_start().len();
        let body = start + 2 + dollar;
        let matched = text[body..].starts_with('$').then(|| {
            let rest = &text[body + 1..];
            let close = rest.find("}}")?;
            (!rest[..close].contains('\n')).then_some(body + 1 + close + 2)
        });
        match matched.flatten() {
            Some(end) => {
                out.push_str(&text[last..start]);
                last = end;
                from = end;
                found = true;
            }
            None => from = start + 1,
        }
    }
    found.then(|| {
        out.push_str(&text[last..]);
        out
    })
}

fn object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Collects compatibility losses while converting.
#[derive(Debug, Default)]
pub struct Decoder {
    pub warnings: Vec<String>,
    pub requires_removal: bool,
}

impl Decoder {
    fn loss(&mut self, location: &str, feature: &str, affects_execution: bool) {
        self.warnings
            .push(format!("{location}: {feature} will be removed"));
        self.requires_removal |= affects_execution;
    }

    fn clean(&mut self, value: &Value, location: &str) -> Value {
        match value {
            Value::String(text) => match strip_dynamic(text) {
                Some(cleaned) => {
                    self.loss(location, "Postman dynamic variables", true);
                    Value::String(cleaned)
                }
                None => value.clone(),
            },
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.clean(item, location))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.clean(v, location)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn extras(&mut self, node: &Map<String, Value>, location: &str) {
        for (key, label) in [
            ("event", "scripts"),
            ("protocolProfileBehavior", "protocol settings"),
            ("proxy", "proxy configuration"),
            ("certificate", "certificate configuration"),
            ("disabled", "disabled request/folder state"),
        ] {
            if truthy(node.get(key)) {
                self.loss(location, label, true);
            }
        }
        if truthy(node.get("response")) {
            self.loss(location, "saved response examples", false);
        }
    }

    fn variables(&mut self, items: &[Value], location: &str) -> Variables {
        let mut values = Variables::new();
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            let key = literal(item.get("key").or_else(|| item.get("id")));
            if key.is_empty() {
                self.loss(location, "variable without a key", true);
            } else if truthy(item.get("disabled"))
                || item.get("enabled") == Some(&Value::Bool(false))
            {
                self.loss(
                    location,
                    &format!("disabled variable {}", py_repr(&key)),
                    false,
                );
            } else {
                if values.contains_key(&key) {
                    self.loss(
                        location,
                        &format!("earlier duplicate variable {}", py_repr(&key)),
                        true,
                    );
                }
                let value = self.clean(&Value::String(literal(item.get("value"))), location);
                values.insert(key, literal(Some(&value)));
            }
        }
        values
    }

    fn auth(&mut self, data: Option<&Value>, location: &str) -> RequestAuth {
        let Some(data) = object(data).filter(|d| !d.is_empty()) else {
            return RequestAuth::default();
        };
        let kind = data.get("type");
        if kind.and_then(Value::as_str) == Some("noauth") {
            return RequestAuth::default();
        }
        let kind_text = match kind {
            Some(Value::String(text)) => text.clone(),
            None | Some(Value::Null) => "None".to_owned(),
            Some(other) => py_dumps(other),
        };
        if !matches!(kind_text.as_str(), "basic" | "digest" | "bearer") {
            self.loss(location, &format!("{kind_text} authentication"), true);
            return RequestAuth::default();
        }
        let mut attributes: Vec<(String, String)> = Vec::new();
        for item in array(data.get(&kind_text)) {
            if let Some(key) = item.get("key").and_then(Value::as_str) {
                let value = literal(item.get("value"));
                match attributes.iter_mut().find(|(k, _)| k == key) {
                    Some(existing) => existing.1 = value,
                    None => attributes.push((key.to_owned(), value)),
                }
            }
        }
        let supported: &[&str] = if kind_text == "bearer" {
            &["token"]
        } else {
            &["username", "password"]
        };
        if attributes
            .iter()
            .any(|(key, value)| !supported.contains(&key.as_str()) && !value.is_empty())
        {
            self.loss(location, "additional authentication parameters", true);
        }
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        match kind_text.as_str() {
            "bearer" => RequestAuth::bearer(get("token")),
            "basic" => RequestAuth::basic(get("username"), get("password")),
            _ => RequestAuth::digest(get("username"), get("password")),
        }
    }

    fn pairs(&mut self, items: &[Value], location: &str) -> Vec<ParamItem> {
        let mut result = Vec::new();
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            if truthy(item.get("description")) {
                self.loss(location, "parameter/header descriptions", false);
            }
            result.push(ParamItem {
                name: literal(item.get("key")),
                value: literal(item.get("value")),
                is_enabled: !truthy(item.get("disabled")),
            });
        }
        result
    }

    fn url(&mut self, value: &Value, location: &str) -> (String, Vec<ParamItem>) {
        let (mut raw, mut pairs) = (String::new(), None);
        match value {
            Value::String(text) => raw.clone_from(text),
            Value::Object(map) => {
                raw = literal(map.get("raw"));
                if raw.is_empty() {
                    let host = match map.get("host") {
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .map(|p| literal(Some(p)))
                            .collect::<Vec<_>>()
                            .join("."),
                        other => literal(other),
                    };
                    let path = match map.get("path") {
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .map(|p| match p {
                                Value::String(s) => s.clone(),
                                other => literal(
                                    other.get("value").or(Some(&Value::String(String::new()))),
                                ),
                            })
                            .collect::<Vec<_>>()
                            .join("/"),
                        other => literal(other),
                    };
                    let protocol = map
                        .get("protocol")
                        .map_or_else(|| "https".to_owned(), |p| literal(Some(p)));
                    raw = format!("{protocol}://{host}");
                    if truthy(map.get("port")) {
                        raw.push(':');
                        raw.push_str(&literal(map.get("port")));
                    }
                    raw.push('/');
                    raw.push_str(&path);
                }
                for variable in array(map.get("variable")) {
                    let key = literal(variable.get("key").or_else(|| variable.get("id")));
                    if !key.is_empty() && !truthy(variable.get("disabled")) {
                        raw = replace_path_variable(&raw, &key, &literal(variable.get("value")));
                    } else {
                        self.loss(location, "disabled URL variable", true);
                    }
                }
                if map.contains_key("query") {
                    pairs = Some(self.pairs(array(map.get("query")), location));
                }
            }
            other => raw = literal(Some(other)),
        }
        let parts = url::split(&raw);
        let pairs = pairs.unwrap_or_else(|| {
            url::parse_pairs(&parts.query)
                .into_iter()
                .map(|(name, value)| ParamItem::new(name, value))
                .collect()
        });
        let base = url::join(&url::UrlParts {
            query: String::new(),
            fragment: String::new(),
            ..parts
        });
        (base, pairs)
    }

    fn body(&mut self, data: Option<&Value>, request: &mut RequestTemplate, location: &str) {
        let Some(data) = object(data).filter(|d| !d.is_empty()) else {
            request.body_type = BodyType::None;
            return;
        };
        if truthy(data.get("disabled")) {
            request.body_type = BodyType::None;
            return;
        }
        let mode = data.get("mode");
        match mode.and_then(Value::as_str) {
            Some("raw") => {
                let language = data
                    .get("options")
                    .and_then(|o| o.get("raw"))
                    .and_then(|r| r.get("language"))
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                request.body_type = if language == "json" {
                    BodyType::Json
                } else {
                    BodyType::Plain
                };
                request.body = literal(data.get("raw")).into_bytes();
            }
            Some("urlencoded") => {
                request.body_type = BodyType::Urlencoded;
                let fields = self.pairs(array(data.get("urlencoded")), location);
                request.set_form_fields(&fields);
            }
            Some("formdata") => {
                request.body_type = BodyType::Multipart;
                let mut fields = Vec::new();
                for item in array(data.get("formdata")) {
                    let kind = item.get("type").and_then(Value::as_str).unwrap_or("text");
                    if !matches!(kind, "file" | "text") {
                        self.loss(location, "unsupported multipart field", true);
                        continue;
                    }
                    let file = kind == "file";
                    let empty = Value::String(String::new());
                    let source = if file {
                        item.get("src")
                    } else {
                        item.get("value")
                    }
                    .unwrap_or(&empty);
                    let values: Vec<&Value> = match source {
                        Value::Array(list) => list.iter().collect(),
                        other => vec![other],
                    };
                    for value in values {
                        fields.push(FormItem {
                            name: literal(item.get("key")),
                            value: literal(Some(value)),
                            is_enabled: !truthy(item.get("disabled")),
                            kind: if file { FormKind::File } else { FormKind::Text },
                            content_type: literal(item.get("contentType")),
                            filename: String::new(),
                        });
                    }
                }
                request.set_multipart_fields(&fields);
            }
            Some("file") => {
                request.body_type = BodyType::Binary;
                request.body = literal(data.get("file").and_then(|f| f.get("src"))).into_bytes();
            }
            _ => {
                let mode = match mode {
                    Some(Value::String(text)) => text.clone(),
                    None | Some(Value::Null) => "None".to_owned(),
                    Some(other) => py_dumps(other),
                };
                self.loss(location, &format!("{mode} body"), true);
                request.body_type = BodyType::None;
            }
        }
    }

    fn items(
        &mut self,
        items: &[Value],
        folders: &mut Vec<TransferFolder>,
        requests: &mut Vec<RequestTemplate>,
        inherited_auth: Option<&Value>,
        location: &str,
    ) -> Result<(), PostmanError> {
        for (index, item) in items.iter().enumerate() {
            let Some(item) = item.as_object() else {
                continue;
            };
            let name = match item.get("name") {
                Some(value) if truthy(Some(value)) => literal(Some(value)),
                _ => "Untitled".to_owned(),
            };
            let path = format!("{location}/{name}");
            self.extras(item, &path);
            let position = i64::try_from(index).unwrap_or(i64::MAX);
            if let Some(children) = item.get("item") {
                if truthy(item.get("variable")) {
                    self.loss(&path, "folder variables", true);
                }
                let mut folder = TransferFolder {
                    name: Name::new(name).ok_or(PostmanError::EmptyName)?,
                    description: description(item.get("description")),
                    position,
                    folders: Vec::new(),
                    requests: Vec::new(),
                };
                let auth = item.get("auth").or(inherited_auth);
                self.items(
                    array(Some(children)),
                    &mut folder.folders,
                    &mut folder.requests,
                    auth,
                    &path,
                )?;
                folders.push(folder);
                continue;
            }
            let data = match item.get("request") {
                Some(Value::String(text)) => json!({"url": text, "method": "GET"}),
                Some(other) => other.clone(),
                None => Value::Object(Map::new()),
            };
            let data = self.clean(&data, &path);
            let data = data.as_object().cloned().unwrap_or_default();
            self.extras(&data, &path);
            let (url, queries) = self.url(
                data.get("url").unwrap_or(&Value::String(String::new())),
                &path,
            );
            let headers = match data.get("header") {
                Some(Value::String(text)) => text
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| {
                        let (key, value) = line.split_once(':').unwrap_or((line, ""));
                        json!({"key": key.trim(), "value": value.trim()})
                    })
                    .collect(),
                other => array(other).to_vec(),
            };
            let auth_source = data
                .get("auth")
                .cloned()
                .or_else(|| inherited_auth.cloned());
            let auth_source = auth_source.map(|auth| self.clean(&auth, &path));
            let mut request = RequestTemplate {
                name,
                position,
                description: description(data.get("description")),
                method: data
                    .get("method")
                    .map_or_else(|| "GET".to_owned(), |m| literal(Some(m))),
                url,
                queries,
                headers: self.pairs(&headers, &path),
                auth: self.auth(auth_source.as_ref(), &path),
                ..RequestTemplate::default()
            };
            self.body(data.get("body"), &mut request, &path);
            requests.push(request);
        }
        Ok(())
    }

    /// Validate v2.1 and convert, flattening inherited authentication into each request.
    pub fn decode(&mut self, data: &Value) -> Result<NativeDocument, PostmanError> {
        validate(data)?;
        let info = data
            .get("info")
            .and_then(Value::as_object)
            .ok_or(PostmanError::Version)?;
        if !literal(info.get("schema")).contains("v2.1.0") {
            return Err(PostmanError::Version);
        }
        let variables = self.variables(array(data.get("variable")), "Collection");
        let name = literal(info.get("name"));
        let mut collection = TransferCollection {
            name: Name::new(name.clone()).ok_or(PostmanError::EmptyName)?,
            description: description(info.get("description")),
            variables,
            folders: Vec::new(),
            requests: Vec::new(),
        };
        if let Some(root) = data.as_object() {
            self.extras(root, "Collection");
        }
        self.items(
            array(data.get("item")),
            &mut collection.folders,
            &mut collection.requests,
            data.get("auth"),
            &name,
        )?;
        Ok(NativeDocument::new(collection))
    }

    /// Attach a Postman environment or globals file.
    pub fn companion(
        &mut self,
        data: &Value,
        document: &mut NativeDocument,
    ) -> Result<(), PostmanError> {
        let values = data
            .get("values")
            .and_then(Value::as_array)
            .ok_or(PostmanError::CompanionValues)?;
        if !values.iter().all(Value::is_object) {
            return Err(PostmanError::CompanionValues);
        }
        let variables = self.variables(values, "Companion");
        match data.get("_postman_variable_scope").and_then(Value::as_str) {
            Some("globals") => document.globals.extend(variables),
            None | Some("environment") => {
                let name = match data.get("name") {
                    Some(value) if truthy(Some(value)) => literal(Some(value)),
                    _ => "Imported environment".to_owned(),
                };
                document.environments.push(TransferEnvironment {
                    name: Name::new(name).ok_or(PostmanError::EmptyName)?,
                    variables,
                });
            }
            Some(_) => return Err(PostmanError::CompanionScope),
        }
        Ok(())
    }
}

/// Replace `:name` path variables followed by `/`, `?`, `#` or the end.
fn replace_path_variable(raw: &str, key: &str, value: &str) -> String {
    let needle = format!(":{key}");
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find(&needle) {
        let after = &rest[index + needle.len()..];
        let boundary = after.is_empty() || after.starts_with(['/', '?', '#']) || after == "\n";
        out.push_str(&rest[..index]);
        out.push_str(if boundary { value } else { &needle });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn export_pairs(items: &[ParamItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| json!({"key": i.name, "value": i.value, "disabled": !i.is_enabled}))
            .collect(),
    )
}

fn export_auth(auth: &RequestAuth) -> Result<Value, PostmanError> {
    use poster_core::model::AuthKind;
    Ok(match auth.kind {
        None => json!({"type": "noauth"}),
        Some(AuthKind::BearerToken) => {
            let token = auth
                .bearer_token
                .as_ref()
                .ok_or(PostmanError::Credentials)?;
            json!({"type": "bearer", "bearer": [{"key": "token", "value": token.token, "type": "string"}]})
        }
        Some(kind) => {
            let (label, credentials) = match kind {
                AuthKind::Basic => ("basic", auth.basic.as_ref()),
                _ => ("digest", auth.digest.as_ref()),
            };
            let credentials = credentials.ok_or(PostmanError::Credentials)?;
            json!({"type": label, label: [
                {"key": "username", "value": credentials.username, "type": "string"},
                {"key": "password", "value": credentials.password, "type": "string"},
            ]})
        }
    })
}

fn export_request(
    request: &RequestTemplate,
    warnings: &mut Vec<String>,
) -> Result<Value, PostmanError> {
    let mut headers = export_pairs(&request.headers);
    if !request.cookies.is_empty() {
        let cookie = request
            .cookies
            .iter()
            .filter(|c| c.is_enabled)
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookie.is_empty()
            && let Some(list) = headers.as_array_mut()
        {
            list.push(json!({"key": "Cookie", "value": cookie}));
        }
        warnings.push(format!(
            "{}: cookies converted to a Cookie header; disabled cookie entries omitted",
            request.name
        ));
    }
    if !request.options.is_empty() {
        warnings.push(format!(
            "{}: per-request settings cannot be represented in Postman and are omitted",
            request.name
        ));
    }
    let parts = url::split(&request.url);
    let queries: Vec<ParamItem> = if request.queries.is_empty() {
        url::parse_pairs(&parts.query)
            .into_iter()
            .map(|(n, v)| ParamItem::new(n, v))
            .collect()
    } else {
        request.queries.clone()
    };
    let query = url::encode_pairs(
        queries
            .iter()
            .filter(|q| q.is_enabled)
            .map(|q| (q.name.as_str(), q.value.as_str())),
    );
    let raw = url::join_with_fragment(&url::UrlParts { query, ..parts });
    let mut data = json!({
        "method": request.method,
        "description": request.description,
        "url": {"raw": raw, "query": export_pairs(&queries)},
        "header": headers,
        "auth": export_auth(&request.auth)?,
    });
    let text = || String::from_utf8_lossy(&request.body).into_owned();
    let body = match request.body_type {
        BodyType::Json | BodyType::Plain | BodyType::Text => {
            let language = if request.body_type == BodyType::Json {
                "json"
            } else {
                "text"
            };
            Some(json!({"mode": "raw", "raw": text(), "options": {"raw": {"language": language}}}))
        }
        BodyType::Urlencoded | BodyType::FormData => {
            let fields = request.form_fields().unwrap_or_default();
            Some(json!({"mode": "urlencoded", "urlencoded": export_pairs(&fields)}))
        }
        BodyType::Binary => Some(json!({"mode": "file", "file": {"src": text()}})),
        BodyType::Multipart => {
            let mut fields = Vec::new();
            for item in request.multipart_fields().unwrap_or_default() {
                let file = item.kind == FormKind::File;
                let mut field = Map::new();
                field.insert("key".into(), Value::String(item.name.clone()));
                field.insert(
                    "type".into(),
                    Value::String(if file { "file" } else { "text" }.into()),
                );
                field.insert("disabled".into(), Value::Bool(!item.is_enabled));
                field.insert(
                    if file { "src" } else { "value" }.into(),
                    Value::String(item.value.clone()),
                );
                if !item.content_type.is_empty() {
                    field.insert(
                        "contentType".into(),
                        Value::String(item.content_type.clone()),
                    );
                }
                if !item.filename.is_empty() {
                    warnings.push(format!(
                        "{}: custom multipart filename omitted",
                        request.name
                    ));
                }
                fields.push(Value::Object(field));
            }
            Some(json!({"mode": "formdata", "formdata": fields}))
        }
        BodyType::None => None,
    };
    if let (Some(body), Some(map)) = (body, data.as_object_mut()) {
        map.insert("body".into(), body);
    }
    Ok(json!({"name": request.name, "request": data}))
}

fn export_items(
    folders: &[TransferFolder],
    requests: &[RequestTemplate],
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>, PostmanError> {
    let mut children: Vec<(i64, Value)> = Vec::new();
    for folder in folders {
        let inner = export_items(&folder.folders, &folder.requests, warnings)?;
        children.push((
            folder.position,
            json!({"name": folder.name.as_str(), "description": folder.description, "item": inner}),
        ));
    }
    for request in requests {
        children.push((request.position, export_request(request, warnings)?));
    }
    children.sort_by_key(|(position, _)| *position);
    Ok(children.into_iter().map(|(_, item)| item).collect())
}

/// Export the supported subset; the result is validated against the schema.
pub fn encode(document: &NativeDocument) -> Result<(Value, Vec<String>), PostmanError> {
    let mut warnings = Vec::new();
    let collection = &document.collection;
    let data = json!({
        "info": {"name": collection.name.as_str(), "description": collection.description, "schema": SCHEMA_URL},
        "item": export_items(&collection.folders, &collection.requests, &mut warnings)?,
        "variable": collection.variables.iter().map(|(k, v)| json!({"key": k, "value": v, "type": "string"})).collect::<Vec<_>>(),
    });
    validate(&data)?;
    Ok((data, warnings))
}

/// Pretty JSON with a trailing newline, as the Python app writes exports.
pub fn to_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

pub fn companion(name: &str, values: &Variables, scope: &str) -> Value {
    json!({
        "name": name,
        "_postman_variable_scope": scope,
        "values": values.iter().map(|(k, v)| json!({"key": k, "value": v, "enabled": true, "type": "default"})).collect::<Vec<_>>(),
    })
}

/// Publish the collection with its environment and globals companions in a new directory, all
/// or nothing: files are written to a temporary directory next to it, then renamed into place.
pub fn write_bundle(
    path: &Path,
    collection: &[u8],
    document: &NativeDocument,
) -> Result<(), PostmanError> {
    if path.exists() {
        return Err(PostmanError::DestinationExists);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".poster-export-")
        .tempdir_in(parent)?;
    atomic_write(
        &temporary.path().join("collection.postman_collection.json"),
        collection,
    )?;
    for (index, environment) in document.environments.iter().enumerate() {
        let data = companion(
            environment.name.as_str(),
            &environment.variables,
            "environment",
        );
        atomic_write(
            &temporary.path().join(format!(
                "environment-{}.postman_environment.json",
                index + 1
            )),
            &to_bytes(&data),
        )?;
    }
    if !document.globals.is_empty() {
        let data = companion("Globals", &document.globals, "globals");
        atomic_write(
            &temporary.path().join("globals.postman_globals.json"),
            &to_bytes(&data),
        )?;
    }
    std::fs::rename(temporary.path(), path)?;
    // The directory now lives at `path`; nothing is left for the guard to remove.
    let _ = temporary.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_schema_loads_and_validates() {
        assert!(VALIDATOR.is_ok());
        assert!(
            validate(&json!({"info": {"name": "C", "schema": SCHEMA_URL}, "item": []})).is_ok()
        );
        assert!(validate(&json!({"info": {"name": "C"}, "item": "nope"})).is_err());
    }

    #[test]
    fn python_helpers_match() {
        assert_eq!(
            py_dumps(&json!({"a": [1, true, null, "é"]})),
            r#"{"a": [1, true, null, "é"]}"#
        );
        assert_eq!(py_repr("a"), "'a'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a'\"b"), "'a\\'\"b'");
        assert_eq!(strip_dynamic("x{{$guid}}y{{ $t }}z"), Some("xyz".into()));
        assert_eq!(strip_dynamic("{{normal}}"), None);
        assert_eq!(strip_dynamic("{{$a\n}}"), None);
        assert_eq!(
            replace_path_variable("/u/:id/:idx?:id", "id", "7"),
            "/u/7/:idx?7"
        );
    }
}
