//! Small line tokenizers for highlighting. They only color text: every byte of a line ends up
//! in exactly one span, in order, so the displayed text is never changed.

use std::ops::Range;

use crate::viewer::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Plain,
    Key,
    String,
    Number,
    Keyword,
    Punctuation,
    Tag,
    Attribute,
    Comment,
}

pub type Spans = Vec<(Range<usize>, Token)>;

pub fn line(language: Language, text: &str) -> Spans {
    match language {
        Language::Json => json(text),
        Language::Xml | Language::Html => markup(text),
        Language::Plain => vec![(0..text.len(), Token::Plain)],
    }
}

fn push(spans: &mut Spans, range: Range<usize>, token: Token) {
    if range.is_empty() {
        return;
    }
    if let Some((last, kind)) = spans.last_mut()
        && *kind == token
        && last.end == range.start
    {
        last.end = range.end;
        return;
    }
    spans.push((range, token));
}

pub fn json(text: &str) -> Spans {
    let bytes = text.as_bytes();
    let mut spans = Spans::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let token = match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                let rest = text[i..].trim_start();
                if rest.starts_with(':') {
                    Token::Key
                } else {
                    Token::String
                }
            }
            b'-' | b'0'..=b'9' => {
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E')
                {
                    i += 1;
                }
                Token::Number
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                i += 1;
                Token::Punctuation
            }
            c if c.is_ascii_alphabetic() => {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if matches!(&text[start..i], "true" | "false" | "null") {
                    Token::Keyword
                } else {
                    Token::Plain
                }
            }
            _ => {
                i += 1;
                while i < bytes.len()
                    && !matches!(
                        bytes[i],
                        b'"' | b'-' | b'0'..=b'9' | b'{' | b'}' | b'[' | b']' | b':' | b','
                    )
                    && !bytes[i].is_ascii_alphabetic()
                {
                    i += 1;
                }
                Token::Plain
            }
        };
        // Escapes can step past the end; keep ranges on character boundaries.
        let end = text.ceil_char_boundary(i.min(bytes.len()));
        i = end;
        push(&mut spans, start..end, token);
    }
    spans
}

pub fn markup(text: &str) -> Spans {
    let bytes = text.as_bytes();
    let mut spans = Spans::new();
    let mut i = 0;
    while i < bytes.len() {
        if text[i..].starts_with("<!--") {
            let end = text[i..].find("-->").map_or(bytes.len(), |e| i + e + 3);
            push(&mut spans, i..end, Token::Comment);
            i = end;
        } else if bytes[i] == b'<' {
            let close = text[i..].find('>').map_or(bytes.len(), |e| i + e + 1);
            let mut j = i + 1;
            while j < close && matches!(bytes[j], b'/' | b'?' | b'!') {
                j += 1;
            }
            let name_end = (j..close)
                .find(|&k| bytes[k].is_ascii_whitespace() || matches!(bytes[k], b'>' | b'/'))
                .unwrap_or(close);
            push(&mut spans, i..name_end, Token::Tag);
            let mut k = name_end;
            while k < close {
                match bytes[k] {
                    b'"' | b'\'' => {
                        let quote = bytes[k];
                        let end = (k + 1..close)
                            .find(|&m| bytes[m] == quote)
                            .map_or(close, |m| m + 1);
                        push(&mut spans, k..end, Token::String);
                        k = end;
                    }
                    b'>' | b'/' | b'?' => {
                        push(&mut spans, k..k + 1, Token::Tag);
                        k += 1;
                    }
                    c if c.is_ascii_whitespace() || c == b'=' => {
                        push(&mut spans, k..k + 1, Token::Plain);
                        k += 1;
                    }
                    _ => {
                        let end = (k..close)
                            .find(|&m| {
                                bytes[m].is_ascii_whitespace()
                                    || matches!(bytes[m], b'=' | b'>' | b'/' | b'"' | b'\'')
                            })
                            .unwrap_or(close);
                        let end = text.ceil_char_boundary(end.max(k + 1));
                        push(&mut spans, k..end, Token::Attribute);
                        k = end;
                    }
                }
            }
            i = close;
        } else {
            let end = text[i..].find('<').map_or(bytes.len(), |e| i + e);
            push(&mut spans, i..end, Token::Plain);
            i = end;
        }
    }
    spans
}

/// `Name: value` header lines.
pub fn header(text: &str) -> Spans {
    match text.find(':') {
        Some(colon) => {
            let mut spans = Spans::new();
            push(&mut spans, 0..colon, Token::Key);
            push(&mut spans, colon..text.len(), Token::Plain);
            spans
        }
        None => vec![(0..text.len(), Token::Plain)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str, spans: &Spans) -> Vec<(String, Token)> {
        spans
            .iter()
            .map(|(r, t)| (text[r.clone()].to_owned(), *t))
            .collect()
    }

    fn covers(text: &str, spans: &Spans) {
        let mut at = 0;
        for (range, _) in spans {
            assert_eq!(range.start, at, "gap or overlap in {text:?}");
            at = range.end;
        }
        assert_eq!(at, text.len(), "{text:?} not fully covered");
    }

    #[test]
    fn json_tells_keys_from_string_values() {
        let text = r#"  "name": "Ann", "n": -1.5e3, "ok": true, "x": null"#;
        let spans = json(text);
        covers(text, &spans);
        let found = kinds(text, &spans);
        assert!(found.contains(&("\"name\"".into(), Token::Key)));
        assert!(found.contains(&("\"Ann\"".into(), Token::String)));
        assert!(found.contains(&("-1.5e3".into(), Token::Number)));
        assert!(found.contains(&("true".into(), Token::Keyword)));
        assert!(found.contains(&("null".into(), Token::Keyword)));
    }

    #[test]
    fn markup_marks_tags_attributes_strings_and_comments() {
        let text = r#"<a href="x" id='y'>text<!-- note --></a>"#;
        let spans = markup(text);
        covers(text, &spans);
        let found = kinds(text, &spans);
        assert!(found.contains(&("<a".into(), Token::Tag)));
        assert!(found.contains(&("href".into(), Token::Attribute)));
        assert!(found.contains(&("\"x\"".into(), Token::String)));
        assert!(found.contains(&("<!-- note -->".into(), Token::Comment)));
        assert!(found.contains(&("text".into(), Token::Plain)));
    }

    #[test]
    fn every_tokenizer_covers_arbitrary_input_exactly() {
        for text in [
            "",
            "\"unterminated",
            "\"esc\\",
            "ünï \"ç\": [1,2]",
            "<",
            "<a b=\"c",
            "<ü attr=é>",
            "plain",
            "a: b: c",
        ] {
            covers(text, &json(text));
            covers(text, &markup(text));
            covers(text, &header(text));
            covers(text, &line(Language::Plain, text));
        }
    }

    #[test]
    fn header_names_are_keys() {
        let spans = header("content-type: text/plain");
        assert_eq!(
            kinds("content-type: text/plain", &spans)[0],
            ("content-type".into(), Token::Key)
        );
    }
}
