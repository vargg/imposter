//! Variable presentation helpers: which `{{tokens}}` to mark, what to suggest while typing, and
//! the Variables screen's editable scope. Pure functions; widgets only draw their results.

use std::ops::Range;

use imposter_app::core::generate;
use imposter_app::core::model::Variables;
pub use imposter_app::core::variables::Scope;
use imposter_app::core::variables::{Definition, Snapshot, Source};
use imposter_app::store::scopes::{CaptureSlot, CaptureSlots, EditableScope};

/// A `{{name}}` token as the editors mark it: no braces or newline inside, not empty
/// (`\{\{([^{}\n]+)\}\}`, stricter than resolution, so half-typed text is not marked).
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
        Source::Run => "This run",
        Source::Environment { .. } => "Environment",
        Source::Secret { .. } => "Secret",
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
        Source::Run => "an earlier request of this run".to_owned(),
        Source::Secret { environment: None } => "the global secrets".to_owned(),
        Source::Secret {
            environment: Some((_, name)),
        } => format!("the {name} secrets"),
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

fn next_row_key() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// An editable secret, with stable widget identity across row moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: u64,
    pub name: String,
    pub value: String,
}

impl Row {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: next_row_key(),
            name: name.into(),
            value: value.into(),
        }
    }
}

pub type SecretEntry = (String, String);

pub fn secret_entries(rows: &[Row]) -> Vec<SecretEntry> {
    rows.iter()
        .filter(|row| !row.name.is_empty() || !row.value.is_empty())
        .map(|row| (row.name.clone(), row.value.clone()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Ordinary(usize),
    Capture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Ordinary {
    values: Vec<String>,
    selected: usize,
}

/// A variable's ordinary choices, its latest capture, and a separate input draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableRow {
    pub key: u64,
    pub name: String,
    pub draft: String,
    ordinary: Option<Ordinary>,
    capture: Option<CaptureSlot>,
}

impl VariableRow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            key: next_row_key(),
            name: name.into(),
            draft: String::new(),
            ordinary: None,
            capture: None,
        }
    }

    pub fn from_stored(
        name: impl Into<String>,
        current: Option<String>,
        mut values: Vec<String>,
        capture: Option<CaptureSlot>,
    ) -> Self {
        let ordinary = current.map(|current| {
            let selected = values
                .iter()
                .position(|v| *v == current)
                .unwrap_or_else(|| {
                    // Legacy scalars may be absent from the list of alternatives.
                    values.insert(0, current);
                    0
                });
            Ordinary { values, selected }
        });
        Self {
            ordinary,
            capture,
            ..Self::new(name)
        }
    }

    pub fn blank(&self) -> bool {
        self.name.is_empty() && self.draft.is_empty() && self.current().is_none()
    }

    pub fn values(&self) -> &[String] {
        self.ordinary
            .as_ref()
            .map_or(&[], |ordinary| &ordinary.values)
    }

    /// Equal values share one chip even if they arrived by different routes.
    pub fn distinct_capture(&self) -> Option<&str> {
        let capture = self.capture.as_ref()?;
        (!self.values().contains(&capture.value)).then_some(capture.value.as_str())
    }

    pub fn current(&self) -> Option<&str> {
        if let Some(capture) = &self.capture
            && capture.active
        {
            return Some(&capture.value);
        }
        self.ordinary
            .as_ref()
            .map(|ordinary| ordinary.values[ordinary.selected].as_str())
    }

    pub fn select(&mut self, choice: Choice) {
        if let Some(capture) = &mut self.capture {
            capture.active = choice == Choice::Capture;
        }
        if let Choice::Ordinary(index) = choice
            && let Some(ordinary) = &mut self.ordinary
        {
            ordinary.selected = index;
        }
    }

    fn keep(&mut self, value: String) -> usize {
        let ordinary = self.ordinary.get_or_insert_with(|| Ordinary {
            values: Vec::new(),
            selected: 0,
        });
        ordinary
            .values
            .iter()
            .position(|v| *v == value)
            .unwrap_or_else(|| {
                let index = ordinary.values.len();
                ordinary.values.push(value);
                index
            })
    }

    pub fn add_draft(&mut self) {
        let value = std::mem::take(&mut self.draft);
        let index = self.keep(value);
        self.select(Choice::Ordinary(index));
    }

    pub fn keep_capture(&mut self) {
        if let Some(capture) = &self.capture {
            let active = capture.active;
            let index = self.keep(capture.value.clone());
            if active {
                self.select(Choice::Ordinary(index));
            }
        }
    }

    pub fn can_remove(&self) -> bool {
        self.values().len() + usize::from(self.distinct_capture().is_some()) > 1
    }

    pub fn remove(&mut self, choice: Choice) {
        if !self.can_remove() {
            return;
        }
        match choice {
            Choice::Capture => {
                if self.capture.take().is_some_and(|capture| capture.active)
                    && let Some(ordinary) = &mut self.ordinary
                {
                    ordinary.selected = ordinary.values.len() - 1;
                }
            }
            Choice::Ordinary(index) => {
                let was_active = self.current() == Some(self.values()[index].as_str());
                let Some(ordinary) = &mut self.ordinary else {
                    return;
                };
                ordinary.values.remove(index);
                if ordinary.values.is_empty() {
                    self.ordinary = None;
                    self.select(Choice::Capture);
                } else {
                    let selected = if was_active {
                        index.min(ordinary.values.len() - 1)
                    } else {
                        ordinary
                            .selected
                            .saturating_sub(usize::from(index < ordinary.selected))
                            .min(ordinary.values.len() - 1)
                    };
                    ordinary.selected = selected;
                    if was_active {
                        // The capture follows the ordinary chips in display order.
                        if index >= self.values().len() && self.distinct_capture().is_some() {
                            self.select(Choice::Capture);
                        } else {
                            self.select(Choice::Ordinary(selected));
                        }
                    }
                }
            }
        }
    }
}

pub fn variable_rows(scope: EditableScope) -> Vec<VariableRow> {
    let EditableScope {
        variables,
        saved,
        mut captures,
    } = scope;
    let mut rows: Vec<_> = variables
        .into_iter()
        .map(|(name, current)| {
            let list = saved.values(&name);
            let capture = captures.shift_remove(&name);
            VariableRow::from_stored(name, Some(current), list, capture)
        })
        .collect();
    rows.extend(
        captures
            .into_iter()
            .map(|(name, capture)| VariableRow::from_stored(name, None, Vec::new(), Some(capture))),
    );
    rows
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VariableError {
    #[error("Variable names must be unique within a scope (\\\"{0}\\\" repeats)")]
    DuplicateName(String),
    #[error("Name the variable before saving its values")]
    UnnamedValues,
}

pub fn capture_slots(rows: &[VariableRow]) -> CaptureSlots {
    rows.iter()
        .filter_map(|row| {
            row.capture
                .clone()
                .map(|capture| (row.name.clone(), capture))
        })
        .collect()
}

/// Save and Enter use the same draft commit. The caller supplies a copy until validation succeeds.
pub fn to_scope(rows: &mut [VariableRow]) -> Result<EditableScope, VariableError> {
    let mut scope = EditableScope::default();
    for row in rows.iter_mut().filter(|row| !row.blank()) {
        if row.name.is_empty() {
            return Err(VariableError::UnnamedValues);
        }
        if scope.variables.contains_key(&row.name) || scope.captures.contains_key(&row.name) {
            return Err(VariableError::DuplicateName(row.name.clone()));
        }
        if !row.draft.is_empty() || row.current().is_none() {
            row.add_draft();
        }
        if let Some(ordinary) = &row.ordinary {
            scope
                .variables
                .insert(row.name.clone(), ordinary.values[ordinary.selected].clone());
            scope.saved.set(&row.name, ordinary.values.clone());
        }
        if let Some(capture) = &row.capture {
            scope.captures.insert(row.name.clone(), capture.clone());
        }
    }
    Ok(scope)
}
/// The secret rows as their scope stores them: names unique and valid, values long enough
/// to remove from output safely (`redact::MIN_SECRET`).
pub fn to_secrets(rows: &[Row]) -> Result<Variables, String> {
    let min = imposter_app::core::redact::MIN_SECRET;
    let mut secrets = Variables::new();
    for (name, value) in secret_entries(rows) {
        if name.is_empty() {
            return Err("Name the secret before saving its value".to_owned());
        }
        if name.contains(['{', '}', '\n']) {
            return Err(format!("Secret \"{name}\": names cannot contain braces"));
        }
        if secrets.contains_key(&name) {
            return Err(format!(
                "Secret names must be unique within a scope (\"{name}\" repeats)"
            ));
        }
        if value.chars().count() < min {
            return Err(format!(
                "Secret \"{name}\" needs at least {min} characters, so it can be removed from output reliably"
            ));
        }
        secrets.insert(name, value);
    }
    Ok(secrets)
}

#[cfg(test)]
mod tests {
    use imposter_app::core::ids::{CollectionId, EnvironmentId};
    use imposter_app::core::model::SavedValues;
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
                    secrets: Variables::new(),
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
    fn legacy_scalars_become_chips_and_saved_order_is_preserved() {
        let mut scope = EditableScope::default();
        scope.variables.insert("a".to_owned(), "1".to_owned());
        scope.variables.insert("b".to_owned(), String::new());
        scope.saved.set("a", ["9".into(), "1".into()]);
        let mut rows = variable_rows(scope);
        assert_eq!(rows[0].values(), ["9", "1"]);
        assert_eq!(rows[0].current(), Some("1"));
        assert_eq!(rows[1].values(), [""]);
        rows.push(VariableRow::new(""));
        let saved = to_scope(&mut rows).unwrap();
        assert_eq!(saved.saved.values("a"), ["9", "1"]);
        assert_eq!(saved.saved.values("b"), [""]);
        rows.push(VariableRow::new("a"));
        assert_eq!(
            to_scope(&mut rows),
            Err(VariableError::DuplicateName("a".into()))
        );
    }

    #[test]
    fn removing_the_active_capture_selects_the_last_ordinary_choice() {
        let mut row = VariableRow::from_stored(
            "trace",
            Some("1".into()),
            vec!["1".into(), "2".into()],
            Some(CaptureSlot {
                value: "3".into(),
                active: true,
            }),
        );
        row.remove(Choice::Capture);
        assert_eq!(row.current(), Some("2"));
        assert!(row.distinct_capture().is_none());
        row.remove(Choice::Ordinary(1));
        assert_eq!(row.current(), Some("1"));
        assert!(!row.can_remove());
    }
}
