//! Variable presentation helpers: which `{{tokens}}` to mark, what to suggest while typing, and
//! the Variables screen's editable scope. Pure functions; widgets only draw their results.

use std::ops::Range;

use imposter_app::core::generate;
use imposter_app::core::model::{SavedValues, Variables};
pub use imposter_app::core::variables::Scope;
use imposter_app::core::variables::{Definition, Snapshot, Source};

/// A `{{name}}` token as the editors mark it: no braces or newline inside, not empty (the
/// Python app's `\{\{([^{}\n]+)\}\}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub name: String,
}

pub fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find("{{") {
        let start = from + offset;
        let body = start + 2;
        let end = text[body..].find(['{', '}', '\n']).map(|e| body + e);
        match end {
            Some(end) if end > body && text[end..].starts_with("}}") => {
                out.push(Token {
                    range: start..end + 2,
                    name: text[body..end].trim().to_owned(),
                });
                from = end + 2;
            }
            _ => from = start + 1,
        }
    }
    out
}

pub fn token_at(text: &str, byte: usize) -> Option<Token> {
    tokens(text)
        .into_iter()
        .find(|t| t.range.start <= byte && byte < t.range.end)
}

/// The unfinished `{{query` just before `cursor` (a byte offset), if the cursor is inside one.
pub fn open_token(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let before = &text[..cursor];
    let start = before.rfind("{{")?;
    let typed = &before[start + 2..];
    if typed.contains('\n') || typed.contains("}}") {
        return None;
    }
    Some((start, typed))
}

/// A name offered while typing `{{`, and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub name: String,
    /// The scope and its name, or what a generated value makes. Never a value.
    pub detail: String,
}

/// Defined names matching `query` (case-insensitive substring), in definition order. Once
/// the query starts with `$`, generated values come first. Names only: suggestions never
/// show values.
pub fn suggestions(snapshot: &Snapshot, query: &str) -> Vec<Suggestion> {
    let query = query.trim().to_lowercase();
    let defined = snapshot
        .definitions
        .iter()
        .filter(|(name, _)| name.to_lowercase().contains(&query))
        .filter_map(|(name, definitions)| {
            let source = &definitions.first()?.source;
            Some(Suggestion {
                name: name.clone(),
                detail: format!("{} · {}", scope_label(source), source.label()),
            })
        });
    if !query.starts_with('$') {
        return defined.collect();
    }
    generate::FUNCTIONS
        .iter()
        .filter(|(name, _)| name.to_lowercase().contains(&query))
        .map(|(name, description)| Suggestion {
            name: (*name).to_owned(),
            detail: format!("Generated · {description}"),
        })
        .chain(defined)
        .collect()
}

/// Whether a `{{name}}` resolves: a defined variable, or a generated value it can make.
pub fn resolves(snapshot: Option<&Snapshot>, name: &str) -> bool {
    snapshot.is_some_and(|s| s.definitions.contains_key(name))
        || matches!(generate::describe(name), Some(Ok(_)))
}

/// Replace the open token starting at `start` (up to `cursor`, plus closing braces already
/// typed after it) with `{{name}}`; returns the new text and the cursor after the token.
pub fn complete(text: &str, start: usize, cursor: usize, name: &str) -> (String, usize) {
    let after = &text[cursor..];
    let closing = if after.starts_with("}}") {
        2
    } else {
        usize::from(after.starts_with('}'))
    };
    let replacement = format!("{{{{{name}}}}}");
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..start]);
    out.push_str(&replacement);
    out.push_str(&text[cursor + closing..]);
    (out, start + replacement.len())
}

pub fn scope_label(source: &Source) -> &'static str {
    match source {
        Source::Global => "Global",
        Source::Collection { .. } | Source::Captured => "Collection",
        Source::Input => "Input",
        Source::Environment { .. } => "Environment",
    }
}

/// Where a value is defined, as a sentence says it: "Global", "the Dev environment".
pub fn where_defined(source: &Source) -> String {
    match source {
        Source::Global => "Global".to_owned(),
        Source::Environment { name, .. } => format!("the {name} environment"),
        Source::Collection { name, .. } => format!("the {name} collection"),
        Source::Captured => format!("the {}", Source::CAPTURED_NAME.to_lowercase()),
        Source::Input => "a command-line input".to_owned(),
    }
}

/// One heading and value per definition for the inspector, highest precedence first: "Defined
/// in the Dev environment (overrides Global)", then "Overridden in Global" for each other.
pub fn describe(definitions: &[Definition], masked: bool) -> Vec<(String, String)> {
    const PREVIEW: usize = 4096;
    let overridden: Vec<String> = definitions
        .iter()
        .skip(1)
        .map(|d| where_defined(&d.source))
        .collect();
    definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let heading = if index > 0 {
                format!("Overridden in {}", where_defined(&definition.source))
            } else if overridden.is_empty() {
                format!("Defined in {}", where_defined(&definition.source))
            } else {
                format!(
                    "Defined in {} (overrides {})",
                    where_defined(&definition.source),
                    join_and(&overridden)
                )
            };
            let value = if masked {
                "••••••••".to_owned()
            } else if definition.value.is_empty() {
                "Empty value".to_owned()
            } else if definition.value.chars().count() > PREVIEW {
                let cut: String = definition.value.chars().take(PREVIEW).collect();
                format!("{cut}\n… Value preview limited to 4096 characters")
            } else {
                definition.value.clone()
            };
            (heading, value)
        })
        .collect()
}

/// "a", "a and b", "a, b and c".
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One editable variable. `key` identifies the row's widgets while rows above it are added,
/// removed or the layout shifts, so typing keeps its focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: u64,
    pub name: String,
    pub value: String,
    /// Values saved to swap the current one with, in the order saved.
    pub saved: Vec<String>,
}

impl Row {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            key: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name: name.into(),
            value: value.into(),
            saved: Vec::new(),
        }
    }

    pub fn with_saved(
        name: impl Into<String>,
        value: impl Into<String>,
        saved: Vec<String>,
    ) -> Self {
        Self {
            saved,
            ..Self::new(name, value)
        }
    }

    /// Keep the current value for swapping back to later; a value is saved once.
    pub fn save_current(&mut self) {
        let value = self.value.clone();
        self.add_saved(value);
    }

    /// Keep `value` for later without touching the current value. Returns whether it was
    /// new; empty and already kept values are not.
    pub fn add_saved(&mut self, value: String) -> bool {
        if value.is_empty() || self.saved.contains(&value) {
            return false;
        }
        self.saved.push(value);
        true
    }
}

/// A named row as the scope stores it: name, current value and saved values.
pub type Entry = (String, String, Vec<String>);

/// Rows with a name, in order: what the scope holds.
pub fn entries(rows: &[Row]) -> Vec<Entry> {
    rows.iter()
        .filter(|r| !r.name.is_empty())
        .map(|r| (r.name.clone(), r.value.clone(), r.saved.clone()))
        .collect()
}

pub fn rows(variables: &Variables, saved: &SavedValues) -> Vec<Row> {
    variables
        .iter()
        .map(|(name, value)| Row::with_saved(name.clone(), value.clone(), saved.values(name)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Variable names must be unique within a scope (\"{0}\" repeats)")]
pub struct DuplicateName(pub String);

/// The rows as the scope stores them; names must be unique.
pub fn to_scope(rows: &[Row]) -> Result<(Variables, SavedValues), DuplicateName> {
    let mut variables = Variables::new();
    let mut saved = SavedValues::default();
    for (name, value, values) in entries(rows) {
        if variables.contains_key(&name) {
            return Err(DuplicateName(name));
        }
        saved.set(&name, values);
        variables.insert(name, value);
    }
    Ok((variables, saved))
}

#[cfg(test)]
mod tests {
    use imposter_app::core::ids::{CollectionId, EnvironmentId};
    use imposter_app::core::variables::{ActiveEnvironment, CollectionScope, VariableContext};

    use super::*;

    fn snapshot() -> Snapshot {
        let globals: Variables = [
            ("host".into(), "g".into()),
            ("token".into(), "secret".into()),
        ]
        .into_iter()
        .collect();
        let collection: Variables = [("Host".into(), "c".into())].into_iter().collect();
        let env: Variables = [("host".into(), String::new())].into_iter().collect();
        Snapshot::build(
            &VariableContext {
                globals,
                environment: Some(ActiveEnvironment {
                    id: Some(EnvironmentId(4)),
                    name: "Dev".into(),
                    values: env,
                    saved: SavedValues::default(),
                }),
                ..VariableContext::default()
            },
            Some(CollectionScope::Live {
                id: Some(CollectionId(2)),
                name: "API",
                values: &collection,
                saved: &SavedValues::default(),
            }),
        )
    }

    #[test]
    fn tokens_follow_the_highlight_pattern() {
        let names: Vec<String> = tokens("{{a}} {{ b }} {{}} {{c\n}} {{x{y}} {{d}}")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["a", "b", "d"]);
        assert_eq!(token_at("go {{host}}/x", 5).unwrap().name, "host");
        assert_eq!(token_at("go {{host}}/x", 1), None);
        assert_eq!(token_at("go {{host}}/x", 11), None);
    }

    #[test]
    fn the_open_token_is_found_before_the_cursor() {
        assert_eq!(open_token("http://{{ho", 11), Some((7, "ho")));
        assert_eq!(open_token("{{a}} and {{", 12), Some((10, "")));
        assert_eq!(open_token("{{done}} x", 10), None);
        assert_eq!(open_token("{{a\nb", 5), None);
        assert_eq!(open_token("plain", 5), None);
    }

    #[test]
    fn suggestions_match_names_without_values() {
        let snapshot = snapshot();
        let names: Vec<String> = suggestions(&snapshot, "HO")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["host", "Host"]);
        assert_eq!(
            suggestions(&snapshot, "host")[0].detail,
            "Environment · Dev"
        );
        assert_eq!(
            suggestions(&snapshot, "").len(),
            3,
            "no generated values unasked"
        );
    }

    #[test]
    fn a_dollar_suggests_generated_values_and_they_resolve() {
        let snapshot = snapshot();
        let names: Vec<String> = suggestions(&snapshot, "$rand")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["$randomUUID", "$randomInt", "$randomString"]);
        assert!(
            suggestions(&snapshot, "$")[0]
                .detail
                .starts_with("Generated · ")
        );
        assert!(resolves(Some(&snapshot), "host"));
        assert!(resolves(None, "$uuid label"));
        assert!(!resolves(Some(&snapshot), "$randomInt 9 1"));
        assert!(!resolves(Some(&snapshot), "$nope"));
        assert!(!resolves(None, "host"));
    }

    #[test]
    fn completion_replaces_the_open_token_and_closing_braces() {
        assert_eq!(
            complete("http://{{ho/x", 7, 11, "host"),
            ("http://{{host}}/x".into(), 15)
        );
        assert_eq!(complete("{{ho}}", 0, 4, "host"), ("{{host}}".into(), 8));
        assert_eq!(complete("{{ho}", 0, 4, "host"), ("{{host}}".into(), 8));
    }

    #[test]
    fn inspection_lists_definitions_with_masking_and_previews() {
        let snapshot = snapshot();
        let lines = describe(&snapshot.definitions["host"], false);
        assert_eq!(
            lines[0],
            (
                "Defined in the Dev environment (overrides Global)".into(),
                "Empty value".into()
            )
        );
        assert_eq!(lines[1], ("Overridden in Global".into(), "g".into()));
        let api = Definition {
            value: "c".into(),
            source: Source::Collection {
                id: None,
                name: "API".into(),
            },
            saved: Vec::new(),
        };
        let mut three = snapshot.definitions["host"].clone();
        three.insert(1, api);
        assert_eq!(
            describe(&three, false)[0].0,
            "Defined in the Dev environment (overrides the API collection and Global)"
        );
        assert_eq!(
            describe(&three[2..], false)[0].0,
            "Defined in Global",
            "nothing to override"
        );
        assert_eq!(
            describe(&snapshot.definitions["token"], true)[0].1,
            "••••••••"
        );
        let long = Definition {
            value: "x".repeat(5000),
            source: Source::Global,
            saved: Vec::new(),
        };
        assert!(
            describe(&[long], false)[0]
                .1
                .ends_with("… Value preview limited to 4096 characters")
        );
        assert_eq!(Scope::of(&Source::Captured), None);
        assert_eq!(
            where_defined(&Source::Captured),
            "the captured collection (deleted)"
        );
        assert_eq!(
            Scope::of(&Source::Environment {
                id: Some(EnvironmentId(1)),
                name: "x".into()
            }),
            Some(Scope::Environment(EnvironmentId(1)))
        );
    }

    #[test]
    fn scope_rows_round_trip_and_reject_duplicates() {
        let variables: Variables = [("a".into(), "1".into()), ("b".into(), String::new())]
            .into_iter()
            .collect();
        let mut saved = SavedValues::default();
        saved.set("a", ["1".to_owned(), "9".to_owned()]);
        let mut edited = rows(&variables, &saved);
        assert_eq!(edited[0].saved, ["1", "9"]);
        edited.push(Row::new("", ""));
        assert_eq!(to_scope(&edited).unwrap(), (variables, saved));
        edited.push(Row::new("a", "2"));
        assert_eq!(to_scope(&edited), Err(DuplicateName("a".into())));
    }

    #[test]
    fn saving_the_current_value_keeps_each_value_once() {
        let mut row = Row::new("id", "42");
        row.save_current();
        row.value = "77".into();
        row.save_current();
        row.save_current();
        assert_eq!(row.saved, ["42", "77"]);
    }
}
