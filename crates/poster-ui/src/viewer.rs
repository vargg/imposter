//! What the response viewer shows: the bounded preview text, its status line, display rows
//! and search. Pure functions; the widget only draws their results.

use std::ops::Range;

use poster_app::core::execution::{Outcome, Response};

use crate::theme::Tone;

/// Only this much of a body is decoded, formatted and shown.
pub const PREVIEW_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Json,
    Xml,
    Html,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub text: String,
    pub language: Language,
    /// The body was truncated at capture, or the preview is shorter than the body.
    pub limited: bool,
}

pub fn content_type(headers: &[(String, String)]) -> &str {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map_or("", |(_, v)| v.as_str())
}

fn language(content_type: &str) -> Language {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if essence.contains("json") {
        Language::Json
    } else if essence.contains("html") {
        Language::Html
    } else if essence.contains("xml") {
        Language::Xml
    } else {
        Language::Plain
    }
}

/// Decode as UTF-8, or Latin-1 when that fails, as the Python app did. When the preview limit
/// cut a character in two, the incomplete end is dropped instead of falling back.
fn decode(bytes: &[u8], cut: bool) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(error) if cut && error.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()
        }
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// Re-indent valid JSON with two spaces, keeping every token exactly as received (numbers are
/// never rounded, escapes stay as sent).
pub fn pretty_json(text: &str) -> Option<String> {
    serde_json::from_str::<serde::de::IgnoredAny>(text).ok()?;
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut depth = 0_usize;
    let mut chars = text.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n("  ", depth));
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '{' | '[' => {
                out.push(c);
                let close = if c == '{' { '}' } else { ']' };
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                if chars.peek() == Some(&close) {
                    chars.next();
                    out.push(close);
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

pub fn preview(response: &Response) -> Preview {
    let content_type = content_type(&response.headers);
    let mut language = language(content_type);
    let shown = &response.body[..response.body.len().min(PREVIEW_LIMIT)];
    let mut text = decode(shown, response.body.len() > PREVIEW_LIMIT);
    let mut limited = response.truncated;
    if language == Language::Json
        && response.body.len() <= PREVIEW_LIMIT
        && let Some(formatted) = pretty_json(&text)
    {
        if formatted.len() > PREVIEW_LIMIT {
            limited = true;
            text = formatted[..formatted.floor_char_boundary(PREVIEW_LIMIT)].to_owned();
        } else {
            text = formatted;
        }
    }
    if response.body.len() > PREVIEW_LIMIT {
        text.push_str("\n… Preview limited to 256 KiB");
        language = Language::Plain;
        limited = true;
    }
    Preview {
        text,
        language,
        limited,
    }
}

pub fn headers_text(response: &Response) -> String {
    response
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The status line: text, tone and tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub tone: Tone,
    pub detail: String,
}

pub fn status(response: &Response) -> Status {
    match response.outcome {
        Outcome::Cancelled => Status {
            text: "Cancelled".into(),
            tone: Tone::Warning,
            detail: "Execution cancelled".into(),
        },
        Outcome::TransportError => {
            let reason = if response.reason.is_empty() {
                response.error_kind.clone()
            } else {
                response.reason.clone()
            };
            Status {
                text: format!("Transport failure: {reason}"),
                tone: Tone::Error,
                detail: reason,
            }
        }
        Outcome::Response => Status {
            text: format!("Status: {}", response.status),
            tone: match response.status {
                200..=299 => Tone::Ok,
                300..=399 => Tone::Warning,
                _ => Tone::Error,
            },
            detail: format!("{} {}", response.status, response.reason),
        },
    }
}

/// A history timestamp in the system time zone: "24 Oct 2026, 21:05:35".
pub fn local_time(timestamp: poster_app::core::execution::Timestamp) -> String {
    jiff::Timestamp::from_millisecond(timestamp.0).map_or_else(
        |_| timestamp.to_rfc3339(),
        |instant| {
            instant
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%d %b %Y, %H:%M:%S")
                .to_string()
        },
    )
}

pub fn format_duration(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("Time: {:.0} ms", seconds * 1000.0)
    } else {
        format!("Time: {seconds:.2}s")
    }
}

pub fn size_text(response: &Response) -> String {
    if response.truncated {
        format!("Captured: {} bytes (truncated)", response.body.len())
    } else {
        format!("Size: {} bytes", response.body.len())
    }
}

/// The notice under the status line, joining what the user should know.
pub fn notice(response: &Response, preview: &Preview) -> String {
    let mut parts: Vec<&str> = Vec::new();
    match response.outcome {
        Outcome::Cancelled => parts.push("Execution cancelled"),
        Outcome::Response if response.body.is_empty() => parts.push("Response body is empty"),
        Outcome::Response if preview.limited => parts.push("Response truncated or preview limited"),
        _ => {}
    }
    if !response.history_error.is_empty() {
        parts.push(&response.history_error);
    }
    parts.join(" · ")
}

/// Display rows: each line, split into chunks of at most `columns` characters. Without wrapping,
/// very long lines are still chunked so a single row never costs a whole-body layout.
pub fn rows(text: &str, columns: Option<usize>) -> Vec<Range<usize>> {
    const UNWRAPPED_CHUNK: usize = 4096;
    let width = columns.unwrap_or(UNWRAPPED_CHUNK).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.len();
        let mut chunk_start = start;
        let mut count = 0;
        for (offset, _) in line.char_indices() {
            if count == width {
                out.push(chunk_start..start + offset);
                chunk_start = start + offset;
                count = 0;
            }
            count += 1;
        }
        out.push(chunk_start..end);
        start = end + 1;
    }
    out
}

/// Case-insensitive, non-overlapping matches of `needle` in `haystack`, as byte ranges.
pub fn find_all(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    let folded: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if folded.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut search_from = 0;
    while search_from < haystack.len() {
        let mut found = None;
        for (start, _) in haystack[search_from..].char_indices() {
            let start = search_from + start;
            let mut expected = folded.iter();
            let mut pending: Vec<char> = Vec::new();
            for (offset, c) in haystack[start..].char_indices() {
                pending.extend(c.to_lowercase());
                let end = start + offset + c.len_utf8();
                let mut matched = true;
                while let Some(&p) = pending.first() {
                    match expected.next() {
                        Some(&e) if e == p => {
                            pending.remove(0);
                        }
                        _ => {
                            matched = false;
                            break;
                        }
                    }
                }
                if !matched {
                    break;
                }
                if expected.len() == 0 {
                    found = Some(start..end);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some(range) = found else { break };
        search_from = range.end;
        out.push(range);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &[u8], content_type: &str) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            headers: vec![("content-type".into(), content_type.into())],
            body: body.to_vec(),
            ..Response::default()
        }
    }

    #[test]
    fn json_is_reindented_keeping_tokens_verbatim() {
        let pretty = pretty_json(
            r#"{"a":[1,2.50,{}],"big":123456789012345678901234,"s":"x\"y\u00e9","e":[ ]}"#,
        )
        .unwrap();
        assert_eq!(
            pretty,
            "{\n  \"a\": [\n    1,\n    2.50,\n    {}\n  ],\n  \"big\": 123456789012345678901234,\n  \"s\": \"x\\\"y\\u00e9\",\n  \"e\": []\n}"
        );
        assert_eq!(pretty_json("{oops"), None);
    }

    #[test]
    fn previews_format_json_and_bound_large_bodies() {
        let small = preview(&response(br#"{"k":1}"#, "application/json; charset=utf-8"));
        assert_eq!(
            (small.text.as_str(), small.language, small.limited),
            ("{\n  \"k\": 1\n}", Language::Json, false)
        );

        let invalid = preview(&response(b"{not json", "application/json"));
        assert_eq!(invalid.text, "{not json");

        let big = vec![b'a'; PREVIEW_LIMIT + 10];
        let large = preview(&response(&big, "application/json"));
        assert!(large.text.ends_with("\n… Preview limited to 256 KiB"));
        assert_eq!((large.language, large.limited), (Language::Plain, true));
        assert_eq!(
            large.text.len(),
            PREVIEW_LIMIT + "\n… Preview limited to 256 KiB".len()
        );
    }

    #[test]
    fn non_utf8_bodies_fall_back_to_latin1_and_cut_characters_are_dropped() {
        assert_eq!(
            preview(&response(&[0x63, 0x61, 0x66, 0xe9], "text/plain")).text,
            "café"
        );
        let mut cut = vec![b'a'; PREVIEW_LIMIT - 1];
        cut.extend("é".as_bytes());
        cut.extend(b"more");
        assert!(
            preview(&response(&cut, "text/plain"))
                .text
                .starts_with(&"a".repeat(PREVIEW_LIMIT - 1))
        );
    }

    #[test]
    fn languages_follow_the_content_type() {
        assert_eq!(language("text/html; charset=utf-8"), Language::Html);
        assert_eq!(language("application/xml"), Language::Xml);
        assert_eq!(language("application/problem+json"), Language::Json);
        assert_eq!(language(""), Language::Plain);
        assert_eq!(
            content_type(&[("Content-Type".into(), "x/y".into())]),
            "x/y"
        );
    }

    #[test]
    fn status_tones_and_notices() {
        let ok = response(b"x", "text/plain");
        assert_eq!(
            status(&ok),
            Status {
                text: "Status: 200".into(),
                tone: Tone::Ok,
                detail: "200 OK".into()
            }
        );
        assert_eq!(
            status(&Response {
                status: 302,
                ..ok.clone()
            })
            .tone,
            Tone::Warning
        );
        assert_eq!(
            status(&Response {
                status: 503,
                ..ok.clone()
            })
            .tone,
            Tone::Error
        );
        let failed = Response::failure(Outcome::TransportError, "ConnectError", "refused");
        assert_eq!(status(&failed).text, "Transport failure: refused");
        let cancelled = Response::failure(Outcome::Cancelled, "Cancelled", "");
        assert_eq!(status(&cancelled).tone, Tone::Warning);

        let empty = Response {
            history_error: "history down".into(),
            ..response(b"", "text/plain")
        };
        assert_eq!(
            notice(&empty, &preview(&empty)),
            "Response body is empty · history down"
        );
        let truncated = Response {
            truncated: true,
            ..ok
        };
        assert_eq!(
            notice(&truncated, &preview(&truncated)),
            "Response truncated or preview limited"
        );
        assert_eq!(size_text(&truncated), "Captured: 1 bytes (truncated)");
        assert_eq!(format_duration(0.1234), "Time: 123 ms");
        assert_eq!(format_duration(2.5), "Time: 2.50s");
        assert_eq!(headers_text(&response(b"", "a/b")), "content-type: a/b");
    }

    #[test]
    fn local_times_use_a_readable_format() {
        let text = local_time(poster_app::core::execution::Timestamp(1_793_000_000_000));
        assert!(text.contains("2026"), "{text}");
        assert_eq!(text.len(), "26 Oct 2026, 07:33:20".len());
        assert_eq!(
            local_time(poster_app::core::execution::Timestamp(i64::MAX)),
            poster_app::core::execution::Timestamp(i64::MAX).to_rfc3339()
        );
    }

    #[test]
    fn rows_split_lines_and_long_lines() {
        let text = "ab\ncdefg\n\nh";
        let slices: Vec<&str> = rows(text, Some(2)).into_iter().map(|r| &text[r]).collect();
        assert_eq!(slices, ["ab", "cd", "ef", "g", "", "h"]);
        let unicode = "ééé";
        let chunks: Vec<&str> = rows(unicode, Some(2))
            .into_iter()
            .map(|r| &unicode[r])
            .collect();
        assert_eq!(chunks, ["éé", "é"]);
        let long = "x".repeat(10_000);
        assert_eq!(rows(&long, None).len(), 3);
    }

    #[test]
    fn search_is_case_insensitive_and_returns_byte_ranges() {
        let text = "Item one, ITEM two, élan ÉLAN";
        let found: Vec<&str> = find_all(text, "item")
            .into_iter()
            .map(|r| &text[r])
            .collect();
        assert_eq!(found, ["Item", "ITEM"]);
        let accented: Vec<&str> = find_all(text, "Élan")
            .into_iter()
            .map(|r| &text[r])
            .collect();
        assert_eq!(accented, ["élan", "ÉLAN"]);
        assert_eq!(find_all(text, ""), Vec::<Range<usize>>::new());
        assert_eq!(find_all("aaaa", "aa"), [0..2, 2..4]);
        assert_eq!(find_all("abc", "abcd"), Vec::<Range<usize>>::new());
    }
}
