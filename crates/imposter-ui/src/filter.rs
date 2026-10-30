//! The sidebar filter: a small query language over requests, after Yaak's.
//!
//! Words are combined with AND (written or not); `OR`, `NOT` or `-word`, `"phrases"` and
//! parentheses work as usual. `method:`, `url:` and `name:` look at one field; other words
//! look at the request's folder path, name and URL. Matching ignores case and looks for the
//! text anywhere, except `method:`, which must be the whole method. Text that does not parse
//! is searched as typed, so the filter never errors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Method,
    Url,
    Name,
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

/// What a request offers to the filter.
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

impl Query {
    pub fn parse(text: &str) -> Self {
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return Self::All;
        }
        let mut parser = Parser {
            tokens: &tokens,
            at: 0,
        };
        match parser.or() {
            Some(query) if parser.at == tokens.len() => query,
            _ => Self::Text(text.trim().to_owned()),
        }
    }

    pub fn is_all(&self) -> bool {
        *self == Self::All
    }

    pub fn matches(&self, item: &Candidate<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Text(text) => contains(item.path, text) || contains(item.url, text),
            Self::Field(Field::Method, method) => item.method.eq_ignore_ascii_case(method),
            Self::Field(Field::Url, text) => contains(item.url, text),
            Self::Field(Field::Name, text) => contains(item.name, text),
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
            Token::Field(field, value) => match field.to_lowercase().as_str() {
                "method" => Query::Field(Field::Method, value),
                "url" => Query::Field(Field::Url, value),
                "name" => Query::Field(Field::Name, value),
                _ => Query::Text(format!("{field}:{value}")),
            },
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
            .filter(|i| query.matches(i))
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
    }
}
