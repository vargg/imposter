//! Checks: declarative expectations about a response, stored with the request and evaluated
//! after every response. No scripts: a check is a selector, an operator and an expected value.

use std::borrow::Cow;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution::{Outcome, Response};
use crate::model::Variables;
use crate::select::{ResponseView, Selector};
use crate::variables::substitute;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Equals,
    NotEquals,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Contains,
    NotContains,
    Matches,
    Exists,
    NotExists,
    IsType,
}

impl Op {
    pub const ALL: [Self; 12] = [
        Self::Equals,
        Self::NotEquals,
        Self::Less,
        Self::LessOrEqual,
        Self::Greater,
        Self::GreaterOrEqual,
        Self::Contains,
        Self::NotContains,
        Self::Matches,
        Self::Exists,
        Self::NotExists,
        Self::IsType,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Equals => "=",
            Self::NotEquals => "≠",
            Self::Less => "<",
            Self::LessOrEqual => "≤",
            Self::Greater => ">",
            Self::GreaterOrEqual => "≥",
            Self::Contains => "contains",
            Self::NotContains => "does not contain",
            Self::Matches => "matches",
            Self::Exists => "exists",
            Self::NotExists => "does not exist",
            Self::IsType => "is of type",
        }
    }

    /// Whether the operator compares with an expected value.
    pub const fn takes_value(self) -> bool {
        !matches!(self, Self::Exists | Self::NotExists)
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub selector: Selector,
    pub op: Op,
    /// May contain `{{variables}}`, resolved from the same snapshot as the request.
    #[serde(default)]
    pub expected: String,
}

impl Check {
    pub fn new(selector: Selector, op: Op, expected: impl Into<String>) -> Self {
        Self {
            enabled: true,
            selector,
            op,
            expected: expected.into(),
        }
    }

    /// `status = 200`, `$.id exists`, `header Content-Type contains json`.
    pub fn describe(&self) -> String {
        let mut text = format!("{} {}", self.selector.describe(), self.op.label());
        if self.op.takes_value() {
            text.push(' ');
            text.push_str(&self.expected);
        }
        text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Failed,
    /// The check could not be decided, and why: a cut or non-JSON body, a bad path or regular
    /// expression, an unresolved variable.
    Unchecked(String),
}

/// Values longer than this are shortened in results; a body check must not copy the body.
pub const ACTUAL_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    /// The check as evaluated, its expected value resolved.
    pub check: Check,
    pub verdict: Verdict,
    /// What the selector read, as text; `None` when it named nothing.
    #[serde(default)]
    pub actual: Option<String>,
}

/// Evaluate the enabled `checks` against `response`. Nothing is checked when no response
/// arrived (a transport error or a cancel).
pub fn evaluate(checks: &[Check], response: &Response, values: &Variables) -> Vec<CheckResult> {
    if response.outcome != Outcome::Response {
        return Vec::new();
    }
    let view = ResponseView::new(response);
    checks
        .iter()
        .filter(|check| check.enabled)
        .map(|check| {
            let mut missing = BTreeSet::new();
            let expected = substitute(
                &check.expected,
                |name| values.get(name).map(String::as_str),
                &mut missing,
            );
            let check = Check {
                expected,
                ..check.clone()
            };
            if !missing.is_empty() {
                let names: Vec<String> = missing.into_iter().collect();
                return CheckResult {
                    check,
                    verdict: Verdict::Unchecked(format!(
                        "Unresolved variables: {}",
                        names.join(", ")
                    )),
                    actual: None,
                };
            }
            match view.read(&check.selector) {
                Err(error) => CheckResult {
                    check,
                    verdict: Verdict::Unchecked(error.to_string()),
                    actual: None,
                },
                Ok(actual) => {
                    let verdict = decide(&check, actual.as_ref());
                    CheckResult {
                        check,
                        verdict,
                        actual: actual.as_ref().map(|value| shorten(&text(value))),
                    }
                }
            }
        })
        .collect()
}

fn shorten(text: &str) -> String {
    match text.char_indices().nth(ACTUAL_LIMIT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// A value as the user reads it: strings without quotes, everything else as JSON.
fn text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(other.to_string()),
    }
}

fn verdict(passed: bool) -> Verdict {
    if passed {
        Verdict::Passed
    } else {
        Verdict::Failed
    }
}

fn decide(check: &Check, actual: Option<&Value>) -> Verdict {
    let expected = check.expected.trim();
    let Some(actual) = actual else {
        return verdict(check.op == Op::NotExists);
    };
    match check.op {
        Op::Exists => Verdict::Passed,
        Op::NotExists => Verdict::Failed,
        Op::Equals | Op::NotEquals => {
            let same = match status_class(&check.selector, expected) {
                Some(class) => actual.as_u64().is_some_and(|status| status / 100 == class),
                None => equal(actual, &check.expected),
            };
            verdict(same == (check.op == Op::Equals))
        }
        Op::Less | Op::LessOrEqual | Op::Greater | Op::GreaterOrEqual => {
            let Ok(limit) = expected.parse::<f64>() else {
                return Verdict::Unchecked(format!("{expected:?} is not a number"));
            };
            let Some(number) = number(actual) else {
                return Verdict::Failed;
            };
            verdict(match check.op {
                Op::Less => number < limit,
                Op::LessOrEqual => number <= limit,
                Op::Greater => number > limit,
                _ => number >= limit,
            })
        }
        Op::Contains | Op::NotContains => {
            let contains = match actual {
                Value::String(text) => text.contains(&check.expected),
                Value::Array(items) => items.iter().any(|item| equal(item, &check.expected)),
                Value::Object(members) => members.contains_key(&check.expected),
                other => other.to_string().contains(&check.expected),
            };
            verdict(contains == (check.op == Op::Contains))
        }
        Op::Matches => match regex::Regex::new(&check.expected) {
            Ok(pattern) => verdict(pattern.is_match(&text(actual))),
            Err(error) => Verdict::Unchecked(format!("Invalid regular expression: {error}")),
        },
        Op::IsType => {
            let kind = match expected.to_ascii_lowercase().as_str() {
                "string" => actual.is_string(),
                "number" => actual.is_number(),
                "integer" => actual.is_i64() || actual.is_u64(),
                "boolean" => actual.is_boolean(),
                "null" => actual.is_null(),
                "array" => actual.is_array(),
                "object" => actual.is_object(),
                _ => {
                    return Verdict::Unchecked(format!(
                        "Unknown type {expected:?}: use string, number, integer, boolean, null, array or object"
                    ));
                }
            };
            verdict(kind)
        }
    }
}

/// `2xx`-style expectations for the status.
fn status_class(selector: &Selector, expected: &str) -> Option<u64> {
    let bytes = expected.as_bytes();
    (*selector == Selector::Status
        && bytes.len() == 3
        && (b'1'..=b'5').contains(&bytes[0])
        && bytes[1..].iter().all(|b| b.eq_ignore_ascii_case(&b'x')))
    .then(|| u64::from(bytes[0] - b'0'))
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Strings compare as text (`expected` may also be written as a JSON string); numbers by
/// value, so `1.0` equals `1`; anything else when `expected` parses as equal JSON.
#[allow(clippy::float_cmp)] // "=" means the same number, not one within a margin
fn equal(actual: &Value, expected: &str) -> bool {
    match actual {
        Value::String(text) => {
            text == expected
                || serde_json::from_str::<String>(expected.trim()).is_ok_and(|e| e == *text)
        }
        Value::Number(_) => expected
            .trim()
            .parse::<f64>()
            .ok()
            .zip(number(actual))
            .is_some_and(|(e, a)| e == a),
        other => serde_json::from_str::<Value>(expected).is_ok_and(|e| same_json(&e, other)),
    }
}

fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_json(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, a)| y.get(key).is_some_and(|b| same_json(a, b)))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Response {
        Response {
            status: 201,
            reason: "Created".into(),
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ],
            body: br#"{"id":7,"price":19.50,"name":"Ann","tags":["x","y"],"meta":{"ok":true},"none":null,"items":[{"n":1},{"n":2}]}"#.to_vec(),
            elapsed: 0.1234,
            ..Response::default()
        }
    }

    fn run(selector: Selector, op: Op, expected: &str) -> Verdict {
        let values: Variables = [("want".to_owned(), "Ann".to_owned())]
            .into_iter()
            .collect();
        let mut results = evaluate(&[Check::new(selector, op, expected)], &response(), &values);
        results.pop().unwrap().verdict
    }

    fn json(path: &str, op: Op, expected: &str) -> Verdict {
        run(Selector::Json(path.into()), op, expected)
    }

    #[test]
    fn status_compares_by_value_and_class() {
        assert_eq!(run(Selector::Status, Op::Equals, "201"), Verdict::Passed);
        assert_eq!(run(Selector::Status, Op::Equals, " 2xx "), Verdict::Passed);
        assert_eq!(run(Selector::Status, Op::Equals, "2XX"), Verdict::Passed);
        assert_eq!(run(Selector::Status, Op::NotEquals, "4xx"), Verdict::Passed);
        assert_eq!(run(Selector::Status, Op::Equals, "200"), Verdict::Failed);
        assert_eq!(run(Selector::Status, Op::Less, "300"), Verdict::Passed);
        assert_eq!(
            json("$.id", Op::Equals, "7xx"),
            Verdict::Failed,
            "classes are for the status only"
        );
    }

    #[test]
    fn headers_ignore_case_and_join_repeated_values() {
        let header = |name: &str| Selector::Header(name.into());
        assert_eq!(
            run(header("Content-Type"), Op::Contains, "json"),
            Verdict::Passed
        );
        assert_eq!(
            run(header("Set-Cookie"), Op::Equals, "a=1, b=2"),
            Verdict::Passed
        );
        assert_eq!(run(header("X-Missing"), Op::NotExists, ""), Verdict::Passed);
        assert_eq!(run(header("X-Missing"), Op::Exists, ""), Verdict::Failed);
        assert_eq!(run(header("X-Missing"), Op::Equals, ""), Verdict::Failed);
    }

    #[test]
    fn json_values_compare_by_kind() {
        assert_eq!(json("$.id", Op::Equals, "7"), Verdict::Passed);
        assert_eq!(json("$.id", Op::Equals, "7.0"), Verdict::Passed);
        assert_eq!(json("$.price", Op::Equals, "19.5"), Verdict::Passed);
        assert_eq!(json("$.name", Op::Equals, "Ann"), Verdict::Passed);
        assert_eq!(json("$.name", Op::Equals, "\"Ann\""), Verdict::Passed);
        assert_eq!(json("$.name", Op::Equals, "{{want}}"), Verdict::Passed);
        assert_eq!(json("$.name", Op::Equals, "ann"), Verdict::Failed);
        assert_eq!(
            json("$.meta", Op::Equals, r#"{ "ok": true }"#),
            Verdict::Passed
        );
        assert_eq!(json("$.tags", Op::Equals, r#"["x","y"]"#), Verdict::Passed);
        assert_eq!(json("$.tags", Op::Equals, r#"["y","x"]"#), Verdict::Failed);
        assert_eq!(json("$.none", Op::Equals, "null"), Verdict::Passed);
        assert_eq!(
            json("$.none", Op::Exists, ""),
            Verdict::Passed,
            "null exists"
        );
        assert_eq!(
            json("$.items[*].n", Op::Equals, "[1, 2.0]"),
            Verdict::Passed
        );
        assert_eq!(json("$.nothing", Op::NotExists, ""), Verdict::Passed);
        assert_eq!(json("$.nothing", Op::Equals, "1"), Verdict::Failed);
    }

    #[test]
    fn numbers_text_patterns_and_types() {
        assert_eq!(json("$.price", Op::Greater, "19"), Verdict::Passed);
        assert_eq!(json("$.price", Op::LessOrEqual, "19.5"), Verdict::Passed);
        assert_eq!(json("$.price", Op::GreaterOrEqual, "20"), Verdict::Failed);
        assert_eq!(
            json("$.name", Op::Less, "3"),
            Verdict::Failed,
            "not a number"
        );
        assert!(matches!(
            json("$.id", Op::Less, "many"),
            Verdict::Unchecked(_)
        ));
        assert_eq!(run(Selector::Duration, Op::Less, "124"), Verdict::Passed);
        assert_eq!(run(Selector::Duration, Op::Equals, "123"), Verdict::Passed);
        assert_eq!(run(Selector::Size, Op::Greater, "50"), Verdict::Passed);

        assert_eq!(json("$.name", Op::Contains, "nn"), Verdict::Passed);
        assert_eq!(json("$.tags", Op::Contains, "y"), Verdict::Passed);
        assert_eq!(json("$.tags", Op::NotContains, "z"), Verdict::Passed);
        assert_eq!(json("$.meta", Op::Contains, "ok"), Verdict::Passed, "a key");
        assert_eq!(json("$.id", Op::Contains, "7"), Verdict::Passed);
        assert_eq!(
            run(Selector::Body, Op::Contains, "\"Ann\""),
            Verdict::Passed
        );

        assert_eq!(json("$.name", Op::Matches, "^A.n$"), Verdict::Passed);
        assert_eq!(json("$.id", Op::Matches, r"^\d+$"), Verdict::Passed);
        assert!(
            matches!(json("$.name", Op::Matches, "("), Verdict::Unchecked(r) if r.starts_with("Invalid regular expression"))
        );

        for (path, kind) in [
            ("$.name", "string"),
            ("$.price", "number"),
            ("$.id", "integer"),
            ("$.meta.ok", "Boolean"),
            ("$.none", "null"),
            ("$.tags", "array"),
            ("$.meta", "object"),
        ] {
            assert_eq!(json(path, Op::IsType, kind), Verdict::Passed, "{path}");
        }
        assert_eq!(json("$.price", Op::IsType, "integer"), Verdict::Failed);
        assert!(matches!(
            json("$.id", Op::IsType, "date"),
            Verdict::Unchecked(_)
        ));
    }

    #[test]
    fn undecidable_checks_say_why() {
        assert!(
            matches!(json("id", Op::Exists, ""), Verdict::Unchecked(r) if r.starts_with("Invalid JSONPath"))
        );
        assert_eq!(
            json("$.x", Op::Equals, "{{nobody}}"),
            Verdict::Unchecked("Unresolved variables: nobody".into())
        );
        let mut cut = response();
        cut.truncated = true;
        for selector in [
            Selector::Json("$.id".into()),
            Selector::Body,
            Selector::Size,
        ] {
            let results = evaluate(
                &[Check::new(selector, Op::Exists, "")],
                &cut,
                &Variables::new(),
            );
            assert!(
                matches!(&results[0].verdict, Verdict::Unchecked(r) if r.contains("capture limit"))
            );
        }
        let mut html = response();
        html.body = b"<html>".to_vec();
        let results = evaluate(
            &[Check::new(Selector::Json("$".into()), Op::Exists, "")],
            &html,
            &Variables::new(),
        );
        assert!(
            matches!(&results[0].verdict, Verdict::Unchecked(r) if r.starts_with("The body is not JSON"))
        );
    }

    #[test]
    fn results_keep_the_resolved_check_and_a_short_actual_value() {
        let mut big = response();
        big.body = format!("{{\"long\":\"{}\"}}", "é".repeat(300)).into_bytes();
        let checks = [
            Check::new(Selector::Json("$.long".into()), Op::Exists, ""),
            Check {
                enabled: false,
                ..Check::new(Selector::Status, Op::Equals, "500")
            },
            Check::new(Selector::Status, Op::Equals, "{{want}}"),
        ];
        let values: Variables = [("want".to_owned(), "200".to_owned())]
            .into_iter()
            .collect();
        let results = evaluate(&checks, &big, &values);
        assert_eq!(results.len(), 2, "disabled checks are skipped");
        let actual = results[0].actual.as_deref().unwrap();
        assert_eq!(actual.chars().count(), ACTUAL_LIMIT + 1);
        assert!(actual.ends_with('…'));
        assert_eq!(results[1].check.expected, "200");
        assert_eq!(results[1].check.describe(), "status = 200");
        assert_eq!(results[1].actual.as_deref(), Some("201"));
        assert_eq!(results[1].verdict, Verdict::Failed);
    }

    #[test]
    fn nothing_is_checked_without_a_response() {
        let failed = Response::failure(Outcome::TransportError, "Connect", "refused");
        let checks = [Check::new(Selector::Status, Op::Equals, "200")];
        assert_eq!(evaluate(&checks, &failed, &Variables::new()), []);
    }

    #[test]
    fn checks_describe_themselves_and_store_compactly() {
        let check = Check::new(Selector::Header("ETag".into()), Op::Exists, "ignored");
        assert_eq!(check.describe(), "header ETag exists");
        assert_eq!(
            serde_json::to_string(&check).unwrap(),
            r#"{"enabled":true,"selector":{"header":"ETag"},"op":"exists","expected":"ignored"}"#
        );
        let status: Check =
            serde_json::from_str(r#"{"selector":"status","op":"equals","expected":"200"}"#)
                .unwrap();
        assert!(status.enabled, "enabled by default");
        assert!(
            serde_json::from_str::<Check>(r#"{"selector":"status","op":"equals","x":1}"#).is_err()
        );
        assert_eq!(Op::ALL.len(), 12);
        assert_eq!(Op::ALL.iter().filter(|op| !op.takes_value()).count(), 2);
    }
}
