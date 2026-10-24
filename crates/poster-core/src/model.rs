//! Request templates as stored and exchanged. The serde shape is the native `.poster.json` v1
//! format, field for field: every field has a default, unknown fields are rejected, and `null`
//! is written for unset options exactly as the Python app does.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{CollectionId, FolderId, RequestId};

/// An ordered list of variables. Order is kept for display and export.
pub type Variables = IndexMap<String, String>;

fn yes() -> bool {
    true
}

/// A header, query parameter, cookie or form field. Duplicates and empty values are allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamItem {
    pub name: String,
    pub value: String,
    #[serde(default = "yes")]
    pub is_enabled: bool,
}

impl ParamItem {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            is_enabled: true,
        }
    }

    /// Enabled rows with a name are the ones that reach the wire.
    pub fn is_sent(&self) -> bool {
        self.is_enabled && !self.name.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormKind {
    #[default]
    Text,
    File,
}

/// A multipart field: inline text, or a file referenced by path (never its contents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormItem {
    pub name: String,
    pub value: String,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    #[serde(default)]
    pub kind: FormKind,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub filename: String,
}

impl FormItem {
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            is_enabled: true,
            kind: FormKind::Text,
            content_type: String::new(),
            filename: String::new(),
        }
    }

    pub fn file(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: FormKind::File,
            ..Self::text(name, path)
        }
    }

    pub fn is_sent(&self) -> bool {
        self.is_enabled && !self.name.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Basic,
    Digest,
    BearerToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BearerToken {
    pub token: String,
}

/// The stored auth shape keeps every configured scheme, so switching the type in the editor
/// does not lose the other credentials. [`RequestAuth::active`] gives the one in effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RequestAuth {
    #[serde(rename = "type")]
    pub kind: Option<AuthKind>,
    pub basic: Option<Credentials>,
    pub digest: Option<Credentials>,
    pub bearer_token: Option<BearerToken>,
}

/// The authentication in effect for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth<'a> {
    Basic(&'a Credentials),
    Digest(&'a Credentials),
    Bearer(&'a BearerToken),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Authentication credentials are missing")]
pub struct MissingCredentials;

impl RequestAuth {
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            kind: Some(AuthKind::Basic),
            basic: Some(Credentials {
                username: username.into(),
                password: password.into(),
            }),
            ..Self::default()
        }
    }

    pub fn digest(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            kind: Some(AuthKind::Digest),
            digest: Some(Credentials {
                username: username.into(),
                password: password.into(),
            }),
            ..Self::default()
        }
    }

    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            kind: Some(AuthKind::BearerToken),
            bearer_token: Some(BearerToken {
                token: token.into(),
            }),
            ..Self::default()
        }
    }

    pub fn active(&self) -> Result<Option<Auth<'_>>, MissingCredentials> {
        Ok(match self.kind {
            None => None,
            Some(AuthKind::Basic) => {
                Some(Auth::Basic(self.basic.as_ref().ok_or(MissingCredentials)?))
            }
            Some(AuthKind::Digest) => Some(Auth::Digest(
                self.digest.as_ref().ok_or(MissingCredentials)?,
            )),
            Some(AuthKind::BearerToken) => Some(Auth::Bearer(
                self.bearer_token.as_ref().ok_or(MissingCredentials)?,
            )),
        })
    }
}

/// Per-request overrides. `None` inherits the application default; `Some("")` for a path or
/// proxy explicitly disables an inherited value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Options {
    pub follow_redirects: Option<bool>,
    pub verify_ssl: Option<bool>,
    pub attach_cookies: Option<bool>,
    pub proxy_url: Option<String>,
    #[serde(deserialize_with = "positive_timeout")]
    pub timeout: Option<f64>,
    pub ca_bundle: Option<String>,
    pub certificate_path: Option<String>,
    pub key_file: Option<String>,
    pub password: Option<String>,
    /// KiB; `0` captures the whole body.
    pub download_limit: Option<u64>,
}

impl Options {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn positive_timeout<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    let value = Option::<f64>::deserialize(deserializer)?;
    match value {
        Some(seconds) if !(seconds > 0.0 && seconds.is_finite()) => Err(serde::de::Error::custom(
            "timeout must be a positive number of seconds",
        )),
        other => Ok(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BodyType {
    #[serde(rename = "none")]
    None,
    #[default]
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "plain")]
    Plain,
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "form-data")]
    FormData,
    #[serde(rename = "urlencoded")]
    Urlencoded,
    #[serde(rename = "multipart")]
    Multipart,
    #[serde(rename = "binary")]
    Binary,
}

impl BodyType {
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::Json,
        Self::Plain,
        Self::Text,
        Self::FormData,
        Self::Urlencoded,
        Self::Multipart,
        Self::Binary,
    ];

    /// The body bytes hold a JSON array of form items rather than raw content.
    pub const fn is_form(self) -> bool {
        matches!(self, Self::FormData | Self::Urlencoded | Self::Multipart)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json => "json",
            Self::Plain => "plain",
            Self::Text => "text",
            Self::FormData => "form-data",
            Self::Urlencoded => "urlencoded",
            Self::Multipart => "multipart",
            Self::Binary => "binary",
        }
    }
}

/// The portable part of a request: what the user edits, without database identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RequestTemplate {
    pub position: i64,
    pub name: String,
    pub description: String,
    /// An HTTP token; custom methods keep their case.
    pub method: String,
    pub protocol: String,
    pub url: String,
    pub path: String,
    pub auth: RequestAuth,
    pub queries: Vec<ParamItem>,
    pub headers: Vec<ParamItem>,
    pub cookies: Vec<ParamItem>,
    /// Raw content, a JSON array of form items (form body types) or a file path (binary).
    #[serde(with = "crate::base64_bytes")]
    pub body: Vec<u8>,
    pub body_type: BodyType,
    pub options: Options,
}

impl Default for RequestTemplate {
    fn default() -> Self {
        Self {
            position: 0,
            name: String::new(),
            description: String::new(),
            method: "GET".to_owned(),
            protocol: String::new(),
            url: String::new(),
            path: String::new(),
            auth: RequestAuth::default(),
            queries: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            body: Vec::new(),
            body_type: BodyType::default(),
            options: Options::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FormBodyError {
    #[error("Form body must be a JSON array of fields: {0}")]
    Invalid(#[from] serde_json::Error),
}

impl RequestTemplate {
    /// Parse a form body (`form-data`, `urlencoded`) into fields. An empty body has none.
    pub fn form_fields(&self) -> Result<Vec<ParamItem>, FormBodyError> {
        parse_form(&self.body)
    }

    /// Parse a `multipart` body into fields. An empty body has none.
    pub fn multipart_fields(&self) -> Result<Vec<FormItem>, FormBodyError> {
        parse_form(&self.body)
    }

    pub fn set_form_fields(&mut self, fields: &[ParamItem]) {
        self.body = encode_form(fields);
    }

    pub fn set_multipart_fields(&mut self, fields: &[FormItem]) {
        self.body = encode_form(fields);
    }

    /// Check that a form body parses for its type, as an import must before writing anything.
    pub fn validate_body(&self) -> Result<(), FormBodyError> {
        match self.body_type {
            BodyType::FormData | BodyType::Urlencoded => self.form_fields().map(drop),
            BodyType::Multipart => self.multipart_fields().map(drop),
            _ => Ok(()),
        }
    }
}

fn parse_form<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<Vec<T>, FormBodyError> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_slice(body)?)
}

fn encode_form<T: Serialize>(fields: &[T]) -> Vec<u8> {
    // Serializing plain structs of strings and bools into a Vec cannot fail.
    serde_json::to_vec(fields).expect("form fields always serialize")
}

/// A request with its place in the database, or a draft (`id` is `None`) not yet saved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Request {
    pub id: Option<RequestId>,
    pub collection_id: Option<CollectionId>,
    pub folder_id: Option<FolderId>,
    pub template: RequestTemplate,
    /// Collection variables captured at execution, used when the collection is gone.
    pub collection_variables_snapshot: Variables,
}

/// The HTTP methods offered in the method picker; any other token may be typed.
pub const STANDARD_METHODS: [&str; 7] =
    ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// RFC 9110 `token`: what an HTTP method must be.
pub fn is_method_token(method: &str) -> bool {
    !method.is_empty()
        && method
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_tokens() {
        for valid in ["GET", "get", "PROPFIND", "M-SEARCH", "x~y"] {
            assert!(is_method_token(valid), "{valid}");
        }
        for invalid in ["", "GE T", "GÉT", "GET\n", "(GET)"] {
            assert!(!is_method_token(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn active_auth_requires_the_selected_configuration() {
        assert_eq!(RequestAuth::default().active(), Ok(None));
        let basic = RequestAuth::basic("u", "p");
        assert!(matches!(basic.active(), Ok(Some(Auth::Basic(c))) if c.username == "u"));
        let missing = RequestAuth {
            kind: Some(AuthKind::Digest),
            ..RequestAuth::basic("u", "p")
        };
        assert_eq!(missing.active(), Err(MissingCredentials));
        assert!(
            matches!(RequestAuth::bearer("t").active(), Ok(Some(Auth::Bearer(b))) if b.token == "t")
        );
    }

    #[test]
    fn form_bodies_round_trip_and_reject_wrong_shapes() {
        let mut template = RequestTemplate {
            body_type: BodyType::Multipart,
            ..RequestTemplate::default()
        };
        assert_eq!(template.multipart_fields().unwrap(), []);
        let fields = [FormItem::text("a", "1"), FormItem::file("f", "/x")];
        template.set_multipart_fields(&fields);
        assert_eq!(template.multipart_fields().unwrap(), fields);
        template.validate_body().unwrap();

        template.body = br#"{"a": 1}"#.to_vec();
        assert!(template.validate_body().is_err());
        template.body_type = BodyType::Urlencoded;
        template.body = br#"[{"name": "a", "value": "b", "kind": "file"}]"#.to_vec();
        assert!(
            template.validate_body().is_err(),
            "plain form fields have no kind"
        );
        template.body_type = BodyType::Json;
        template.validate_body().unwrap();
    }

    #[test]
    fn timeout_must_be_positive() {
        assert!(serde_json::from_str::<Options>(r#"{"timeout": 0}"#).is_err());
        assert!(serde_json::from_str::<Options>(r#"{"timeout": -1.5}"#).is_err());
        assert_eq!(
            serde_json::from_str::<Options>(r#"{"timeout": 5}"#)
                .unwrap()
                .timeout,
            Some(5.0)
        );
        assert_eq!(
            serde_json::from_str::<Options>(r#"{"timeout": null}"#)
                .unwrap()
                .timeout,
            None
        );
    }

    #[test]
    fn param_rows_reach_the_wire_only_when_enabled_and_named() {
        assert!(ParamItem::new("a", "").is_sent());
        assert!(!ParamItem::new("", "v").is_sent());
        assert!(
            !ParamItem {
                is_enabled: false,
                ..ParamItem::new("a", "v")
            }
            .is_sent()
        );
    }
}
