//! Server-sent events (`text/event-stream`): a parser that follows the WHATWG HTML
//! standard's rules for lines, fields and dispatch, fed chunks as they arrive (a chunk may
//! end mid-line or mid-character), and the stream a response keeps: its events and how it
//! ended. Reading the stream is the transport's; this module does no IO.

use serde::{Deserialize, Serialize};

/// The most events a stream keeps; later ones are counted, not kept.
pub const MAX_EVENTS: usize = 10_000;

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Milliseconds after the response's head arrived.
    pub at_ms: u64,
    /// The `event` field, `message` when the server named none.
    pub event: String,
    /// The last event ID in effect when it was dispatched; empty when none was set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The `data` lines joined with line feeds.
    pub data: String,
}

/// How a stream ended. Closed and Limit are normal ends; the others never pass a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEnd {
    /// The server closed it.
    Closed,
    /// The request's own listening limit (seconds or events) was reached.
    Limit,
    /// The body reached the capture limit; what came later is unknown.
    Cut,
    /// Stopped by the person or the run.
    Cancelled,
    /// Nothing arrived for the timeout.
    TimedOut,
    /// The connection broke.
    Failed,
}

impl StreamEnd {
    /// Whether the stream ended as the request meant it to: what arrived can be checked.
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Closed | Self::Limit)
    }

    /// In words, for statuses and check results ("cut at the size limit").
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Closed => "closed by the server",
            Self::Limit => "stopped at its listening limit",
            Self::Cut => "cut at the size limit",
            Self::Cancelled => "stopped",
            Self::TimedOut => "timed out waiting for data",
            Self::Failed => "broken off",
        }
    }
}

/// The events of a `text/event-stream` response and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    pub end: StreamEnd,
    pub events: Vec<Event>,
    /// Events received past [`MAX_EVENTS`], counted only.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_events: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes fields by reference
const fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl Stream {
    /// Every event received, kept or counted.
    pub fn received(&self) -> u64 {
        self.events.len() as u64 + self.more_events
    }
}

/// Whether a `Content-Type` value names an event stream.
pub fn is_event_stream(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/event-stream"))
}

/// The parsing state between chunks.
#[derive(Debug, Default)]
pub struct Parser {
    /// The bytes of a line not yet ended.
    line: Vec<u8>,
    /// The last chunk ended with CR, so a LF starting the next one ends nothing.
    after_cr: bool,
    /// The byte order mark at the very start was looked for.
    started: bool,
    data: String,
    event: String,
    last_id: String,
}

impl Parser {
    /// Feed the next chunk; the events it completed, stamped `at_ms`.
    pub fn feed(&mut self, mut chunk: &[u8], at_ms: u64) -> Vec<Event> {
        let mut events = Vec::new();
        if chunk.is_empty() {
            return events;
        }
        if !self.started {
            // A byte order mark may still be partial; three bytes decide it.
            self.line.extend_from_slice(chunk);
            if self.line.len() < 3 && b"\xEF\xBB\xBF".starts_with(&self.line) {
                return events;
            }
            self.started = true;
            let held = std::mem::take(&mut self.line);
            let held = held.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&held);
            return self.feed_lines(held, at_ms);
        }
        if self.after_cr && chunk.first() == Some(&b'\n') {
            chunk = &chunk[1..];
        }
        self.after_cr = false;
        events.extend(self.feed_lines(chunk, at_ms));
        events
    }

    fn feed_lines(&mut self, chunk: &[u8], at_ms: u64) -> Vec<Event> {
        let mut events = Vec::new();
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|b| *b == b'\n' || *b == b'\r') {
            self.line.extend_from_slice(&rest[..end]);
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
            events.extend(self.line_done(&line, at_ms));
            let crlf = rest[end] == b'\r' && rest.get(end + 1) == Some(&b'\n');
            if rest[end] == b'\r' && end + 1 == rest.len() {
                self.after_cr = true;
            }
            rest = &rest[end + if crlf { 2 } else { 1 }..];
        }
        self.line.extend_from_slice(rest);
        events
    }

    fn line_done(&mut self, line: &str, at_ms: u64) -> Option<Event> {
        if line.is_empty() {
            return self.dispatch(at_ms);
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => value.clone_into(&mut self.event),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "id" if !value.contains('\0') => value.clone_into(&mut self.last_id),
            // `retry` asks a browser to wait before reconnecting; nothing reconnects here.
            _ => {}
        }
        None
    }

    fn dispatch(&mut self, at_ms: u64) -> Option<Event> {
        let event = std::mem::take(&mut self.event);
        if self.data.is_empty() {
            return None;
        }
        let mut data = std::mem::take(&mut self.data);
        data.pop();
        Some(Event {
            at_ms,
            event: if event.is_empty() {
                "message".to_owned()
            } else {
                event
            },
            id: self.last_id.clone(),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(chunks: &[&[u8]]) -> Vec<(String, String, String)> {
        let mut parser = Parser::default();
        chunks
            .iter()
            .flat_map(|chunk| parser.feed(chunk, 0))
            .map(|e| (e.event, e.id, e.data))
            .collect()
    }

    fn one(event: &str, id: &str, data: &str) -> (String, String, String) {
        (event.into(), id.into(), data.into())
    }

    #[test]
    fn data_lines_join_and_an_empty_line_dispatches() {
        assert_eq!(
            events(&[b"data: first\ndata:second\n\ndata\n\n"]),
            [one("message", "", "first\nsecond"), one("message", "", "")]
        );
    }

    #[test]
    fn fields_comments_and_unknown_lines() {
        assert_eq!(
            events(&[b": keep-alive\nevent: update\nid: 7\nretry: 100\nfoo: bar\ndata: {}\n\nevent: x\n\ndata: y\n\n"]),
            [
                one("update", "7", "{}"),
                one("message", "7", "y"),
            ],
            "the id stays in effect; an event without data dispatches nothing"
        );
        assert_eq!(
            events(&[b"id: a\0b\ndata: z\n\n"]),
            [one("message", "", "z")],
            "an id with NUL is ignored"
        );
    }

    #[test]
    fn every_line_ending_and_chunks_split_anywhere() {
        let whole = "\u{feff}data: caf\u{e9}\r\ndata: two\rdata: three\n\r\n";
        let expected = [one("message", "", "caf\u{e9}\ntwo\nthree")];
        assert_eq!(events(&[whole.as_bytes()]), expected);
        let bytes = whole.as_bytes();
        for split in 1..bytes.len() {
            assert_eq!(
                events(&[&bytes[..split], &bytes[split..]]),
                expected,
                "split at {split}"
            );
        }
        let singles: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(events(&singles), expected);
        assert_eq!(
            events(&[b"data: a\r", b"\ndata: b\n\n"]),
            [one("message", "", "a\nb")],
            "a CRLF split across chunks is one line ending"
        );
    }

    #[test]
    fn an_unfinished_event_is_not_dispatched() {
        assert_eq!(events(&[b"data: half"]), []);
        assert_eq!(events(&[b"data: half\n"]), []);
    }

    #[test]
    fn event_stream_types_and_normal_ends() {
        assert!(is_event_stream("text/event-stream; charset=utf-8"));
        assert!(is_event_stream("Text/Event-Stream"));
        assert!(!is_event_stream("text/plain"));
        assert!(StreamEnd::Closed.is_normal() && StreamEnd::Limit.is_normal());
        for end in [
            StreamEnd::Cut,
            StreamEnd::Cancelled,
            StreamEnd::TimedOut,
            StreamEnd::Failed,
        ] {
            assert!(!end.is_normal(), "{end:?}");
        }
    }
}
