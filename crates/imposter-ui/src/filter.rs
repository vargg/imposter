//! The filter language of the sidebar and History, after Yaak's.
//!
//! Words are combined with AND (written or not); `OR`, `NOT` or `-word`, `"phrases"` and
//! parentheses work as usual. `field:value` looks at one field (which fields depends on the
//! list); other words look at the list's main text. Matching ignores case and looks for the
//! text anywhere, except `method:`, which must be the whole method, and `status:`. Text that
//! does not parse is searched as typed, so the filter never errors.

use imposter_app::core::execution::{Outcome, Via};
use imposter_app::store::executions::ExecutionSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Method,
    Url,
    Name,
    /// `404`, `4xx`, `failed` or `cancelled`.
    Status,
    Environment,
    /// `cli` for runs from the command line, `app` for Send.
    Via,
}

/// The fields the sidebar knows.
pub const REQUEST_FIELDS: &[(&str, Field)] = &[
    ("method", Field::Method),
    ("url", Field::Url),
    ("name", Field::Name),
];

/// The fields History knows.
pub const HISTORY_FIELDS: &[(&str, Field)] = &[
    ("method", Field::Method),
    ("url", Field::Url),
    ("name", Field::Name),
    ("status", Field::Status),
    ("env", Field::Environment),
    ("via", Field::Via),
];

/// What a list item offers to the filter.
pub trait Searchable {
    /// Whether plain text matches the item's main text.
    fn has_text(&self, text: &str) -> bool;
    fn has_field(&self, field: Field, value: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    /// The empty filter.
    All,
    /// Text in the folder path, name or URL.
    Text(String),
    Field(Field, String),
    Not(Box<Query>),
    And(Box<Query>, Box<Query>),
    Or(Box<Query>, Box<Query>),
}

/// What a sidebar request offers to the filter.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// Folder names and the request name, `Folder / Sub / Name`.
    pub path: &'a str,
    pub name: &'a str,
    pub method: &'a str,
    pub url: &'a str,
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

impl Searchable for Candidate<'_> {
    /// The folder path and name, then the URL.
    fn has_text(&self, text: &str) -> bool {
        contains(self.path, text) || contains(self.url, text)
    }

    fn has_field(&self, field: Field, value: &str) -> bool {
        match field {
            Field::Method => self.method.eq_ignore_ascii_case(value),
            Field::Url => contains(self.url, value),
            Field::Name => contains(self.name, value),
            Field::Status | Field::Environment | Field::Via => false,
        }
    }
}

impl Searchable for ExecutionSummary {
    /// The request name, then the URL as sent.
    fn has_text(&self, text: &str) -> bool {
        contains(&self.name, text) || contains(&self.url, text)
    }

    fn has_field(&self, field: Field, value: &str) -> bool {
        match field {
            Field::Method => self.method.eq_ignore_ascii_case(value),
            Field::Url => contains(&self.url, value),
            Field::Name => contains(&self.name, value),
            Field::Environment => contains(&self.environment, value),
            Field::Status => status_matches(self.outcome, self.status, value),
            Field::Via => match value.trim().to_ascii_lowercase().as_str() {
                "cli" | "command_line" | "command-line" => self.via == Via::CommandLine,
                "app" => self.via == Via::App,
                _ => false,
            },
        }
    }
}

/// `404` is that status; `4xx` its class; `failed` and `cancelled` the outcomes without one.
fn status_matches(outcome: Outcome, status: u16, value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "failed" | "error" => return outcome == Outcome::TransportError,
        "cancelled" | "canceled" => return outcome == Outcome::Cancelled,
        _ => {}
    }
    if outcome != Outcome::Response {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.len() == 3 && bytes[0].is_ascii_digit() && &bytes[1..] == b"xx" {
        return u16::from(bytes[0] - b'0') == status / 100;
    }
    value.parse::<u16>() == Ok(status)
}

impl Query {
    /// A sidebar query.
    pub fn parse(text: &str) -> Self {
        Self::parse_with(text, REQUEST_FIELDS)
    }

    /// A query that knows `fields`; other `name:` words are text.
    pub fn parse_with(text: &str, fields: &'static [(&'static str, Field)]) -> Self {
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return Self::All;
        }
        let mut parser = Parser {
            tokens: &tokens,
            at: 0,
            fields,
        };
        match parser.or() {
            Some(query) if parser.at == tokens.len() => query,
            _ => Self::Text(text.trim().to_owned()),
        }
    }

    pub fn is_all(&self) -> bool {
        *self == Self::All
    }

    pub fn matches(&self, item: &impl Searchable) -> bool {
        match self {
            Self::All => true,
            Self::Text(text) => item.has_text(text),
            Self::Field(field, value) => item.has_field(*field, value),
            Self::Not(query) => !query.matches(item),
            Self::And(a, b) => a.matches(item) && b.matches(item),
            Self::Or(a, b) => a.matches(item) || b.matches(item),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Minus,
    Word(String),
    Phrase(String),
    /// `field:` with what follows it.
    Field(String, String),
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let phrase = |i: &mut usize| {
        // Opening quote at `*i`; an unclosed phrase runs to the end.
        *i += 1;
        let start = *i;
        while *i < chars.len() && chars[*i] != '"' {
            *i += 1;
        }
        let text: String = chars[start..*i].iter().collect();
        *i += 1;
        text
    };
    while i < chars.len() {
        match chars[i] {
            c if c.is_whitespace() => i += 1,
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '-' if chars.get(i + 1).is_some_and(|c| !c.is_whitespace()) => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '"' => tokens.push(Token::Phrase(phrase(&mut i))),
            _ => {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() && !"()\":".contains(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if chars.get(i) == Some(&':') {
                    i += 1;
                    let value = if chars.get(i) == Some(&'"') {
                        phrase(&mut i)
                    } else {
                        let start = i;
                        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != ')' {
                            i += 1;
                        }
                        chars[start..i].iter().collect()
                    };
                    tokens.push(Token::Field(word, value));
                } else if word.is_empty() {
                    // A stray quote or colon: keep it as text.
                    tokens.push(Token::Word(chars[i].to_string()));
                    i += 1;
                } else {
                    tokens.push(Token::Word(word));
                }
            }
        }
    }
    tokens
}

struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
    fields: &'static [(&'static str, Field)],
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn keyword(&self, keyword: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w == keyword)
    }

    fn or(&mut self) -> Option<Query> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            self.at += 1;
            left = Query::Or(Box::new(left), Box::new(self.and()?));
        }
        Some(left)
    }

    fn and(&mut self) -> Option<Query> {
        let mut left = self.unary()?;
        loop {
            if self.keyword("AND") {
                self.at += 1;
            } else if matches!(self.peek(), None | Some(Token::Close)) || self.keyword("OR") {
                return Some(left);
            }
            left = Query::And(Box::new(left), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Option<Query> {
        if self.keyword("NOT") || self.peek() == Some(&Token::Minus) {
            self.at += 1;
            return Some(Query::Not(Box::new(self.unary()?)));
        }
        let token = self.peek()?.clone();
        self.at += 1;
        Some(match token {
            Token::Open => {
                let inner = self.or()?;
                if self.peek() != Some(&Token::Close) {
                    return None;
                }
                self.at += 1;
                inner
            }
            Token::Close | Token::Minus => return None,
            Token::Word(word) | Token::Phrase(word) => Query::Text(word),
            Token::Field(field, value) => {
                let lower = field.to_lowercase();
                match self.fields.iter().find(|(name, _)| *name == lower) {
                    Some((_, known)) => Query::Field(*known, value),
                    None => Query::Text(format!("{field}:{value}")),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<'a>(path: &'a str, method: &'a str, url: &'a str) -> Candidate<'a> {
        Candidate {
            path,
            name: path.rsplit(" / ").next().unwrap_or(path),
            method,
            url,
        }
    }

    fn hits(query: &str) -> Vec<&'static str> {
        let items = [
            item("Users / List users", "GET", "{{base}}/users"),
            item("Users / Create user", "POST", "{{base}}/users"),
            item("Health", "GET", "{{base}}/health"),
            item("Orders / Place order", "POST", "{{base}}/store/orders"),
        ];
        let query = Query::parse(query);
        items
            .iter()
            .filter(|i| query.matches(*i))
            .map(|i| i.name)
            .collect()
    }

    #[test]
    fn words_fields_and_operators() {
        assert_eq!(hits("").len(), 4);
        assert_eq!(
            hits("user"),
            ["List users", "Create user"],
            "folder path counts"
        );
        assert_eq!(hits("method:post"), ["Create user", "Place order"]);
        assert_eq!(hits("method:pos"), Vec::<&str>::new(), "the whole method");
        assert_eq!(hits("url:store"), ["Place order"]);
        assert_eq!(hits("name:list"), ["List users"]);
        assert_eq!(hits("users -create"), ["List users"]);
        assert_eq!(hits("NOT users"), ["Health", "Place order"]);
        assert_eq!(hits("health OR order"), ["Health", "Place order"]);
        assert_eq!(hits("users AND method:GET"), ["List users"]);
        assert_eq!(hits("(health OR order) -method:POST"), ["Health"]);
        assert_eq!(hits("\"place order\""), ["Place order"]);
        assert_eq!(hits("name:\"list users\""), ["List users"]);
    }

    #[test]
    fn text_that_does_not_parse_is_searched_as_typed() {
        assert_eq!(Query::parse("(health"), Query::Text("(health".into()));
        assert_eq!(Query::parse("users )"), Query::Text("users )".into()));
        assert_eq!(Query::parse("OR"), Query::Text("OR".into()));
        assert_eq!(Query::parse("  "), Query::All);
        assert_eq!(
            Query::parse("foo:bar"),
            Query::Text("foo:bar".into()),
            "unknown fields are text"
        );
        assert_eq!(Query::parse("\"open"), Query::Text("open".into()));
        assert_eq!(hits("-"), Vec::<&str>::new());
        assert_eq!(
            Query::parse("status:404"),
            Query::Text("status:404".into()),
            "the sidebar has no status"
        );
    }

    fn run(
        name: &str,
        method: &str,
        url: &str,
        status: u16,
        outcome: Outcome,
        environment: &str,
    ) -> ExecutionSummary {
        ExecutionSummary {
            id: imposter_app::core::ids::ExecutionId(0),
            request_id: None,
            timestamp: imposter_app::core::execution::Timestamp(0),
            name: name.into(),
            method: method.into(),
            url: url.into(),
            status,
            outcome,
            elapsed: 0.0,
            environment: environment.into(),
            truncated: false,
            via: Via::App,
        }
    }

    #[test]
    fn history_knows_status_and_environment() {
        let mut runs = [
            run(
                "List users",
                "GET",
                "https://api/users",
                200,
                Outcome::Response,
                "Dev",
            ),
            run(
                "Create user",
                "POST",
                "https://api/users",
                422,
                Outcome::Response,
                "Prod",
            ),
            run(
                "Health",
                "GET",
                "https://api/health",
                0,
                Outcome::TransportError,
                "Dev",
            ),
            run(
                "Slow",
                "GET",
                "https://api/slow",
                0,
                Outcome::Cancelled,
                "Global",
            ),
            run(
                "Gone",
                "DELETE",
                "https://api/users/7",
                404,
                Outcome::Response,
                "Dev",
            ),
        ];
        runs[1].via = Via::CommandLine;
        let hits = |query: &str| -> Vec<&str> {
            let query = Query::parse_with(query, HISTORY_FIELDS);
            runs.iter()
                .filter(|r| query.matches(*r))
                .map(|r| r.name.as_str())
                .collect()
        };
        assert_eq!(hits("users"), ["List users", "Create user", "Gone"]);
        assert_eq!(hits("status:404"), ["Gone"]);
        assert_eq!(hits("status:4xx"), ["Create user", "Gone"]);
        assert_eq!(hits("status:2XX"), ["List users"]);
        assert_eq!(hits("status:failed"), ["Health"]);
        assert_eq!(hits("status:cancelled"), ["Slow"]);
        assert_eq!(hits("-status:2xx env:dev"), ["Health", "Gone"]);
        assert_eq!(hits("method:get -env:global"), ["List users", "Health"]);
        assert_eq!(
            hits("status:0"),
            Vec::<&str>::new(),
            "no response, no status"
        );
        assert_eq!(hits("via:cli"), [runs[1].name.as_str()]);
        assert_eq!(hits("via:app").len(), runs.len() - 1);
        assert_eq!(hits("via:robot"), Vec::<&str>::new());
        assert_eq!(
            hits("name:\"create user\" OR url:health"),
            ["Create user", "Health"]
        );
    }
}
