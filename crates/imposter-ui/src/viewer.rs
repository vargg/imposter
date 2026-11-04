//! What the response viewer shows: the bounded preview text, its status line, display rows
//! and search. Pure functions; the widget only draws their results.

use std::collections::BTreeSet;
use std::ops::Range;

use imposter_app::core::execution::{ExecutionRecord, Outcome, Response};
use imposter_app::core::model::FormKind;
use imposter_app::core::prepare::{self, PreparedAuth, PreparedBody};
use imposter_app::core::select;

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
    /// Paths and folds of formatted JSON, when the whole body is shown formatted.
    pub outline: Option<Outline>,
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
    Some(reindent(text))
}

/// [`pretty_json`] for the start of a document cut by the preview limit or at capture: valid
/// as far as it goes (the parser only ran out of input), so it reads like a whole one rather
/// than one long minified line.
pub fn pretty_json_start(text: &str) -> Option<String> {
    match serde_json::from_str::<serde::de::IgnoredAny>(text) {
        Ok(_) => Some(reindent(text)),
        Err(error) if error.is_eof() => Some(reindent(text)),
        Err(_) => None,
    }
}

/// The byte length of a `{{name}}` placeholder at the start of `rest` (the editor's
/// highlight pattern: no braces or line breaks inside), if one starts there.
fn placeholder_len(rest: &str) -> Option<usize> {
    let inner = rest.strip_prefix("{{")?;
    let end = inner.find("}}")?;
    let name = &inner[..end];
    (!name.is_empty() && !name.contains(['{', '}', '\n'])).then_some(end + 4)
}

/// Check a request body as JSON once its `{{placeholders}}` are filled: one outside a string
/// (`{"n": {{n}}}`) stands for a value, so it reads as `0`. Line numbers stay as typed.
pub fn check_json_template(text: &str) -> Result<(), serde_json::Error> {
    let mut filled = String::with_capacity(text.len());
    let mut in_string = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if !in_string && let Some(len) = placeholder_len(rest) {
            filled.push('0');
            rest = &rest[len..];
            continue;
        }
        match c {
            '"' => in_string = !in_string,
            '\\' if in_string => {
                filled.push(c);
                rest = &rest[1..];
                if let Some(escaped) = rest.chars().next() {
                    filled.push(escaped);
                    rest = &rest[escaped.len_utf8()..];
                }
                continue;
            }
            _ => {}
        }
        filled.push(c);
        rest = &rest[c.len_utf8()..];
    }
    serde_json::from_str::<serde::de::IgnoredAny>(&filled).map(drop)
}

/// A request body re-indented with two spaces, its tokens and placeholders as typed; `None`
/// when it is not JSON even with its placeholders filled.
pub fn format_json_body(text: &str) -> Option<String> {
    check_json_template(text).ok()?;
    Some(reindent(text))
}

fn reindent(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut depth = 0_usize;
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n("  ", depth));
    };
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        // A placeholder standing for a value is copied as typed, not opened as an object.
        if let Some(len) = placeholder_len(rest) {
            out.push_str(&rest[..len]);
            rest = &rest[len..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
        match c {
            '"' => {
                out.push('"');
                let mut chars = rest.char_indices();
                let mut end = rest.len();
                while let Some((at, c)) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => {
                            end = at + 1;
                            break;
                        }
                        _ => {}
                    }
                }
                out.push_str(&rest[..end]);
                rest = &rest[end..];
            }
            '{' | '[' => {
                out.push(c);
                let close = if c == '{' { '}' } else { ']' };
                rest = rest.trim_start();
                if let Some(after) = rest.strip_prefix(close) {
                    out.push(close);
                    rest = after;
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
    out
}

pub fn preview(response: &Response) -> Preview {
    let content_type = content_type(&response.headers);
    let mut language = language(content_type);
    let shown = &response.body[..response.body.len().min(PREVIEW_LIMIT)];
    let mut text = decode(shown, response.body.len() > PREVIEW_LIMIT);
    let mut limited = response.truncated;
    let mut outline = None;
    if language == Language::Json
        && response.body.len() <= PREVIEW_LIMIT
        && let Some(formatted) = pretty_json(&text)
    {
        if formatted.len() > PREVIEW_LIMIT {
            limited = true;
            text = formatted[..formatted.floor_char_boundary(PREVIEW_LIMIT)].to_owned();
        } else {
            outline = Outline::build(&formatted);
            text = formatted;
        }
    }
    let cut = response.body.len() > PREVIEW_LIMIT || response.truncated;
    if language == Language::Json
        && cut
        && outline.is_none()
        && let Some(formatted) = pretty_json_start(&text)
    {
        // Folding needs the whole document, so a cut one has none.
        formatted[..formatted.floor_char_boundary(PREVIEW_LIMIT)].clone_into(&mut text);
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
        outline,
    }
}

/// Where each line of formatted JSON sits in its document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outline {
    pub lines: Vec<OutlineLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineLine {
    /// JSONPath of the value that starts on this line; on a closing line, of the container
    /// it closes.
    pub path: String,
    pub depth: usize,
    /// On a line that opens a non-empty object or array: the line that closes it.
    pub close: Option<usize>,
    /// Byte range of the value starting on this line, through its closing line, without the
    /// separating comma. `None` on closing lines.
    pub value: Option<Range<usize>>,
}

struct Open {
    line: usize,
    path: String,
    object: bool,
    next: usize,
}

impl Outline {
    /// The outline of `text` as [`pretty_json`] writes it; `None` for anything else.
    pub fn build(text: &str) -> Option<Self> {
        Self::build_with(text, select::ROOT, None)
    }

    /// The outline of formatted filter results: a top-level array whose elements are
    /// `paths` in the filtered document. Its first line is named by `query`.
    pub fn of_results(text: &str, query: &str, paths: &[String]) -> Option<Self> {
        Self::build_with(text, query, Some(paths))
    }

    fn build_with(text: &str, root: &str, roots: Option<&[String]>) -> Option<Self> {
        let mut lines: Vec<OutlineLine> = Vec::new();
        let mut stack: Vec<Open> = Vec::new();
        let mut start = 0;
        for (index, line) in text.split('\n').enumerate() {
            let end = start + line.len();
            let content = line.trim_start();
            let lead = start + line.len() - content.len();
            let value_end = end - usize::from(content.ends_with(','));
            if content.starts_with(['}', ']']) {
                let open = stack.pop()?;
                let opener = &mut lines[open.line];
                opener.close = Some(index);
                opener.value = opener.value.as_ref().map(|v| v.start..value_end);
                lines.push(OutlineLine {
                    path: open.path,
                    depth: stack.len(),
                    close: None,
                    value: None,
                });
            } else {
                let depth = stack.len();
                let (path, value_start) = match stack.last_mut() {
                    None if index == 0 => (root.to_owned(), lead),
                    None => return None,
                    Some(parent) if parent.object => {
                        let key_len = string_token_len(content)?;
                        let key: String = serde_json::from_str(&content[..key_len]).ok()?;
                        if !content[key_len..].starts_with(": ") {
                            return None;
                        }
                        (select::member(&parent.path, &key), lead + key_len + 2)
                    }
                    Some(parent) => {
                        let element = parent.next;
                        parent.next += 1;
                        let path = match roots {
                            Some(roots) if depth == 1 => roots.get(element)?.clone(),
                            _ => select::element_path(&parent.path, element),
                        };
                        (path, lead)
                    }
                };
                let rest = &text[value_start..value_end];
                if rest == "{" || rest == "[" {
                    stack.push(Open {
                        line: index,
                        path: path.clone(),
                        object: rest == "{",
                        next: 0,
                    });
                }
                lines.push(OutlineLine {
                    path,
                    depth: stack.len() - usize::from(rest == "{" || rest == "["),
                    close: None,
                    value: Some(value_start..value_end),
                });
            }
            start = end + 1;
        }
        stack.is_empty().then_some(Self { lines })
    }

    /// The value starting on `line` for the clipboard: strings unquoted, everything else as
    /// shown.
    pub fn copy_value(&self, text: &str, line: usize) -> Option<String> {
        let range = self.lines.get(line)?.value.clone()?;
        let raw = &text[range];
        Some(if raw.starts_with('"') {
            serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_owned())
        } else {
            raw.to_owned()
        })
    }

    /// Fold every non-empty container below the top level.
    pub fn collapse_all(&self) -> BTreeSet<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.close.is_some() && line.depth > 0)
            .map(|(index, _)| index)
            .collect()
    }
}

/// The length of the JSON string token at the start of `text`.
fn string_token_len(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    while let Some((offset, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' => return Some(offset + 1),
            _ => {}
        }
    }
    None
}

/// A recorded request as it went on the wire: `METHOD URL`, its headers and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentRequest {
    pub summary: String,
    pub headers: String,
    pub body: String,
    pub language: Language,
}

impl SentRequest {
    /// The request as one text: the request line, the headers, a blank line and the body,
    /// as it goes over the wire.
    pub fn as_text(&self) -> String {
        let mut text = self.summary.clone();
        if !self.headers.is_empty() {
            text.push('\n');
            text.push_str(&self.headers);
        }
        if !self.body.is_empty() {
            text.push_str("\n\n");
            text.push_str(&self.body);
        }
        text
    }
}

/// What `record` sent, prepared the same way the executor prepares it. Records that no longer
/// prepare (older formats) show their method and URL with an explanation.
pub fn sent_request(record: &ExecutionRecord) -> SentRequest {
    sent_request_of(&record.resolved, record.settings.attach_cookies)
}

/// What the resolved request `sent` went out as; see [`sent_request`].
pub fn sent_request_of(
    sent: &imposter_app::core::model::Request,
    attach_cookies: bool,
) -> SentRequest {
    let template = &sent.template;
    let prepared = match prepare::prepare(template, attach_cookies) {
        Ok(prepared) => prepared,
        Err(error) => {
            return SentRequest {
                summary: format!("{} {}", template.method, template.url),
                headers: String::new(),
                body: format!("This request cannot be shown as sent: {error}"),
                language: Language::Plain,
            };
        }
    };
    let mut headers: Vec<String> = prepared
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();
    if let PreparedAuth::Digest(_) = prepared.auth {
        headers.push("Authorization: Digest (answered to the server's challenge)".into());
    }
    let language = language(content_type(&prepared.headers));
    let (body, language) = match prepared.body {
        PreparedBody::Bytes(bytes) if bytes.is_empty() => (String::new(), Language::Plain),
        PreparedBody::Bytes(bytes) => {
            let cut = bytes.len() > PREVIEW_LIMIT;
            let mut text = decode(&bytes[..bytes.len().min(PREVIEW_LIMIT)], cut);
            if cut {
                if language == Language::Json
                    && let Some(formatted) = pretty_json_start(&text)
                {
                    formatted[..formatted.floor_char_boundary(PREVIEW_LIMIT)].clone_into(&mut text);
                }
                text.push_str("\n… Preview limited to 256 KiB");
                (text, Language::Plain)
            } else if language == Language::Json
                && let Some(formatted) = pretty_json(&text)
            {
                (formatted, language)
            } else {
                (text, language)
            }
        }
        PreparedBody::Multipart(items) => {
            let lines: Vec<String> = items
                .iter()
                .map(|item| {
                    let kind = match item.kind {
                        FormKind::File => "file ",
                        FormKind::Text => "",
                    };
                    let content_type = if item.content_type.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", item.content_type)
                    };
                    format!("{}: {kind}{}{content_type}", item.name, item.value)
                })
                .collect();
            (lines.join("\n"), Language::Plain)
        }
        PreparedBody::File(path) => (format!("File: {path}"), Language::Plain),
    };
    SentRequest {
        summary: format!("{} {}", prepared.method, prepared.url),
        headers: headers.join("\n"),
        body,
        language,
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
                text: failure_title(&response.error_kind).to_owned(),
                tone: Tone::Error,
                detail: reason,
            }
        }
        Outcome::Response => Status {
            text: if response.reason.is_empty() {
                response.status.to_string()
            } else {
                format!("{} {}", response.status, response.reason)
            },
            tone: match response.status {
                200..=299 => Tone::Ok,
                300..=399 => Tone::Warning,
                _ => Tone::Error,
            },
            detail: format!("{} {}", response.status, response.reason),
        },
    }
}

/// How long ago `then` was, at `now` (both in milliseconds): "just now", "12 s ago", "5 min
/// ago", "3 h ago", "2 d ago", then the date in the system time zone ("24 Oct 2026").
pub fn relative_time(
    then: imposter_app::core::execution::Timestamp,
    now: imposter_app::core::execution::Timestamp,
) -> String {
    let seconds = (now.0 - then.0).max(0) / 1000;
    match seconds {
        0..=9 => "just now".to_owned(),
        10..=59 => format!("{seconds} s ago"),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        86_400..=604_799 => format!("{} d ago", seconds / 86_400),
        _ => jiff::Timestamp::from_millisecond(then.0).map_or_else(
            |_| then.to_rfc3339(),
            |instant| {
                instant
                    .to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%d %b %Y")
                    .to_string()
            },
        ),
    }
}

/// A URL's origin (`https://api.example.com:8443`) and the rest (`/users?page=1`), so a
/// list can mute the part that repeats. Text that is not a URL is all rest.
pub fn split_origin(url: &str) -> (&str, &str) {
    let Some(scheme) = url.find("://") else {
        return ("", url);
    };
    let after = scheme + 3;
    let end = url[after..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |at| after + at);
    url.split_at(end)
}

/// A history timestamp in the system time zone: "24 Oct 2026, 21:05:35".
pub fn local_time(timestamp: imposter_app::core::execution::Timestamp) -> String {
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

/// A short, plain name for a transport failure kind (`Response::error_kind`).
pub fn failure_title(kind: &str) -> &'static str {
    match kind {
        "ConnectError" => "Couldn't connect",
        "Timeout" => "Timed out",
        "TlsError" => "Secure connection failed",
        "TooManyRedirects" => "Too many redirects",
        "UploadError" => "Upload failed",
        "DigestError" => "Digest authentication failed",
        "BodyError" => "Reading the response failed",
        "ProtocolError" => "The server broke the HTTP protocol",
        "InvalidUrl" => "Invalid URL",
        "InvalidHeader" => "Invalid header",
        "InvalidProxy" => "Invalid proxy",
        _ => "Request failed",
    }
}

/// What to check after a transport failure of `kind`, if there is a useful general hint.
pub fn failure_hint(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "ConnectError" => {
            "Nothing answered at that address. Is the server running, and are the host and port right?"
        }
        "Timeout" => {
            "No answer within the timeout. Check the server, or raise the timeout in this request's Settings tab."
        }
        "TlsError" => {
            "The secure connection failed. Check http vs https, the CA bundle, or certificate verification in Settings."
        }
        "TooManyRedirects" => "The server kept redirecting (more than 20 times).",
        "InvalidUrl" => "The URL must be absolute and start with http:// or https://.",
        "InvalidProxy" => "Check the proxy URL in Settings.",
        "UploadError" => "A file to upload could not be read. Check its path in the Body tab.",
        _ => return None,
    })
}

/// The status bar's one-line result of a send: "200 OK · 12 ms", "Couldn't connect · 3 ms",
/// "Cancelled", toned like the status.
pub fn outcome_summary(response: &Response) -> (String, Tone) {
    let time = format_duration(response.elapsed);
    match response.outcome {
        Outcome::Response => {
            let status = status(response);
            let line = format!("{} · {time}", status.text);
            let failed = response
                .checks
                .iter()
                .filter(|r| r.verdict == imposter_app::core::check::Verdict::Failed)
                .count();
            if failed > 0 {
                let checks = if failed == 1 { "check" } else { "checks" };
                return (format!("{line} · {failed} {checks} failed"), Tone::Error);
            }
            (line, status.tone)
        }
        Outcome::TransportError => (
            format!("{} · {time}", failure_title(&response.error_kind)),
            Tone::Error,
        ),
        Outcome::Cancelled => ("Send cancelled".to_owned(), Tone::Warning),
    }
}

/// "123 ms", "2.50 s".
pub fn format_duration(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else {
        format!("{seconds:.2} s")
    }
}

/// "512 B", "1.5 KB", "584 KB", "12 MB", counting 1024 bytes to the kilobyte.
#[allow(clippy::cast_precision_loss)] // a display value; sizes stay far below 2^52
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = "KB";
    for larger in ["MB", "GB"] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = larger;
    }
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// The body's size; a body cut at the capture limit says so.
pub fn size_text(response: &Response) -> String {
    let size = format_size(response.body.len());
    if response.truncated {
        format!("{size}, cut")
    } else {
        size
    }
}

/// The notice under the status line, joining what the user should know.
pub fn notice(response: &Response, preview: &Preview) -> String {
    let mut parts: Vec<String> = Vec::new();
    match response.outcome {
        Outcome::Cancelled => parts.push("Execution cancelled".to_owned()),
        Outcome::Response if response.body.is_empty() => {
            parts.push("Response body is empty".to_owned());
        }
        Outcome::Response if response.truncated => parts.push(format!(
            "Body cut at {} when captured",
            format_size(response.body.len())
        )),
        Outcome::Response if preview.limited => parts.push(format!(
            "Preview shows {} of {}",
            format_size(PREVIEW_LIMIT),
            format_size(response.body.len())
        )),
        _ => {}
    }
    if !response.history_error.is_empty() {
        parts.push(response.history_error.clone());
    }
    parts.join(" · ")
}

/// The file name offered when saving `response`'s body, by its content type.
pub fn body_file_name(response: &Response) -> &'static str {
    match language(content_type(&response.headers)) {
        Language::Json => "response.json",
        Language::Xml => "response.xml",
        Language::Html => "response.html",
        Language::Plain => "response.txt",
    }
}

/// Whether the response says its body is JSON, whatever its size.
pub fn is_json(response: &Response) -> bool {
    response.outcome == Outcome::Response
        && language(content_type(&response.headers)) == Language::Json
}

/// The results of a JSONPath filter, formatted for the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtered {
    pub text: String,
    pub count: usize,
    pub outline: Option<Outline>,
    /// The formatted results were longer than the preview limit and are cut.
    pub limited: bool,
}

/// The parsed body a filter runs on, kept so later queries do not parse it again.
pub type Document = std::sync::Arc<Result<serde_json::Value, select::SelectError>>;

/// Run `query` over `document`. Results are re-serialized, so integers up to 64 bits and
/// floats keep their value but not their exact spelling (`1.50` becomes `1.5`).
pub fn filter(document: &Document, query: &str) -> Result<Filtered, String> {
    let document = document.as_ref().as_ref().map_err(ToString::to_string)?;
    let path = select::parse_path(query).map_err(|error| error.to_string())?;
    let found = select::query(document, &path);
    let count = found.len();
    let paths: Vec<String> = found.iter().map(|f| f.path.clone()).collect();
    let values = serde_json::Value::Array(found.into_iter().map(|f| f.value).collect());
    let text = serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?;
    Ok(if text.len() > PREVIEW_LIMIT {
        let mut cut = text[..text.floor_char_boundary(PREVIEW_LIMIT)].to_owned();
        cut.push_str("\n… Preview limited to 256 KiB");
        Filtered {
            text: cut,
            count,
            outline: None,
            limited: true,
        }
    } else {
        Filtered {
            outline: Outline::of_results(&text, query.trim(), &paths),
            text,
            count,
            limited: false,
        }
    })
}

/// What a background filter run starts from.
pub enum FilterSource {
    Parsed(Document),
    Body { bytes: Vec<u8>, truncated: bool },
}

impl FilterSource {
    /// Parse when needed, then filter. Runs off the UI thread.
    pub fn run(self, query: &str) -> (Document, Result<Filtered, String>) {
        let document = match self {
            Self::Parsed(document) => document,
            Self::Body { bytes, truncated } => {
                std::sync::Arc::new(select::parse_body(&bytes, truncated))
            }
        };
        let result = filter(&document, query);
        (document, result)
    }
}

/// One display row: a byte range of the text and the line it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub range: Range<usize>,
    pub line: usize,
}

/// Display rows: each line, split into chunks of at most `columns` characters. Without wrapping,
/// very long lines are still chunked so a single row never costs a whole-body layout.
pub fn rows(text: &str, columns: Option<usize>) -> Vec<Row> {
    const UNWRAPPED_CHUNK: usize = 4096;
    let width = columns.unwrap_or(UNWRAPPED_CHUNK).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    for (line, content) in text.split('\n').enumerate() {
        let end = start + content.len();
        let mut chunk_start = start;
        let mut count = 0;
        for (offset, _) in content.char_indices() {
            if count == width {
                out.push(Row {
                    range: chunk_start..start + offset,
                    line,
                });
                chunk_start = start + offset;
                count = 0;
            }
            count += 1;
        }
        out.push(Row {
            range: chunk_start..end,
            line,
        });
        start = end + 1;
    }
    out
}

/// The rows left when the lines inside each folded container are hidden. The folded line
/// itself stays; its closing line is hidden with the contents.
pub fn unfolded(rows: Vec<Row>, outline: &Outline, folded: &BTreeSet<usize>) -> Vec<Row> {
    if folded.is_empty() {
        return rows;
    }
    // The folded line and the last line it hides.
    let mut hiding: Option<(usize, usize)> = None;
    rows.into_iter()
        .filter(|row| {
            if let Some((opener, close)) = hiding {
                if row.line == opener {
                    return true;
                }
                if row.line <= close {
                    return false;
                }
                hiding = None;
            }
            if folded.contains(&row.line)
                && let Some(close) = outline.lines.get(row.line).and_then(|l| l.close)
            {
                hiding = Some((row.line, close));
            }
            true
        })
        .collect()
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

    #[test]
    fn a_body_with_placeholders_is_json_once_they_are_filled() {
        assert!(check_json_template(r#"{"n": {{n}}, "s": "{{s}}"}"#).is_ok());
        assert!(check_json_template("[{{a}}, {{b}}]").is_ok());
        assert!(check_json_template(r#"{"s": "a \" {{x}}"}"#).is_ok());
        // Braces that are no placeholder stay errors, on the line typed.
        let error = check_json_template("{\n\"a\": {{}}\n}").unwrap_err();
        assert_eq!(error.line(), 2);
        assert!(check_json_template(r#"{"n": {{n}} "m": 1}"#).is_err());
    }

    #[test]
    fn formatting_a_body_keeps_tokens_and_placeholders_as_typed() {
        assert_eq!(
            format_json_body(r#"{"id":{{id}},"price":19.50,"tags":["a\"b",{}],"s":"{{x}}"}"#)
                .as_deref(),
            Some(
                "{\n  \"id\": {{id}},\n  \"price\": 19.50,\n  \"tags\": [\n    \"a\\\"b\",\n    {}\n  ],\n  \"s\": \"{{x}}\"\n}"
            )
        );
        assert_eq!(format_json_body("{\"a\": 1,}"), None, "not JSON");
        // Formatting twice changes nothing.
        let once = format_json_body(r#"{"a":[1,2]}"#).unwrap();
        assert_eq!(format_json_body(&once).as_deref(), Some(once.as_str()));
    }
    use imposter_app::core::model::{
        BodyType, FormItem, ParamItem, Request, RequestAuth, RequestTemplate,
    };

    fn record(template: RequestTemplate) -> ExecutionRecord {
        let request = Request {
            template,
            ..Request::default()
        };
        ExecutionRecord {
            timestamp: imposter_app::core::execution::Timestamp(0),
            request_id: None,
            template: request.clone(),
            resolved: request,
            settings: imposter_app::core::settings::AppSettings::default(),
            environment_id: None,
            environment: "Global".into(),
            response: Response::default(),
        }
    }

    #[test]
    fn a_json_post_shows_its_wire_headers_and_formatted_body() {
        let sent = sent_request(&record(RequestTemplate {
            method: "POST".into(),
            url: "http://h/items?a=1".into(),
            headers: vec![ParamItem::new("X-Trace", "on")],
            body: br#"{"a":1}"#.to_vec(),
            body_type: BodyType::Json,
            ..RequestTemplate::default()
        }));
        assert_eq!(sent.summary, "POST http://h/items?a=1");
        assert!(sent.headers.contains("X-Trace: on"), "{}", sent.headers);
        assert!(
            sent.headers
                .to_lowercase()
                .contains("content-type: application/json")
        );
        assert_eq!(sent.body, "{\n  \"a\": 1\n}");
        assert_eq!(sent.language, Language::Json);

        // Larger than the preview: still laid out as far as it goes.
        let sent = sent_request(&record(RequestTemplate {
            method: "POST".into(),
            url: "http://h/".into(),
            body: format!("[{}1]", "{\"id\":1},".repeat(30_000)).into_bytes(),
            body_type: BodyType::Json,
            ..RequestTemplate::default()
        }));
        assert!(sent.body.starts_with("[\n  {\n    \"id\": 1\n  },"));
        assert!(sent.body.ends_with("… Preview limited to 256 KiB"));
    }

    #[test]
    fn multipart_files_and_digest_are_described() {
        let mut template = RequestTemplate {
            method: "POST".into(),
            url: "http://h/".into(),
            body_type: BodyType::Multipart,
            ..RequestTemplate::default()
        };
        template.set_multipart_fields(&[
            FormItem::text("note", "hi"),
            FormItem::file("upload", "/tmp/a.bin"),
        ]);
        template.auth = RequestAuth::digest("u", "p");
        let sent = sent_request(&record(template));
        assert_eq!(sent.body, "note: hi\nupload: file /tmp/a.bin");
        assert!(sent.headers.contains("Digest"), "{}", sent.headers);
    }

    #[test]
    fn unpreparable_records_say_why() {
        let sent = sent_request(&record(RequestTemplate {
            url: "not a url".into(),
            ..RequestTemplate::default()
        }));
        assert_eq!(sent.summary, "GET not a url");
        assert!(
            sent.body
                .starts_with("This request cannot be shown as sent")
        );
    }

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
    fn a_cut_json_body_is_still_laid_out() {
        assert_eq!(pretty_json("{\"a\":[1,"), None, "incomplete");
        assert_eq!(
            pretty_json_start("{\"a\":[1,").as_deref(),
            Some("{\n  \"a\": [\n    1,\n    ")
        );
        assert_eq!(
            pretty_json_start("{oops"),
            None,
            "not JSON as far as it goes"
        );

        let big = format!("[{}1]", "{\"id\":1},".repeat(30_000));
        let preview = preview(&response(big.as_bytes(), "application/json"));
        assert!(preview.limited);
        assert!(
            preview.text.starts_with("[\n  {\n    \"id\": 1\n  },"),
            "{}",
            &preview.text[..40]
        );
        assert!(preview.text.len() <= PREVIEW_LIMIT + 40);
        assert!(preview.text.ends_with("… Preview limited to 256 KiB"));
        assert_eq!(
            preview.outline, None,
            "nothing to fold without the whole document"
        );
    }

    const DOCUMENT: &str =
        r#"{"data":{"items":[{"id":7,"name":"a \"b\""},{"id":8,"tags":[]}],"a b":null},"n":1.50}"#;

    #[test]
    fn outlines_name_every_line_and_fold_containers() {
        let text = pretty_json(DOCUMENT).unwrap();
        let outline = Outline::build(&text).unwrap();
        let lines: Vec<&str> = text.split('\n').collect();
        let shown: Vec<(&str, &str, usize, Option<usize>)> = outline
            .lines
            .iter()
            .zip(&lines)
            .map(|(l, text)| (text.trim(), l.path.as_str(), l.depth, l.close))
            .collect();
        assert_eq!(
            shown,
            [
                ("{", "$", 0, Some(15)),
                ("\"data\": {", "$.data", 1, Some(13)),
                ("\"items\": [", "$.data.items", 2, Some(11)),
                ("{", "$.data.items[0]", 3, Some(6)),
                ("\"id\": 7,", "$.data.items[0].id", 4, None),
                ("\"name\": \"a \\\"b\\\"\"", "$.data.items[0].name", 4, None),
                ("},", "$.data.items[0]", 3, None),
                ("{", "$.data.items[1]", 3, Some(10)),
                ("\"id\": 8,", "$.data.items[1].id", 4, None),
                ("\"tags\": []", "$.data.items[1].tags", 4, None),
                ("}", "$.data.items[1]", 3, None),
                ("],", "$.data.items", 2, None),
                ("\"a b\": null", "$.data['a b']", 2, None),
                ("},", "$.data", 1, None),
                ("\"n\": 1.50", "$.n", 1, None),
                ("}", "$", 0, None),
            ]
        );
        assert_eq!(outline.copy_value(&text, 4).as_deref(), Some("7"));
        assert_eq!(
            outline.copy_value(&text, 5).as_deref(),
            Some("a \"b\""),
            "strings are copied unquoted"
        );
        assert_eq!(
            outline.copy_value(&text, 3).as_deref(),
            Some("{\n        \"id\": 7,\n        \"name\": \"a \\\"b\\\"\"\n      }"),
            "containers are copied as shown, without the separating comma"
        );
        assert_eq!(
            outline.copy_value(&text, 6),
            None,
            "closing lines have no value"
        );
        assert_eq!(outline.copy_value(&text, 14).as_deref(), Some("1.50"));
        assert_eq!(
            outline.collapse_all(),
            BTreeSet::from([1, 2, 3, 7]),
            "everything below the top level"
        );
    }

    #[test]
    fn every_outlined_path_selects_its_value() {
        let text = pretty_json(DOCUMENT).unwrap();
        let outline = Outline::build(&text).unwrap();
        let document = select::parse_body(DOCUMENT.as_bytes(), false).unwrap();
        for (index, line) in outline.lines.iter().enumerate() {
            let Some(range) = line.value.clone() else {
                continue;
            };
            let found = select::query(&document, &select::parse_path(&line.path).unwrap());
            assert_eq!(found.len(), 1, "{}", line.path);
            let shown: serde_json::Value = serde_json::from_str(&text[range]).unwrap();
            assert_eq!(found[0].value, shown, "line {index}: {}", line.path);
        }
    }

    #[test]
    fn outlines_of_results_use_the_paths_in_the_filtered_document() {
        let text = "[\n  {\n    \"id\": 7\n  },\n  8\n]";
        let paths = [
            "$.data.items[0]".to_owned(),
            "$.data.items[1].id".to_owned(),
        ];
        let outline = Outline::of_results(text, "$..items[*]", &paths).unwrap();
        let named: Vec<&str> = outline.lines.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(
            named,
            [
                "$..items[*]",
                "$.data.items[0]",
                "$.data.items[0].id",
                "$.data.items[0]",
                "$.data.items[1].id",
                "$..items[*]"
            ]
        );
        assert_eq!(Outline::of_results(text, "$", &paths[..1]), None);
    }

    #[test]
    fn only_complete_formatted_json_has_an_outline() {
        assert_eq!(Outline::build("{\n  \"a\": 1"), None, "cut");
        assert_eq!(Outline::build("}"), None);
        assert_eq!(Outline::build("1\n2"), None, "two documents");
        assert_eq!(Outline::build("{\n  oops\n}"), None);
        assert_eq!(Outline::build("{\n  \"a\" 1\n}"), None);
        assert_eq!(
            Outline::build("\"unterminated"),
            Some(Outline {
                lines: vec![OutlineLine {
                    path: "$".into(),
                    depth: 0,
                    close: None,
                    value: Some(0..13),
                }],
            }),
            "a scalar line is taken as shown"
        );
        let json = response(DOCUMENT.as_bytes(), "application/json");
        assert!(preview(&json).outline.is_some());
        assert_eq!(
            preview(&response(b"{oops", "application/json")).outline,
            None
        );
        assert_eq!(preview(&response(b"{}", "text/plain")).outline, None);
    }

    #[test]
    fn filters_format_results_with_their_paths_and_explain_failures() {
        let source = FilterSource::Body {
            bytes: DOCUMENT.as_bytes().to_vec(),
            truncated: false,
        };
        let (document, result) = source.run(" $.data.items[*].id ");
        let found = result.unwrap();
        assert_eq!((found.text.as_str(), found.count), ("[\n  7,\n  8\n]", 2));
        let paths: Vec<&str> = found
            .outline
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .map(|l| l.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "$.data.items[*].id",
                "$.data.items[0].id",
                "$.data.items[1].id",
                "$.data.items[*].id"
            ]
        );
        let (_, numbers) = FilterSource::Parsed(document.clone()).run("$.n");
        assert_eq!(numbers.unwrap().text, "[\n  1.5\n]", "re-serialized");
        let (_, none) = FilterSource::Parsed(document.clone()).run("$.nothing");
        assert_eq!(none.unwrap().count, 0);
        let (_, bad) = FilterSource::Parsed(document).run("data");
        assert!(bad.unwrap_err().starts_with("Invalid JSONPath"));

        let (_, cut) = FilterSource::Body {
            bytes: b"{}".to_vec(),
            truncated: true,
        }
        .run("$");
        assert!(cut.unwrap_err().contains("capture limit"));
        let (_, html) = FilterSource::Body {
            bytes: b"<html>".to_vec(),
            truncated: false,
        }
        .run("$");
        assert!(html.unwrap_err().starts_with("The body is not JSON"));

        let many = format!("[{}]", vec!["\"xxxxxxxxxx\""; 30_000].join(","));
        let (_, large) = FilterSource::Body {
            bytes: many.into_bytes(),
            truncated: false,
        }
        .run("$[*]");
        let large = large.unwrap();
        assert!(large.limited && large.outline.is_none() && large.count == 30_000);
        assert!(large.text.ends_with("Preview limited to 256 KiB"));
    }

    #[test]
    fn only_json_responses_can_be_filtered() {
        assert!(is_json(&response(b"", "application/problem+json")));
        assert!(!is_json(&response(b"{}", "text/plain")));
        let mut failed = response(b"", "application/json");
        failed.outcome = Outcome::TransportError;
        assert!(!is_json(&failed));
    }

    #[test]
    fn folded_containers_hide_their_lines_through_the_closing_one() {
        let text = pretty_json(DOCUMENT).unwrap();
        let outline = Outline::build(&text).unwrap();
        let lines = |folded: &[usize]| -> Vec<usize> {
            let mut kept: Vec<usize> = unfolded(
                rows(&text, None),
                &outline,
                &folded.iter().copied().collect(),
            )
            .into_iter()
            .map(|row| row.line)
            .collect();
            kept.dedup();
            kept
        };
        assert_eq!(lines(&[]).len(), 16);
        assert_eq!(lines(&[2]), [0, 1, 2, 12, 13, 14, 15]);
        assert_eq!(
            lines(&[2, 3]),
            [0, 1, 2, 12, 13, 14, 15],
            "nested folds inside"
        );
        assert_eq!(lines(&[3, 7]), [0, 1, 2, 3, 7, 11, 12, 13, 14, 15]);
        assert_eq!(lines(&[0]), [0]);
        assert_eq!(
            lines(&[4]).len(),
            16,
            "a line that opens nothing does not fold"
        );
        let wrapped = unfolded(rows(&text, Some(4)), &outline, &BTreeSet::from([1]));
        assert!(wrapped.iter().all(|row| row.line <= 1 || row.line > 13));
        assert!(
            wrapped.iter().filter(|row| row.line == 1).count() > 1,
            "the folded line wraps"
        );
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
                text: "200 OK".into(),
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
        assert_eq!(
            (
                status(&failed).text.as_str(),
                status(&failed).detail.as_str()
            ),
            ("Couldn't connect", "refused"),
            "the raw text is the detail, not the headline"
        );
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
            "Body cut at 1 B when captured"
        );
        let large = response(&vec![b'x'; PREVIEW_LIMIT + 1024], "text/plain");
        assert_eq!(
            notice(&large, &preview(&large)),
            "Preview shows 256 KB of 257 KB"
        );
        assert_eq!(body_file_name(&large), "response.txt");
        let json = response(b"{}", "application/json");
        assert_eq!(body_file_name(&json), "response.json");
        assert_eq!(size_text(&truncated), "1 B, cut");
        assert_eq!(format_duration(0.1234), "123 ms");
        assert_eq!(format_duration(2.5), "2.50 s");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(584 * 1024 + 100), "584 KB");
        assert_eq!(format_size(12 * 1024 * 1024), "12 MB");
        assert_eq!(format_size(3 << 40), "3072 GB");
        let unnamed = Response {
            status: 599,
            reason: String::new(),
            ..response(b"", "text/plain")
        };
        assert_eq!(status(&unnamed).text, "599");
        assert_eq!(headers_text(&response(b"", "a/b")), "content-type: a/b");
    }

    #[test]
    fn local_times_use_a_readable_format() {
        let text = local_time(imposter_app::core::execution::Timestamp(1_793_000_000_000));
        assert!(text.contains("2026"), "{text}");
        assert_eq!(text.len(), "26 Oct 2026, 07:33:20".len());
        assert_eq!(
            local_time(imposter_app::core::execution::Timestamp(i64::MAX)),
            imposter_app::core::execution::Timestamp(i64::MAX).to_rfc3339()
        );
    }

    #[test]
    fn outcomes_read_in_plain_language() {
        let mut ok = response(b"", "text/plain");
        ok.elapsed = 0.012;
        assert_eq!(
            outcome_summary(&ok),
            ("200 OK · 12 ms".to_owned(), Tone::Ok)
        );
        let mut missing = ok.clone();
        missing.status = 404;
        missing.reason = "Not Found".into();
        assert_eq!(outcome_summary(&missing).1, Tone::Error);
        let mut refused = Response::failure(Outcome::TransportError, "ConnectError", "tcp …");
        refused.elapsed = 0.003;
        assert_eq!(
            outcome_summary(&refused),
            ("Couldn't connect · 3 ms".to_owned(), Tone::Error),
            "never an internal type name"
        );
        assert_eq!(failure_title("Timeout"), "Timed out");
        assert_eq!(failure_title("Something new"), "Request failed");
        let mut checked = ok.clone();
        let result = |verdict| imposter_app::core::check::CheckResult {
            check: imposter_app::core::check::Check::new(
                select::Selector::Status,
                imposter_app::core::check::Op::Equals,
                "200",
            ),
            verdict,
            actual: None,
        };
        checked.checks = vec![
            result(imposter_app::core::check::Verdict::Passed),
            result(imposter_app::core::check::Verdict::Failed),
        ];
        assert_eq!(
            outcome_summary(&checked),
            ("200 OK · 12 ms · 1 check failed".to_owned(), Tone::Error),
            "a failed check outweighs a good status"
        );
        let cancelled = Response::failure(Outcome::Cancelled, "Cancelled", "Request cancelled");
        assert_eq!(outcome_summary(&cancelled).0, "Send cancelled");
    }

    #[test]
    fn rows_split_lines_and_long_lines() {
        let text = "ab\ncdefg\n\nh";
        let slices: Vec<(&str, usize)> = rows(text, Some(2))
            .into_iter()
            .map(|r| (&text[r.range], r.line))
            .collect();
        assert_eq!(
            slices,
            [("ab", 0), ("cd", 1), ("ef", 1), ("g", 1), ("", 2), ("h", 3)]
        );
        let unicode = "ééé";
        let chunks: Vec<&str> = rows(unicode, Some(2))
            .into_iter()
            .map(|r| &unicode[r.range])
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
    #[test]
    fn relative_times_read_as_people_say_them() {
        use imposter_app::core::execution::Timestamp;
        let now = Timestamp(1_800_000_000_000);
        let ago = |seconds: i64| relative_time(Timestamp(now.0 - seconds * 1000), now);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(-5), "just now", "a clock behind the record");
        assert_eq!(ago(42), "42 s ago");
        assert_eq!(ago(5 * 60 + 59), "5 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(2 * 86_400), "2 d ago");
        assert!(ago(30 * 86_400).ends_with(" 2026"), "{}", ago(30 * 86_400));
    }

    #[test]
    fn urls_split_into_origin_and_rest() {
        assert_eq!(
            split_origin("http://127.0.0.1:9/users?page=1"),
            ("http://127.0.0.1:9", "/users?page=1")
        );
        assert_eq!(
            split_origin("https://api.example.com"),
            ("https://api.example.com", "")
        );
        assert_eq!(split_origin("https://h?q=1#f"), ("https://h", "?q=1#f"));
        assert_eq!(split_origin("{{base}}/users"), ("", "{{base}}/users"));
    }
}
