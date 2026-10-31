//! Pointing at parts of a response: its status, a header, a JSONPath into the body, the body
//! text, the duration or the size.
//!
//! Paths are RFC 9535 JSONPath, written one way everywhere so a path copied from the viewer
//! works in a filter or a check: `.name` when the member name allows the shorthand, `['name']`
//! otherwise, `[3]` for array elements.

use std::cell::OnceCell;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json_path::{JsonPath, PathElement};

use crate::execution::Response;

/// Which part of a response to read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selector {
    Status,
    /// Every value of the header, joined with `, ` as HTTP allows.
    Header(String),
    /// A JSONPath into the body. Several matches read as an array.
    Json(String),
    /// The body as text.
    Body,
    /// Whole milliseconds.
    Duration,
    /// Bytes received.
    Size,
}

impl Selector {
    /// Short text for lists: `status`, `header Content-Type`, `$.id`, `body`, `duration`,
    /// `size`.
    pub fn describe(&self) -> String {
        match self {
            Self::Status => "status".into(),
            Self::Header(name) => format!("header {name}"),
            Self::Json(path) => path.clone(),
            Self::Body => "body".into(),
            Self::Duration => "duration (ms)".into(),
            Self::Size => "size (bytes)".into(),
        }
    }
}

/// A response with its body parsed at most once, however many selectors read it.
pub struct ResponseView<'a> {
    response: &'a Response,
    document: OnceCell<Result<Value, SelectError>>,
}

impl<'a> ResponseView<'a> {
    pub fn new(response: &'a Response) -> Self {
        Self {
            response,
            document: OnceCell::new(),
        }
    }

    /// What `selector` reads: `Ok(None)` when it names nothing (a missing header, a path
    /// without matches), an error when the response cannot answer (a cut or non-JSON body, a
    /// bad path).
    pub fn read(&self, selector: &Selector) -> Result<Option<Value>, SelectError> {
        let response = self.response;
        let complete = || {
            if response.truncated {
                Err(SelectError::Truncated)
            } else {
                Ok(())
            }
        };
        Ok(match selector {
            Selector::Status => Some(Value::from(response.status)),
            Selector::Header(name) => {
                let values: Vec<&str> = response
                    .headers
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
                    .map(|(_, v)| v.as_str())
                    .collect();
                (!values.is_empty()).then(|| Value::from(values.join(", ")))
            }
            Selector::Json(path) => {
                let path = parse_path(path)?;
                let document = self
                    .document
                    .get_or_init(|| parse_body(&response.body, response.truncated))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let mut found: Vec<Value> =
                    path.query(document).all().into_iter().cloned().collect();
                match found.len() {
                    0 => None,
                    1 => found.pop(),
                    _ => Some(Value::Array(found)),
                }
            }
            Selector::Body => {
                complete()?;
                Some(Value::from(String::from_utf8_lossy(&response.body)))
            }
            Selector::Duration => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // elapsed is a small non-negative number of seconds
                let millis = (response.elapsed * 1000.0).round().max(0.0) as u64;
                Some(Value::from(millis))
            }
            Selector::Size => {
                complete()?;
                Some(Value::from(response.body.len()))
            }
        })
    }
}

/// One value a query found, with the normalized path that leads to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub path: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    #[error("Invalid JSONPath: {0}")]
    InvalidPath(String),
    #[error("The body was cut at the capture limit, so it is not complete JSON")]
    Truncated,
    #[error("The body is not JSON: {0}")]
    NotJson(String),
}

/// The root path.
pub const ROOT: &str = "$";

pub fn parse_path(path: &str) -> Result<JsonPath, SelectError> {
    JsonPath::parse(path.trim()).map_err(|error| SelectError::InvalidPath(error.to_string()))
}

/// Parse a captured body as JSON. `truncated` bodies are refused rather than reported as
/// invalid, so the user learns why.
pub fn parse_body(body: &[u8], truncated: bool) -> Result<Value, SelectError> {
    if truncated {
        return Err(SelectError::Truncated);
    }
    serde_json::from_slice(body).map_err(|error| SelectError::NotJson(error.to_string()))
}

/// Every value `path` selects in `document`, in document order.
pub fn query(document: &Value, path: &JsonPath) -> Vec<Found> {
    path.query_located(document)
        .into_iter()
        .map(|node| {
            let mut text = String::from(ROOT);
            for element in node.location().iter() {
                match element {
                    PathElement::Name(name) => text = member(&text, name),
                    PathElement::Index(index) => text = element_path(&text, *index),
                }
            }
            Found {
                path: text,
                value: node.node().clone(),
            }
        })
        .collect()
}

/// The path of member `name` of the object at `parent`.
pub fn member(parent: &str, name: &str) -> String {
    let mut chars = name.chars();
    let shorthand = chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic() || !c.is_ascii())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii());
    if shorthand {
        return format!("{parent}.{name}");
    }
    let mut out = format!("{parent}['");
    for c in name.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push_str("']");
    out
}

/// The path of element `index` of the array at `parent`.
pub fn element_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str, path: &str) -> Result<Vec<(String, Value)>, SelectError> {
        let document = parse_body(body.as_bytes(), false)?;
        let path = parse_path(path)?;
        Ok(query(&document, &path)
            .into_iter()
            .map(|found| (found.path, found.value))
            .collect())
    }

    #[test]
    fn queries_return_values_with_their_paths() {
        let body = r#"{"items":[{"id":1,"active":true},{"id":2,"active":false}],"a b":{"it's":3}}"#;
        assert_eq!(
            run(body, "$.items[*].id").unwrap(),
            [
                ("$.items[0].id".to_owned(), Value::from(1)),
                ("$.items[1].id".to_owned(), Value::from(2)),
            ]
        );
        assert_eq!(
            run(body, "$.items[?@.active == true].id").unwrap(),
            [("$.items[0].id".to_owned(), Value::from(1))]
        );
        assert_eq!(
            run(body, "$..['it\\'s']").unwrap(),
            [("$['a b']['it\\'s']".to_owned(), Value::from(3))]
        );
        assert_eq!(run(body, "$").unwrap()[0].0, "$");
        assert_eq!(run(body, "$.missing").unwrap(), []);
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(matches!(
            run("{}", "items"),
            Err(SelectError::InvalidPath(_))
        ));
        assert!(matches!(run("{", "$"), Err(SelectError::NotJson(_))));
        assert_eq!(
            parse_body(b"{\"a\":1}", true),
            Err(SelectError::Truncated),
            "a cut body is refused even when it happens to parse"
        );
    }

    #[test]
    fn written_paths_select_what_they_name() {
        let names = [
            "plain",
            "_x9",
            "ünï",
            "9lives",
            "a b",
            "it's",
            "back\\slash",
            "tab\tnew\nline",
            "\u{1}",
            "",
            "a.b",
        ];
        let mut object = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            object.insert((*name).to_owned(), Value::from(index));
        }
        let document = Value::Array(vec![Value::Object(object)]);
        for (index, name) in names.iter().enumerate() {
            let path = member(&element_path(ROOT, 0), name);
            let found = query(&document, &parse_path(&path).unwrap());
            assert_eq!(found.len(), 1, "{path}");
            assert_eq!(found[0].value, Value::from(index), "{path}");
            assert_eq!(
                found[0].path, path,
                "normalized paths are written the same way"
            );
        }
        assert_eq!(member(ROOT, "plain"), "$.plain");
        assert_eq!(member(ROOT, "a b"), "$['a b']");
    }
}
