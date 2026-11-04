use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use imposter_core::execution::{Response, Timestamp};
use imposter_core::generate::Entropy;
use imposter_core::mock::{MockReply, MockRequest, MockRoute};
use imposter_core::model::{BodyType, ParamItem, RequestTemplate, Variables};

use super::*;

struct Fixed(u64);

impl Entropy for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp(1_700_000_000_000)
    }
    fn utc_offset(&self) -> i32 {
        0
    }
    fn random(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0
    }
}

fn host() -> Host {
    Host {
        entropy: Box::new(Fixed(7)),
        stop: Arc::new(|| None),
    }
}

fn template() -> RequestTemplate {
    RequestTemplate {
        method: "GET".into(),
        url: "https://api.test/items?page=1#top".into(),
        headers: vec![ParamItem::new("Accept", "application/json")],
        body: br#"{"name":"Ann"}"#.to_vec(),
        body_type: BodyType::Json,
        ..RequestTemplate::default()
    }
}

fn response() -> Response {
    Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Set-Cookie".into(), "a=1".into()),
            ("set-cookie".into(), "b=2".into()),
        ],
        body: br#"{"id": 7, "items": [{"n": 1}, {"n": 2}]}"#.to_vec(),
        elapsed: 0.0123,
        ..Response::default()
    }
}

fn vars() -> Variables {
    [("host".to_owned(), "api.test".to_owned())]
        .into_iter()
        .collect()
}

fn before_ok(source: &str) -> (RequestTemplate, Outcome) {
    let mut request = template();
    let outcome = before(source, &mut request, &vars(), host());
    assert_eq!(outcome.error, None, "{source}");
    (request, outcome)
}

fn after_run(source: &str) -> Outcome {
    after(source, &template(), &response(), &vars(), host())
}

#[test]
fn every_reference_example_runs_in_its_phase() {
    for entry in API {
        if entry.phase.covers(Phase::Before) {
            let mut request = template();
            let outcome = before(entry.example, &mut request, &vars(), host());
            assert_eq!(outcome.error, None, "before: {}", entry.name);
        }
        if entry.phase.covers(Phase::After) {
            let outcome = after_run(entry.example);
            assert_eq!(outcome.error, None, "after: {}", entry.name);
        }
        if entry.phase.covers(Phase::Mock) {
            let run = mock_run(entry.example, &mut MockState::default());
            assert!(run.reply.is_ok(), "mock: {}: {:?}", entry.name, run.reply);
        }
    }
}

#[test]
fn the_request_can_be_changed_before_sending() {
    let (request, _) = before_ok(
        r#"
        request.method = "PUT";
        request.set_header("accept", "text/plain");
        request.add_header("X-A", "1");
        request.add_header("X-A", "2");
        request.remove_header("x-a");
        request.set_header("X-Sig", hex(hmac_sha256("k", request.body)));
        "#,
    );
    assert_eq!(request.method, "PUT");
    let headers: Vec<(&str, &str)> = request
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(headers[0], ("accept", "text/plain"), "names ignore case");
    assert!(headers.iter().all(|(n, _)| !n.eq_ignore_ascii_case("x-a")));
    assert_eq!(headers.last().unwrap().0, "X-Sig");
    assert_eq!(headers.last().unwrap().1.len(), 64);

    let (request, _) = before_ok(r#"let r = request; r.set_header("X-Copy", "same request");"#);
    assert!(
        request.headers.iter().any(|h| h.name == "X-Copy"),
        "a copy edits the request"
    );
}

#[test]
fn queries_change_with_the_url() {
    let (request, _) = before_ok(
        r#"
        expect(request.query("page"), "1");
        request.set_query("page", "2");
        request.set_query("q", "a b");
        "#,
    );
    assert_eq!(request.url, "https://api.test/items?page=2&q=a+b#top");
    let (request, _) = before_ok(r#"request.remove_query("page");"#);
    assert_eq!(request.url, "https://api.test/items#top");
    let (request, _) =
        before_ok(r#"request.url = "https://other.test/x?a=1"; expect(request.query("a"), "1");"#);
    assert!(request.queries.is_empty(), "the new URL carries its query");
}

#[test]
fn bodies_are_text_and_only_raw_ones_change() {
    let (request, _) =
        before_ok(r#"let data = request.json(); data.name = "Bo"; request.body = to_json(data);"#);
    assert_eq!(request.body, br#"{"name":"Bo"}"#);
    let mut empty = RequestTemplate {
        body_type: BodyType::None,
        ..template()
    };
    let outcome = before(r#"request.body = "hi";"#, &mut empty, &vars(), host());
    assert_eq!(outcome.error, None);
    assert_eq!(
        (empty.body_type, empty.body.as_slice()),
        (BodyType::Plain, &b"hi"[..])
    );

    let mut form = RequestTemplate {
        body_type: BodyType::Multipart,
        ..template()
    };
    let outcome = before(r#"request.body = "x";"#, &mut form, &vars(), host());
    let error = outcome.error.unwrap().to_string();
    assert!(error.contains("multipart body cannot be set"), "{error}");
    let outcome = before(
        "if request.body != () { throw 1 }",
        &mut form,
        &vars(),
        host(),
    );
    assert_eq!(outcome.error, None, "a form body reads as ()");
}

#[test]
fn a_failed_before_script_leaves_the_request_alone() {
    let mut request = template();
    let outcome = before(
        "request.set_header(\"X\", \"1\");\nthrow \"no key\";",
        &mut request,
        &vars(),
        host(),
    );
    assert_eq!(
        outcome.error,
        Some(ScriptError::Run {
            line: Some(2),
            message: "no key".into()
        })
    );
    assert_eq!(request, template());
    assert!(
        before(
            "request.method = \"BAD METHOD\";",
            &mut request,
            &vars(),
            host()
        )
        .error
        .is_some()
    );
}

#[test]
fn after_scripts_read_the_response_and_run_tests() {
    let outcome = after_run(
        r#"
        test("status", response.status == 200);
        test("id", || { expect(response.path("$.id"), 8) });
        test("items", || response.path("$.items[*].n").len() == 2);
        test("false closure", || false);
        test("cookies", || { expect(response.header("SET-COOKIE"), "a=1, b=2") });
        test("map", || { expect(response.headers["content-type"], "application/json") });
        test("time", || { expect(response.time_ms, 12) });
        test("missing", || response.path("$.nope") == ());
        "#,
    );
    assert_eq!(outcome.error, None);
    let results: Vec<(&str, bool, &str)> = outcome
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.passed, t.message.as_str()))
        .collect();
    assert_eq!(
        results,
        [
            ("status", true, ""),
            ("id", false, "expected 8, got 7"),
            ("items", true, ""),
            ("false closure", false, "false"),
            ("cookies", true, ""),
            ("map", true, ""),
            ("time", true, ""),
            ("missing", true, ""),
        ]
    );
    let outcome = after_run(r#"request.set_header("X", "1");"#);
    assert!(outcome.error.unwrap().to_string().contains("already sent"));
    let mut request = template();
    let outcome = before(r#"test("x", true);"#, &mut request, &vars(), host());
    assert!(
        outcome
            .error
            .unwrap()
            .to_string()
            .contains("after-response script")
    );
}

#[test]
fn a_cut_body_says_so_instead_of_reading_as_broken_json() {
    let cut = Response {
        truncated: true,
        ..response()
    };
    let outcome = after("response.json();", &template(), &cut, &vars(), host());
    let error = outcome.error.unwrap().to_string();
    assert!(
        error.to_lowercase().contains("cut") || error.to_lowercase().contains("truncat"),
        "{error}"
    );
}

#[test]
fn variables_read_the_snapshot_and_this_runs_sets() {
    let (_, outcome) = before_ok(
        r#"
        expect(vars.get("host"), "api.test");
        expect(vars.get("missing"), ());
        vars.set("token", "abc");
        vars.set("count", 3);
        expect(vars.get("token"), "abc");
        "#,
    );
    assert_eq!(
        outcome.sets,
        [
            ("token".to_owned(), "abc".to_owned()),
            ("count".to_owned(), "3".to_owned())
        ]
    );
    let mut request = template();
    assert!(
        before(r#"vars.set("", "x");"#, &mut request, &vars(), host())
            .error
            .is_some()
    );
}

#[test]
fn digests_and_hmacs_match_known_vectors() {
    // RFC 4231, test case 2.
    let outcome = after_run(
        r#"
        expect(hex(hmac_sha256("Jefe", "what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        expect(hex(sha256("abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        expect(hex(sha1("abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        expect(hex(md5("abc")), "900150983cd24fb0d6963f7d28e17f72");
        let chained = hmac_sha256(hmac_sha256("AWS4key", "20260101"), "us-east-1");
        expect(hex(chained).len(), 64);
        expect(base64url_encode("??>"), "Pz8-");
        expect(base64_encode("hi"), "aGk=");
        "#,
    );
    assert_eq!(outcome.error, None);
    let outcome = after_run("sha256(42);");
    assert!(outcome.error.unwrap().to_string().contains("text or bytes"));
}

#[test]
fn generated_values_come_from_the_host() {
    let (_, outcome) = before_ok(
        r"print(uuid()); print(now_ms()); print(now_iso()); let n = random_int(1, 3); if n < 1 || n > 3 { throw n }",
    );
    assert_eq!(outcome.console[1], "1700000000000");
    assert_eq!(outcome.console[2], "2023-11-14T22:13:20.000Z");
    assert_eq!(outcome.console[0].len(), 36);
}

#[test]
fn limits_stop_runaway_scripts() {
    let stopped = |source: &str| -> String {
        let mut request = template();
        match before(source, &mut request, &vars(), host()).error {
            Some(ScriptError::Stopped(message)) => message,
            other => panic!("{source}: {other:?}"),
        }
    };
    assert_eq!(
        stopped("loop {}"),
        "Stopped: used more than 1,000,000 operations"
    );
    assert!(stopped(r#"let s = "x"; loop { s += s; }"#).contains("grew past the limit"));
    assert!(stopped("fn f(n) { f(n + 1) } f(0);").contains("too deeply"));

    let flag = Arc::new(AtomicBool::new(false));
    let seen = flag.clone();
    let stopping = Host {
        entropy: Box::new(Fixed(1)),
        stop: Arc::new(move || seen.load(Ordering::SeqCst).then_some(Stop::Cancelled)),
    };
    flag.store(true, Ordering::SeqCst);
    let mut request = template();
    let outcome = before(
        "let i = 0; while i < 100000 { i += 1; }",
        &mut request,
        &vars(),
        stopping,
    );
    assert_eq!(
        outcome.error,
        Some(ScriptError::Stopped(
            "Stopped: the send was cancelled".into()
        ))
    );
    let deadline = Host {
        entropy: Box::new(Fixed(1)),
        stop: Arc::new(|| Some(Stop::TimeLimit)),
    };
    let outcome = before(
        "let i = 0; while i < 100000 { i += 1; }",
        &mut request,
        &vars(),
        deadline,
    );
    assert_eq!(
        outcome.error,
        Some(ScriptError::Stopped(
            "Stopped: the script took too long".into()
        ))
    );
}

#[test]
fn closures_cannot_hide_a_limit_as_a_failed_test() {
    let outcome = after_run(r#"test("spin", || { loop {} });"#);
    assert!(matches!(outcome.error, Some(ScriptError::Stopped(_))));
    assert_eq!(outcome.tests, []);
}

#[test]
fn eval_and_import_are_not_available() {
    assert!(check(r#"eval("1")"#).is_err());
    assert!(check(r#"import "file" as f;"#).is_err());
    let long = "1;".repeat(MAX_SOURCE / 2 + 1);
    assert_eq!(check(&long), Err(ScriptError::TooLong));
}

#[test]
fn compile_errors_point_at_their_line_and_column() {
    match check("let a = 1;\nlet b = ;") {
        Err(ScriptError::Compile { line, column, .. }) => assert_eq!((line, column), (2, 9)),
        other => panic!("{other:?}"),
    }
    assert_eq!(check("let a = 1;"), Ok(()));
}

#[test]
fn the_console_is_capped() {
    let (_, outcome) = before_ok("for i in 0..1000 { print(i); }");
    assert_eq!(outcome.console.len(), MAX_CONSOLE_LINES + 1);
    assert_eq!(outcome.console.last().unwrap(), "… output cut");
    let (_, outcome) = before_ok(r#"debug("x");"#);
    assert_eq!(outcome.console, ["\"x\""]);
}

#[test]
fn the_readme_lists_the_reference_as_the_app_does() {
    let readme = include_str!("../../../README.md");
    let rows: Vec<&str> = readme
        .lines()
        .skip_while(|line| !line.starts_with("| When | Use | What it does |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .collect();
    assert_eq!(
        rows,
        readme_rows(),
        "update the README's script table from imposter_script::API"
    );
}

fn mock_request() -> MockRequest {
    MockRequest {
        method: "POST".into(),
        path: "/orders".into(),
        query: "page=2&q=a+b".into(),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Tag".into(), "a".into()),
            ("x-tag".into(), "b".into()),
        ],
        body: br#"{"item": "pen", "qty": 2}"#.to_vec(),
    }
}

fn static_reply() -> MockReply {
    MockReply::from_route(&MockRoute {
        status: 200,
        headers: vec![ParamItem::new("X-Static", "1")],
        body: "static".into(),
        ..MockRoute::default()
    })
}

fn mock_run(source: &str, state: &mut MockState) -> MockRun {
    mock(source, &mock_request(), static_reply(), state, host())
}

fn reply(source: &str) -> MockReply {
    let run = mock_run(source, &mut MockState::default());
    run.reply.unwrap_or_else(|e| panic!("{source}: {e}"))
}

#[test]
fn a_mock_reads_the_request_and_shapes_the_answer() {
    let answer = reply(
        r#"
        expect(request.method, "POST");
        expect(request.path, "/orders");
        expect(request.query("q"), "a b");
        expect(request.header("x-tag"), "a, b");
        expect(request.headers["content-type"], "application/json");
        let order = request.json();
        response.status = 201;
        response.set_header("Location", "/orders/" + order.item);
        response.remove_header("x-static");
        response.body = request.body;
        "#,
    );
    assert_eq!(answer.status, 201);
    assert_eq!(
        answer.headers,
        [("Location".to_owned(), "/orders/pen".to_owned())]
    );
    assert_eq!(answer.body, mock_request().body, "echoed");

    let json = reply(r"response.send_json(#{ ok: true });");
    assert_eq!(json.body, br#"{"ok":true}"#);
    assert!(
        json.headers
            .contains(&("Content-Type".to_owned(), "application/json".to_owned()))
    );
    let unchanged = reply("let a = 1;");
    assert_eq!(
        unchanged,
        static_reply(),
        "a script that changes nothing gives the static answer"
    );
}

#[test]
fn mock_state_lasts_between_requests_only_when_a_script_succeeds() {
    let mut state = MockState::default();
    let script = r"state.calls = (state.calls ?? 0) + 1;
        if state.calls == 1 { response.status = 500; }
        response.body = `${state.calls}`;";
    assert_eq!(mock_run(script, &mut state).reply.unwrap().status, 500);
    let second = mock_run(script, &mut state).reply.unwrap();
    assert_eq!((second.status, second.body.as_slice()), (200, &b"2"[..]));
    let failing = r#"state.calls = 100; throw "boom";"#;
    assert!(mock_run(failing, &mut state).reply.is_err());
    assert_eq!(
        mock_run(script, &mut state).reply.unwrap().body,
        b"3",
        "the failed run left state alone"
    );
    assert!(
        mock_run("state = 1;", &mut state).reply.is_err(),
        "state stays a map"
    );
}

#[test]
fn a_mock_cannot_send_what_the_server_will_not() {
    let error = |source: &str| {
        mock_run(source, &mut MockState::default())
            .reply
            .unwrap_err()
            .to_string()
    };
    assert!(error("response.status = 99;").contains("between 200 and 599"));
    assert!(error("response.delay_ms = 20000;").contains("10,000"));
    assert!(
        error(r#"response.set_header("Content-Length", "1");"#).contains("sets this header itself")
    );
    assert!(error(r#"response.set_header("Bad Name", "1");"#).contains("one word"));
    assert!(error(r#"test("x", true);"#).contains("request scripts"));
    assert!(error(r#"vars.get("x");"#).contains("vars"));
    assert!(error("loop {}").contains("operations"));
    assert_eq!(
        reply("response.delay_ms = 250;").delay,
        std::time::Duration::from_millis(250)
    );
}

#[test]
fn a_mock_keeps_its_console_even_when_it_fails() {
    let run = mock_run(
        r#"print("before"); throw "after";"#,
        &mut MockState::default(),
    );
    assert_eq!(run.console, ["before"]);
    assert_eq!(
        run.reply.unwrap_err().describe("Mock script"),
        "Mock script, line 1: after"
    );
}

#[test]
fn a_script_compiled_once_runs_like_one_compiled_each_time() {
    let source = r#"request.set_header("X-Id", uuid()); vars.set("n", 1);"#;
    let script = compile(source).unwrap();
    let mut first = template();
    let mut second = template();
    let once = before_compiled(&script, &mut first, &vars(), host());
    let again = before_compiled(&script, &mut second, &vars(), host());
    let mut plain = template();
    let fresh = before(source, &mut plain, &vars(), host());
    assert_eq!((once.error.clone(), again.error.clone()), (None, None));
    assert_eq!(first, plain, "the same engine work, compiled or not");
    assert_eq!(once.sets, fresh.sets);
    let after_script = compile(r#"test("ok", response.status == 200);"#).unwrap();
    let tested = after_compiled(&after_script, &template(), &response(), &vars(), host());
    assert_eq!(tested.tests.len(), 1);
    assert!(tested.tests[0].passed);
}

#[test]
fn compiling_reports_where_it_failed() {
    match compile("let a = 1;\nlet b = ;") {
        Err(ScriptError::Compile { line, column, .. }) => assert_eq!((line, column), (2, 9)),
        other => panic!("{other:?}"),
    }
    let long = "1;".repeat(MAX_SOURCE / 2 + 1);
    assert!(matches!(compile(&long), Err(ScriptError::TooLong)));
    let mut request = template();
    let outcome = before("let = ;", &mut request, &vars(), host());
    assert!(matches!(outcome.error, Some(ScriptError::Compile { .. })));
    assert_eq!(request, template(), "nothing ran");
}
