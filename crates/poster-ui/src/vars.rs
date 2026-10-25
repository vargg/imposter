//! Variable presentation helpers: which `{{tokens}}` to mark, what to suggest while typing, and
//! the Variables screen's editable scope. Pure functions; widgets only draw their results.

use std::ops::Range;

use poster_app::core::ids::{CollectionId, EnvironmentId};
use poster_app::core::model::Variables;
use poster_app::core::variables::{Definition, Snapshot, Source};

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

/// Defined names matching `query` (case-insensitive substring), in definition order. Names
/// only: suggestions never show values.
pub fn suggestions<'a>(snapshot: &'a Snapshot, query: &str) -> Vec<(&'a str, &'a Definition)> {
    let query = query.trim().to_lowercase();
    snapshot
        .definitions
        .iter()
        .filter(|(name, _)| name.to_lowercase().contains(&query))
        .filter_map(|(name, definitions)| Some((name.as_str(), definitions.first()?)))
        .collect()
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
        Source::Collection { .. } => "Collection",
        Source::Environment { .. } => "Environment",
    }
}

/// One line per definition for the inspector, highest precedence first.
pub fn describe(definitions: &[Definition], masked: bool) -> Vec<(String, String)> {
    const PREVIEW: usize = 4096;
    definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let heading = format!(
                "{} · {} · {}",
                if index == 0 {
                    "Effective"
                } else {
                    "Overridden"
                },
                scope_label(&definition.source),
                definition.source.label()
            );
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

/// The scope the Variables screen edits. Independent of the active execution environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scope {
    #[default]
    Global,
    Environment(EnvironmentId),
    Collection(CollectionId),
}

impl Scope {
    /// Where an inspected definition lives.
    pub fn of(source: &Source) -> Option<Self> {
        match source {
            Source::Global => Some(Self::Global),
            Source::Environment { id, .. } => Some(Self::Environment(*id)),
            Source::Collection { id: Some(id), .. } => Some(Self::Collection(*id)),
            Source::Collection { id: None, .. } => None,
        }
    }
}

/// One editable variable. `key` identifies the row's widgets while rows above it are added,
/// removed or the layout shifts, so typing keeps its focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: u64,
    pub name: String,
    pub value: String,
}

impl Row {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            key: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Rows with a name, in order: what the scope holds.
pub fn entries(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|r| !r.name.is_empty())
        .map(|r| (r.name.clone(), r.value.clone()))
        .collect()
}

pub fn rows(variables: &Variables) -> Vec<Row> {
    variables
        .iter()
        .map(|(name, value)| Row::new(name.clone(), value.clone()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Variable names must be unique within a scope (\"{0}\" repeats)")]
pub struct DuplicateName(pub String);

/// The rows as stored variables; names must be unique.
pub fn to_variables(rows: &[Row]) -> Result<Variables, DuplicateName> {
    let mut variables = Variables::new();
    for (name, value) in entries(rows) {
        if variables.contains_key(&name) {
            return Err(DuplicateName(name));
        }
        variables.insert(name, value);
    }
    Ok(variables)
}

#[cfg(test)]
mod tests {
    use poster_app::core::variables::{ActiveEnvironment, CollectionScope, VariableContext};

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
                    id: EnvironmentId(4),
                    name: "Dev".into(),
                    values: env,
                }),
            },
            Some(CollectionScope::Live {
                id: CollectionId(2),
                name: "API",
                values: &collection,
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
        let names: Vec<&str> = suggestions(&snapshot, "HO")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["host", "Host"]);
        let (_, definition) = suggestions(&snapshot, "host")[0];
        assert_eq!(definition.source.label(), "Dev");
        assert_eq!(suggestions(&snapshot, "").len(), 3);
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
            ("Effective · Environment · Dev".into(), "Empty value".into())
        );
        assert_eq!(
            lines[1],
            ("Overridden · Global · Global".into(), "g".into())
        );
        assert_eq!(
            describe(&snapshot.definitions["token"], true)[0].1,
            "••••••••"
        );
        let long = Definition {
            value: "x".repeat(5000),
            source: Source::Global,
        };
        assert!(
            describe(&[long], false)[0]
                .1
                .ends_with("… Value preview limited to 4096 characters")
        );
        assert_eq!(
            Scope::of(&Source::Collection {
                id: None,
                name: String::new()
            }),
            None
        );
        assert_eq!(
            Scope::of(&Source::Environment {
                id: EnvironmentId(1),
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
        let mut edited = rows(&variables);
        edited.push(Row::new("", ""));
        assert_eq!(to_variables(&edited).unwrap(), variables);
        edited.push(Row::new("a", "2"));
        assert_eq!(to_variables(&edited), Err(DuplicateName("a".into())));
    }
}
