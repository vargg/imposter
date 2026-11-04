//! What scripts can use, in one table: the in-app reference and the README are drawn from it,
//! and a test runs every example, so the table cannot drift from what is registered.

/// Where something can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The before-sending script.
    Before,
    /// The after-response script.
    After,
    /// Both request scripts.
    Requests,
    /// A mock route's response script.
    Mock,
    /// Every script.
    All,
}

impl Phase {
    /// In the order the reference lists them.
    pub const LISTED: [Self; 5] = [
        Self::Before,
        Self::After,
        Self::Requests,
        Self::Mock,
        Self::All,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Before => "Before sending",
            Self::After => "After response",
            Self::Requests => "Request scripts",
            Self::Mock => "Mock responses",
            Self::All => "Every script",
        }
    }

    /// Whether what is listed under `self` can be used in a script of `script`'s kind.
    pub const fn covers(self, script: Self) -> bool {
        matches!(
            (self, script),
            (Self::All, _)
                | (Self::Requests, Self::Before | Self::After)
                | (Self::Before, Self::Before)
                | (Self::After, Self::After)
                | (Self::Mock, Self::Mock)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiEntry {
    pub phase: Phase,
    pub name: &'static str,
    pub description: &'static str,
    /// Runs as written in its phase (the "Both" ones in each).
    pub example: &'static str,
}

const fn entry(
    phase: Phase,
    name: &'static str,
    description: &'static str,
    example: &'static str,
) -> ApiEntry {
    ApiEntry {
        phase,
        name,
        description,
        example,
    }
}

pub const API: &[ApiEntry] = &[
    entry(
        Phase::Before,
        "request.method, request.url",
        "Read or change the method and URL. Setting the URL takes its query from it.",
        r#"request.method = "POST"; request.url = request.url + "?page=2";"#,
    ),
    entry(
        Phase::Before,
        "request.header(name), request.set_header(name, value), request.add_header(name, value), request.remove_header(name)",
        "Read or change headers; names ignore case. set_header replaces every value of the name. Auth headers are added after the script.",
        r#"request.set_header("X-Request-Id", uuid()); request.remove_header("X-Debug");"#,
    ),
    entry(
        Phase::Before,
        "request.query(name), request.set_query(name, value), request.remove_query(name)",
        "Read or change query parameters; the URL follows.",
        r#"request.set_query("ts", `${now_ms()}`);"#,
    ),
    entry(
        Phase::Before,
        "request.body, request.json()",
        "The body as text, or parsed as JSON. A JSON or text body can be replaced; form, multipart and file bodies cannot.",
        r"let data = request.json(); data.sent_at = now_iso(); request.body = to_json(data);",
    ),
    entry(
        Phase::After,
        "request",
        "The request as it was sent: the same fields, read-only.",
        r#"print(request.method + " " + request.url);"#,
    ),
    entry(
        Phase::After,
        "response.status, response.reason, response.time_ms, response.size, response.truncated",
        "The status, its reason, the time in milliseconds, the body's size in bytes, and whether the body was cut at the capture limit.",
        r"print(`${response.status} ${response.reason} in ${response.time_ms} ms`);",
    ),
    entry(
        Phase::After,
        "response.header(name), response.headers",
        "One header (every value joined with \", \"), or all of them by lowercase name.",
        r#"let type = response.header("content-type"); let all = response.headers;"#,
    ),
    entry(
        Phase::After,
        "response.body, response.json(), response.path(jsonpath)",
        "The body as text, parsed as JSON, or the value a JSONPath selects (several values as an array, none as ()).",
        r#"let id = response.path("$.id"); let doc = response.json();"#,
    ),
    entry(
        Phase::After,
        "test(name, condition), test(name, || { … })",
        "A test, shown with the checks and kept in History. A closure fails when it returns false or throws.",
        r#"test("status is 200", response.status == 200); test("has an id", || { expect(response.path("$.id"), 7); });"#,
    ),
    entry(
        Phase::Requests,
        "vars.get(name), vars.set(name, value)",
        "Read a variable, or set one for later requests: it goes where captures go (the scope that defines it, else the active environment, else Global).",
        r#"vars.set("last_run", now_iso()); let host = vars.get("host");"#,
    ),
    entry(
        Phase::Mock,
        "request.method, request.path, request.query(name), request.header(name), request.headers",
        "The request the route matched, read-only; headers by name, ignoring case.",
        r#"print(request.method + " " + request.path); let page = request.query("page");"#,
    ),
    entry(
        Phase::Mock,
        "request.body, request.json()",
        "The request's body as text (up to 1 MiB), or parsed as JSON.",
        r"let order = request.json(); let raw = request.body;",
    ),
    entry(
        Phase::Mock,
        "response.status, response.body, response.set_header(name, value), response.remove_header(name)",
        "The answer, starting from the route's status, headers and body. Statuses are 200 to 599; the server sets Content-Length itself.",
        r#"response.status = 201; response.set_header("Location", "/orders/1");"#,
    ),
    entry(
        Phase::Mock,
        "response.send_json(value)",
        "Answer with a value as JSON, setting Content-Type.",
        r"response.send_json(#{ ok: true, items: [1, 2] });",
    ),
    entry(
        Phase::Mock,
        "response.delay_ms",
        "Wait before answering, up to 10,000 ms.",
        r"response.delay_ms = 50;",
    ),
    entry(
        Phase::Mock,
        "state",
        "A map kept between requests while the server runs; it starts empty on every Start.",
        r"state.calls = (state.calls ?? 0) + 1; if state.calls == 1 { response.status = 500; }",
    ),
    entry(
        Phase::All,
        "expect(actual, expected)",
        "Throws \"expected …, got …\" when they differ: inside a test it fails the test with that message.",
        r"expect(1 + 1, 2);",
    ),
    entry(
        Phase::All,
        "sha256(data), sha512(data), sha1(data), md5(data)",
        "Digests of text or bytes, as bytes.",
        r#"let digest = hex(sha256("body"));"#,
    ),
    entry(
        Phase::All,
        "hmac_sha256(key, data), hmac_sha512(key, data), hmac_sha1(key, data)",
        "HMACs of text or bytes, as bytes; a key may be the bytes of an earlier HMAC.",
        r#"let signature = base64_encode(hmac_sha256("secret", "message"));"#,
    ),
    entry(
        Phase::All,
        "hex(data), base64_encode(data), base64url_encode(data), base64_decode(text)",
        "Encode text or bytes; base64_decode gives bytes (`.as_string()` reads them as text).",
        r#"let text = base64_decode(base64_encode("hi")).as_string();"#,
    ),
    entry(
        Phase::All,
        "url_encode(text), url_decode(text)",
        "Percent-encoding as RFC 3986: everything but letters, digits and - _ . ~ is escaped.",
        r#"let q = url_encode("a b/c");"#,
    ),
    entry(
        Phase::All,
        "uuid(), now_ms(), now_iso(), random_int(min, max)",
        "Generated values, as {{$uuid}}, {{$timestampMs}}, {{$isoTimestamp}} and {{$randomInt}} make them.",
        r"let id = uuid(); let n = random_int(1, 6);",
    ),
    entry(
        Phase::All,
        "parse_json(text), to_json(value)",
        "JSON text to a value and back.",
        r#"let value = parse_json("{\"a\": [1, 2]}"); let text = to_json(value);"#,
    ),
    entry(
        Phase::All,
        "print(value), debug(value)",
        "Write to the console: under the response (not kept in History), or in the Mocks screen's request log.",
        r#"print("hello"); debug([1, 2]);"#,
    ),
];

/// The README's script reference rows, generated from [`API`]; a test keeps them equal.
pub fn readme_rows() -> Vec<String> {
    API.iter()
        .map(|entry| {
            format!(
                "| {} | `{}` | {} |",
                entry.phase.label(),
                entry.name.replace('|', "\\|"),
                entry.description.replace('|', "\\|")
            )
        })
        .collect()
}
