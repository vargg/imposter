//! What scripts can use, in one table: the in-app reference and the README are drawn from it,
//! and a test runs every example, so the table cannot drift from what is registered.

/// When something can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The before-sending script.
    Before,
    /// The after-response script.
    After,
    Both,
}

impl Phase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Before => "Before sending",
            Self::After => "After response",
            Self::Both => "Both",
        }
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
        Phase::Both,
        "expect(actual, expected)",
        "Throws \"expected …, got …\" when they differ: inside a test it fails the test with that message.",
        r"expect(1 + 1, 2);",
    ),
    entry(
        Phase::Both,
        "vars.get(name), vars.set(name, value)",
        "Read a variable, or set one for later requests: it goes where captures go (the scope that defines it, else the active environment, else Global).",
        r#"vars.set("last_run", now_iso()); let host = vars.get("host");"#,
    ),
    entry(
        Phase::Both,
        "sha256(data), sha512(data), sha1(data), md5(data)",
        "Digests of text or bytes, as bytes.",
        r#"let digest = hex(sha256("body"));"#,
    ),
    entry(
        Phase::Both,
        "hmac_sha256(key, data), hmac_sha512(key, data), hmac_sha1(key, data)",
        "HMACs of text or bytes, as bytes; a key may be the bytes of an earlier HMAC.",
        r#"let signature = base64_encode(hmac_sha256("secret", "message"));"#,
    ),
    entry(
        Phase::Both,
        "hex(data), base64_encode(data), base64url_encode(data), base64_decode(text)",
        "Encode text or bytes; base64_decode gives bytes (`.as_string()` reads them as text).",
        r#"let text = base64_decode(base64_encode("hi")).as_string();"#,
    ),
    entry(
        Phase::Both,
        "url_encode(text), url_decode(text)",
        "Percent-encoding as RFC 3986: everything but letters, digits and - _ . ~ is escaped.",
        r#"let q = url_encode("a b/c");"#,
    ),
    entry(
        Phase::Both,
        "uuid(), now_ms(), now_iso(), random_int(min, max)",
        "Generated values, as {{$uuid}}, {{$timestampMs}}, {{$isoTimestamp}} and {{$randomInt}} make them.",
        r"let id = uuid(); let n = random_int(1, 6);",
    ),
    entry(
        Phase::Both,
        "parse_json(text), to_json(value)",
        "JSON text to a value and back.",
        r#"let value = parse_json("{\"a\": [1, 2]}"); let text = to_json(value);"#,
    ),
    entry(
        Phase::Both,
        "print(value), debug(value)",
        "Write to the console under the response; it is not kept in History.",
        r#"print("hello"); debug([1, 2]);"#,
    ),
];
