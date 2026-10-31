//! Pointing at parts of a JSON response body with RFC 9535 JSONPath.
//!
//! Paths are written one way everywhere, so a path copied from the viewer works in a filter:
//! `.name` when the member name allows the shorthand, `['name']` otherwise, `[3]` for array
//! elements.

use std::fmt::Write as _;

use serde_json::Value;
use serde_json_path::{JsonPath, PathElement};

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
