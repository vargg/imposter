//! curl commands: parse one into a request template, or write a prepared request as one.
//!
//! Parsing understands POSIX shell quoting (`'…'`, `"…"`, `$'…'`, backslashes, line
//! continuations) and the Windows cmd escapes browsers write (`^"`, `^` at line ends). It
//! covers the options browsers' "Copy as cURL" and hand-written commands use; others are
//! reported as warnings and ignored.

use std::fmt::Write as _;

use crate::model::{BodyType, FormItem, ParamItem, RequestAuth, RequestTemplate};
use crate::prepare::{Prepared, PreparedAuth, PreparedBody};
use crate::settings::AppSettings;
use crate::url;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CurlError {
    #[error("Paste a curl command: it starts with \"curl\"")]
    NotCurl,
    #[error("A quote is not closed")]
    UnclosedQuote,
    #[error("{0} needs a value")]
    MissingValue(String),
    #[error("The command has no URL")]
    NoUrl,
    #[error("{option} has an invalid value: {value}")]
    InvalidValue { option: String, value: String },
}

/// A parsed command and what was ignored or approximated.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub template: RequestTemplate,
    pub warnings: Vec<String>,
}

// ---- tokenizing --------------------------------------------------------------------------

/// cmd writes `^` before special characters and at the end of continued lines.
fn strip_cmd_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '^' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            Some('\n') | None => {}
            Some(escaped) => out.push(escaped),
        }
    }
    out
}

fn looks_like_cmd(input: &str) -> bool {
    input.contains("^\"") || input.lines().any(|line| line.trim_end().ends_with(" ^"))
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buffer = [0; 4];
    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
}

fn hex(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, max: usize) -> Option<u32> {
    let mut digits = String::new();
    while digits.len() < max && chars.peek().is_some_and(char::is_ascii_hexdigit) {
        digits.extend(chars.next());
    }
    u32::from_str_radix(&digits, 16).ok()
}

/// One escape inside `$'…'`, after its backslash.
fn ansi_c_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut Vec<u8>) {
    match chars.next() {
        Some('n') => out.push(b'\n'),
        Some('t') => out.push(b'\t'),
        Some('r') => out.push(b'\r'),
        Some('0') => out.push(0),
        Some('e' | 'E') => out.push(0x1b),
        Some('x') => {
            if let Some(byte) = hex(chars, 2).and_then(|v| u8::try_from(v).ok()) {
                out.push(byte);
            }
        }
        Some('u') => {
            if let Some(c) = hex(chars, 4).and_then(char::from_u32) {
                push_char(out, c);
            }
        }
        Some('U') => {
            if let Some(c) = hex(chars, 8).and_then(char::from_u32) {
                push_char(out, c);
            }
        }
        Some(other @ ('\\' | '\'' | '"' | '?')) => push_char(out, other),
        Some(other) => {
            out.push(b'\\');
            push_char(out, other);
        }
        None => out.push(b'\\'),
    }
}

/// Split a shell command into words, as bash would (without expansions).
fn tokenize(input: &str) -> Result<Vec<String>, CurlError> {
    let input = if looks_like_cmd(input) {
        strip_cmd_escapes(input)
    } else {
        input.to_owned()
    };
    let mut words = Vec::new();
    let mut word: Vec<u8> = Vec::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();
    let push = push_char;
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(String::from_utf8_lossy(&word).into_owned());
                    word.clear();
                    in_word = false;
                }
            }
            // A backslash before a line break continues the line.
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some('\r') => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                Some(escaped) => {
                    in_word = true;
                    push(&mut word, escaped);
                }
            },
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => push(&mut word, c),
                        None => return Err(CurlError::UnclosedQuote),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => ansi_c_escape(&mut chars, &mut word),
                        Some(c) => push(&mut word, c),
                        None => return Err(CurlError::UnclosedQuote),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('\n') => {}
                            Some(c @ ('$' | '`' | '"' | '\\')) => push(&mut word, c),
                            Some(c) => {
                                push(&mut word, '\\');
                                push(&mut word, c);
                            }
                            None => return Err(CurlError::UnclosedQuote),
                        },
                        Some(c) => push(&mut word, c),
                        None => return Err(CurlError::UnclosedQuote),
                    }
                }
            }
            c => {
                in_word = true;
                push(&mut word, c);
            }
        }
    }
    if in_word {
        words.push(String::from_utf8_lossy(&word).into_owned());
    }
    Ok(words)
}

// ---- options -----------------------------------------------------------------------------

/// Long options that take a value; short ones are mapped to these names first.
const WITH_VALUE: &[&str] = &[
    "request",
    "header",
    "data",
    "data-raw",
    "data-binary",
    "data-ascii",
    "data-urlencode",
    "form",
    "form-string",
    "user",
    "cookie",
    "user-agent",
    "referer",
    "proxy",
    "max-time",
    "connect-timeout",
    "cacert",
    "cert",
    "key",
    "pass",
    "url",
    "oauth2-bearer",
    "output",
    "write-out",
    "cookie-jar",
    "upload-file",
    "proxy-user",
    "range",
    "retry",
    "max-redirs",
    "limit-rate",
    "resolve",
    "connect-to",
    "interface",
    "config",
];

/// Options that change nothing Imposter sends.
const HARMLESS: &[&str] = &[
    "compressed",
    "silent",
    "show-error",
    "verbose",
    "include",
    "fail",
    "fail-with-body",
    "no-buffer",
    "globoff",
    "http1.1",
    "location-trusted",
    "progress-bar",
    "no-progress-meter",
];

fn long_name(short: char) -> Option<&'static str> {
    Some(match short {
        'X' => "request",
        'H' => "header",
        'd' => "data",
        'F' => "form",
        'u' => "user",
        'b' => "cookie",
        'A' => "user-agent",
        'e' => "referer",
        'x' => "proxy",
        'm' => "max-time",
        'E' => "cert",
        'o' => "output",
        'w' => "write-out",
        'c' => "cookie-jar",
        'T' => "upload-file",
        'U' => "proxy-user",
        'r' => "range",
        'K' => "config",
        'L' => "location",
        'k' => "insecure",
        'I' => "head",
        'G' => "get",
        's' => "silent",
        'S' => "show-error",
        'v' => "verbose",
        'i' => "include",
        'f' => "fail",
        'N' => "no-buffer",
        'g' => "globoff",
        _ => return None,
    })
}

/// Expand `-XPOST`, `-sSL`, `--opt=value` into `(name, value)` pairs and positional words.
/// Options in order as `(long name, value)`, and the positional words.
type OptionList = (Vec<(String, Option<String>)>, Vec<String>);

fn options(words: &[String]) -> Result<OptionList, CurlError> {
    let mut found = Vec::new();
    let mut positional = Vec::new();
    let mut rest = words.iter();
    let takes_value = |name: &str| WITH_VALUE.contains(&name);
    while let Some(word) = rest.next() {
        if let Some(long) = word.strip_prefix("--") {
            if long.is_empty() {
                positional.extend(rest.by_ref().cloned());
                break;
            }
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) if takes_value(name) => (name, Some(value.to_owned())),
                _ => (long, None),
            };
            let value = match inline {
                Some(value) => Some(value),
                None if takes_value(name) => Some(
                    rest.next()
                        .cloned()
                        .ok_or_else(|| CurlError::MissingValue(format!("--{name}")))?,
                ),
                None => None,
            };
            found.push((name.to_owned(), value));
        } else if let Some(shorts) = word.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (index, short) in shorts.char_indices() {
                let Some(name) = long_name(short) else {
                    found.push((format!("-{short}"), None));
                    continue;
                };
                if takes_value(name) {
                    let attached = &shorts[index + short.len_utf8()..];
                    let value = if attached.is_empty() {
                        rest.next()
                            .cloned()
                            .ok_or_else(|| CurlError::MissingValue(format!("-{short}")))?
                    } else {
                        attached.to_owned()
                    };
                    found.push((name.to_owned(), Some(value)));
                    break;
                }
                found.push((name.to_owned(), None));
            }
        } else {
            positional.push(word.clone());
        }
    }
    Ok((found, positional))
}

#[derive(Default)]
struct Body {
    data: Vec<String>,
    file: Option<String>,
    form: Vec<FormItem>,
}

fn form_item(spec: &str, literal: bool, warnings: &mut Vec<String>) -> FormItem {
    let (name, rest) = spec.split_once('=').unwrap_or((spec, ""));
    if literal {
        return FormItem::text(name, rest);
    }
    let mut parts = rest.split(';');
    let value = parts.next().unwrap_or_default();
    let mut item = if let Some(path) = value.strip_prefix('@') {
        FormItem::file(name, path)
    } else if let Some(path) = value.strip_prefix('<') {
        warnings.push(format!("-F {name}=<{path}: sent as a file upload"));
        FormItem::file(name, path)
    } else {
        FormItem::text(name, value)
    };
    for part in parts {
        if let Some(content_type) = part.strip_prefix("type=") {
            content_type.clone_into(&mut item.content_type);
        } else if let Some(filename) = part.strip_prefix("filename=") {
            filename.trim_matches('"').clone_into(&mut item.filename);
        }
    }
    item
}

fn url_encoded(spec: &str, warnings: &mut Vec<String>) -> String {
    if let Some((name, value)) = spec.split_once('=') {
        if name.is_empty() {
            url::quote_plus(value)
        } else {
            format!("{name}={}", url::quote_plus(value))
        }
    } else if spec.contains('@') {
        warnings.push(format!(
            "--data-urlencode {spec}: reading files is not supported"
        ));
        String::new()
    } else {
        url::quote_plus(spec)
    }
}

fn header(spec: &str) -> Option<ParamItem> {
    let (name, value) = spec.split_once(':')?;
    let value = value.trim();
    // `Name:` removes a header in curl; there is nothing to remove here.
    (!value.is_empty()).then(|| ParamItem::new(name.trim(), value))
}

/// A short name for the draft: the URL's path, or its host.
fn request_name(address: &str) -> String {
    let parts = url::split(address);
    if parts.path.is_empty() || parts.path == "/" {
        parts.netloc
    } else {
        parts.path
    }
}

fn content_type(headers: &[ParamItem]) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
}

/// Parse a curl command into a request template for an unsaved draft.
pub fn parse(command: &str) -> Result<Parsed, CurlError> {
    let words = tokenize(command.trim())?;
    match words.first().map(String::as_str) {
        Some("curl" | "curl.exe") => {}
        _ => return Err(CurlError::NotCurl),
    }
    let (found, positional) = options(&words[1..])?;
    let mut template = RequestTemplate {
        body_type: BodyType::None,
        ..RequestTemplate::default()
    };
    let mut warnings = Vec::new();
    let mut body = Body::default();
    let (mut method, mut get, mut head, mut digest) = (None, false, false, false);
    let mut user: Option<String> = None;
    let mut address = None;
    for (name, value) in found {
        let value = value.unwrap_or_default();
        match name.as_str() {
            "request" => method = Some(value),
            "header" => template.headers.extend(header(&value)),
            "user-agent" => template.headers.push(ParamItem::new("User-Agent", value)),
            "referer" => template.headers.push(ParamItem::new("Referer", value)),
            "data" | "data-ascii" | "data-binary" => match value.strip_prefix('@') {
                Some(path) => body.file = Some(path.to_owned()),
                None => body.data.push(value),
            },
            "data-raw" => body.data.push(value),
            "data-urlencode" => body.data.push(url_encoded(&value, &mut warnings)),
            "form" => body.form.push(form_item(&value, false, &mut warnings)),
            "form-string" => body.form.push(form_item(&value, true, &mut warnings)),
            "user" => user = Some(value),
            "digest" => digest = true,
            "basic" => digest = false,
            "oauth2-bearer" => template.auth = RequestAuth::bearer(value),
            "cookie" if value.contains('=') => {
                for pair in value.split(';') {
                    if let Some((name, value)) = pair.split_once('=') {
                        template
                            .cookies
                            .push(ParamItem::new(name.trim(), value.trim()));
                    }
                }
            }
            "cookie" => warnings.push(format!("--cookie {value}: cookie files are not supported")),
            "url" => address = Some(value),
            "location" => template.options.follow_redirects = Some(true),
            "insecure" => template.options.verify_ssl = Some(false),
            "head" => head = true,
            "get" => get = true,
            "proxy" => template.options.proxy_url = Some(value),
            "max-time" => match value.parse::<f64>() {
                Ok(seconds) if seconds > 0.0 && seconds.is_finite() => {
                    template.options.timeout = Some(seconds);
                }
                _ => {
                    return Err(CurlError::InvalidValue {
                        option: "--max-time".into(),
                        value,
                    });
                }
            },
            "cacert" => template.options.ca_bundle = Some(value),
            "cert" => match value.split_once(':').filter(|(path, _)| path.len() > 1) {
                Some((path, password)) => {
                    template.options.certificate_path = Some(path.to_owned());
                    template.options.password = Some(password.to_owned());
                }
                None => template.options.certificate_path = Some(value),
            },
            "key" => template.options.key_file = Some(value),
            "pass" => template.options.password = Some(value),
            harmless if HARMLESS.contains(&harmless) => {}
            other => {
                let shown = if other.starts_with('-') {
                    other.to_owned()
                } else {
                    format!("--{other}")
                };
                warnings.push(format!("{shown} is not supported and was ignored"));
            }
        }
    }
    let mut positional = positional.into_iter();
    let address = address
        .or_else(|| positional.next())
        .ok_or(CurlError::NoUrl)?;
    for extra in positional {
        warnings.push(format!("{extra}: only the first URL is used"));
    }
    if let Some(user) = user {
        let (name, password) = user.split_once(':').unwrap_or((&user, ""));
        template.auth = if digest {
            RequestAuth::digest(name, password)
        } else {
            RequestAuth::basic(name, password)
        };
    }

    let data = body.data.join("&");
    template.url = address;
    if get && !body.data.is_empty() {
        let separator = if template.url.contains('?') { '&' } else { '?' };
        template.url = format!("{}{separator}{data}", template.url);
    } else if !body.form.is_empty() {
        template.body_type = BodyType::Multipart;
        template.set_multipart_fields(&body.form);
        // The boundary of a pasted multipart header would not match the new body.
        template.headers.retain(|h| {
            !(h.name.eq_ignore_ascii_case("content-type")
                && h.value.to_ascii_lowercase().starts_with("multipart/"))
        });
    } else if let Some(path) = body.file {
        if !body.data.is_empty() {
            warnings.push("Inline data next to a data file was ignored".into());
        }
        template.body_type = BodyType::Binary;
        template.body = path.into_bytes();
    } else if !body.data.is_empty() {
        set_data(&mut template, data);
    }

    template.method = match (method, head) {
        (Some(method), _) => method,
        (None, true) => "HEAD".into(),
        (None, false) if template.body_type != BodyType::None => "POST".into(),
        (None, false) => "GET".into(),
    };
    template.name = request_name(&template.url);
    Ok(Parsed { template, warnings })
}

/// Choose the body type for `-d` data from its content type, as curl would send it.
fn set_data(template: &mut RequestTemplate, data: String) {
    let declared = content_type(&template.headers);
    let pairs = !data.is_empty() && data.split('&').all(|part| part.contains('='));
    let json = serde_json::from_str::<serde::de::IgnoredAny>(&data).is_ok();
    match declared.as_deref() {
        Some(kind) if kind.contains("json") => {
            template.body_type = BodyType::Json;
            template.body = data.into_bytes();
        }
        Some(kind) if kind.starts_with("application/x-www-form-urlencoded") && pairs => {
            form(template, &data);
        }
        None if pairs && !json => form(template, &data),
        None if json => {
            template.body_type = BodyType::Json;
            template.body = data.into_bytes();
        }
        None => {
            // curl labels `-d` data as a form; keep that for raw text.
            template.headers.push(ParamItem::new(
                "Content-Type",
                "application/x-www-form-urlencoded",
            ));
            template.body_type = BodyType::Text;
            template.body = data.into_bytes();
        }
        Some(_) => {
            template.body_type = BodyType::Text;
            template.body = data.into_bytes();
        }
    }
}

fn form(template: &mut RequestTemplate, data: &str) {
    let fields: Vec<ParamItem> = url::parse_pairs(data)
        .into_iter()
        .map(|(name, value)| ParamItem::new(name, value))
        .collect();
    template.body_type = BodyType::Urlencoded;
    template.set_form_fields(&fields);
}

// ---- writing -----------------------------------------------------------------------------

/// Quote for a POSIX shell: single quotes, or `$'…'` when the bytes are not UTF-8 text.
fn quote(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => format!("'{}'", text.replace('\'', r"'\''")),
        _ => {
            let mut out = String::from("$'");
            for &byte in bytes {
                match byte {
                    b'\'' => out.push_str(r"\'"),
                    b'\\' => out.push_str(r"\\"),
                    b'\n' => out.push_str(r"\n"),
                    0x20..=0x7e => out.push(char::from(byte)),
                    _ => {
                        let _ = write!(out, "\\x{byte:02x}");
                    }
                }
            }
            out.push('\'');
            out
        }
    }
}

/// A prepared request as a curl command, one option per line. `settings` are the resolved
/// settings the request was sent with.
pub fn to_command(prepared: &Prepared, settings: &AppSettings) -> String {
    let mut parts = vec![format!("curl {}", quote(prepared.url.as_bytes()))];
    let has_body = !matches!(&prepared.body, PreparedBody::Bytes(bytes) if bytes.is_empty());
    let implied = if has_body { "POST" } else { "GET" };
    if prepared.method != implied {
        parts.push(format!("-X {}", quote(prepared.method.as_bytes())));
    }
    for (name, value) in &prepared.headers {
        parts.push(format!(
            "-H {}",
            quote(format!("{name}: {value}").as_bytes())
        ));
    }
    match &prepared.body {
        PreparedBody::Bytes(bytes) if bytes.is_empty() => {}
        PreparedBody::Bytes(bytes) => parts.push(format!("--data-raw {}", quote(bytes))),
        PreparedBody::Multipart(items) => {
            for item in items {
                let mut spec = match item.kind {
                    crate::model::FormKind::File => format!("{}=@{}", item.name, item.value),
                    crate::model::FormKind::Text => format!("{}={}", item.name, item.value),
                };
                if !item.content_type.is_empty() {
                    let _ = write!(spec, ";type={}", item.content_type);
                }
                if !item.filename.is_empty() {
                    let _ = write!(spec, ";filename={}", item.filename);
                }
                let option = match item.kind {
                    crate::model::FormKind::Text if item.content_type.is_empty() => "--form-string",
                    _ => "-F",
                };
                parts.push(format!("{option} {}", quote(spec.as_bytes())));
            }
        }
        PreparedBody::File(path) => {
            parts.push(format!(
                "--data-binary {}",
                quote(format!("@{path}").as_bytes())
            ));
        }
    }
    if let PreparedAuth::Digest(credentials) = &prepared.auth {
        let user = format!("{}:{}", credentials.username, credentials.password);
        parts.push(format!("--digest -u {}", quote(user.as_bytes())));
    }
    if settings.follow_redirects {
        parts.push("-L".into());
    }
    if !settings.verify_ssl {
        parts.push("-k".into());
    }
    let mut path_option = |option: &str, value: &str| {
        if !value.is_empty() {
            parts.push(format!("{option} {}", quote(value.as_bytes())));
        }
    };
    path_option("--proxy", &settings.proxy_url);
    path_option("--cacert", &settings.ca_bundle);
    path_option("--cert", &settings.certificate_path);
    path_option("--key", &settings.key_file);
    path_option("--pass", &settings.password);
    parts.join(" \\\n  ")
}
