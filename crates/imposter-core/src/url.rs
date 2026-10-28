//! URL and query handling with the exact semantics of Python's `urllib.parse`
//! (`urlsplit`, `quote_plus`, `urlencode`, `parse_qsl`), which the Python app used to build
//! the request line. Parity is pinned by `tests/fixtures/parity/prepare.json`.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UrlParts {
    /// Lowercased.
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
}

/// Python's `urlsplit`: strips leading C0 controls and spaces, removes tabs and newlines, and
/// treats `name:` as a scheme only when it starts with a letter.
pub fn split(url: &str) -> UrlParts {
    let cleaned: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = cleaned.as_str();
    let mut scheme = String::new();
    if let Some(colon) = rest.find(':') {
        let candidate = &rest[..colon];
        if colon > 0
            && candidate.starts_with(|c: char| c.is_ascii_alphabetic())
            && candidate.chars().all(is_scheme_char)
        {
            scheme = candidate.to_ascii_lowercase();
            rest = &rest[colon + 1..];
        }
    }
    let mut netloc = "";
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = &after[..end];
        rest = &after[end..];
    }
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    UrlParts {
        scheme,
        netloc: netloc.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    }
}

/// Python's `urlunsplit` for the request line: no fragment.
pub fn join(parts: &UrlParts) -> String {
    let mut url = String::new();
    if !parts.scheme.is_empty() {
        url.push_str(&parts.scheme);
        url.push(':');
    }
    if !parts.netloc.is_empty() {
        url.push_str("//");
        url.push_str(&parts.netloc);
        if !parts.path.is_empty() && !parts.path.starts_with('/') {
            url.push('/');
        }
    }
    url.push_str(&parts.path);
    if !parts.query.is_empty() {
        url.push('?');
        url.push_str(&parts.query);
    }
    url
}

/// Like [`join`], keeping the fragment: for rebuilding the URL the user edits.
pub fn join_with_fragment(parts: &UrlParts) -> String {
    let mut url = join(parts);
    if !parts.fragment.is_empty() {
        url.push('#');
        url.push_str(&parts.fragment);
    }
    url
}

/// Python's `quote_plus(value, safe="")`: letters, digits and `_.-~` stay, space becomes `+`.
pub fn quote_plus(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                // Writing to a String cannot fail.
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Python's `urlencode` over ordered pairs.
pub fn encode_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    pairs
        .into_iter()
        .map(|(name, value)| format!("{}={}", quote_plus(name), quote_plus(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Python's `unquote_plus`: `+` is a space, valid `%XX` escapes are decoded, invalid escapes
/// stay as typed, and invalid UTF-8 becomes U+FFFD.
pub fn unquote_plus(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit) =>
            {
                out.push(hex(bytes[i + 1]) << 4 | hex(bytes[i + 2]));
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => digit - b'A' + 10,
    }
}

/// Python's `parse_qsl(query, keep_blank_values=True)`: pairs split on `&`, a missing `=`
/// means an empty value, empty segments are skipped.
pub fn parse_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let (name, value) = segment.split_once('=').unwrap_or((segment, ""));
            (unquote_plus(name), unquote_plus(value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_matches_python() {
        let parts = split("HTTP://Example.com:80/a b?q=1#frag");
        assert_eq!(
            parts,
            UrlParts {
                scheme: "http".into(),
                netloc: "Example.com:80".into(),
                path: "/a b".into(),
                query: "q=1".into(),
                fragment: "frag".into(),
            }
        );
        assert_eq!(split("localhost:8080/x").scheme, "localhost");
        assert_eq!(split("http:///x").netloc, "");
        assert_eq!(split("//h/x").netloc, "h");
        assert_eq!(split("1http://h").scheme, "");
        assert_eq!(split("  \u{1}http://h\t/p\n").path, "/p");
    }

    #[test]
    fn join_rebuilds_without_fragment() {
        let parts = UrlParts {
            scheme: "http".into(),
            netloc: "h".into(),
            path: "x".into(),
            query: "a=1".into(),
            fragment: "f".into(),
        };
        assert_eq!(join(&parts), "http://h/x?a=1");
        assert_eq!(join_with_fragment(&parts), "http://h/x?a=1#f");
        assert_eq!(join_with_fragment(&split("example.com/x")), "example.com/x");
    }

    #[test]
    fn quoting_matches_python() {
        assert_eq!(
            encode_pairs([("a b", "~*!'()/é"), ("x", "")]),
            "a+b=~%2A%21%27%28%29%2F%C3%A9&x="
        );
    }

    #[test]
    fn parsing_matches_python() {
        assert_eq!(
            parse_pairs("a=1&&b&c=%zz&d=%ff+x;e=1&t=%4"),
            [
                ("a".into(), "1".into()),
                ("b".into(), String::new()),
                ("c".into(), "%zz".into()),
                ("d".into(), "\u{fffd} x;e=1".into()),
                ("t".into(), "%4".into()),
            ]
        );
    }
}
