//! Load runs end to end: a real database, a keep-alive loopback server, and the summary.
#![cfg(test)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use imposter_app::executor::ExecuteError;
use imposter_app::load::{Progress, Report, run};
use imposter_core::capture::Capture;
use imposter_core::check::{Check, Op};
use imposter_core::load::LoadPlan;
use imposter_core::model::{ParamItem, Request, RequestTemplate, Variables};
use imposter_core::select::Selector;
use imposter_store::{Store, collections, environments, executions};
use imposter_testkit::{LoadReply, LoadServer};
use tokio_util::sync::CancellationToken;

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("imposter.db")).unwrap();
    (dir, store)
}

async fn saved(store: &Store, template: RequestTemplate) -> Request {
    store
        .call(move |db| {
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::create_request(db, collection.id, None, &template)
        })
        .await
        .unwrap()
}

fn get(url: &str) -> RequestTemplate {
    RequestTemplate {
        name: "probe".into(),
        url: url.to_owned(),
        ..RequestTemplate::default()
    }
}

fn plan(requests: u32, concurrency: u16, keep_alive: bool) -> LoadPlan {
    LoadPlan::new(requests, concurrency, None, Duration::ZERO, keep_alive).unwrap()
}

async fn go(store: &Store, request: Request, plan: LoadPlan) -> Report {
    run(
        &store.handle(),
        request,
        plan,
        CancellationToken::new(),
        Arc::new(Progress::default()),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn as_many_requests_are_in_flight_as_the_concurrency() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply {
        gate: 4,
        ..LoadReply::ok(b"ok")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let progress = Arc::new(Progress::default());
    let report = run(
        &store.handle(),
        request,
        plan(12, 4, true),
        CancellationToken::new(),
        progress.clone(),
    )
    .await
    .unwrap();
    assert_eq!(server.peak(), 4);
    assert_eq!(report.summary.completed, 12);
    assert_eq!(report.summary.statuses.get(&200), Some(&12));
    assert_eq!(report.summary.bytes, 24);
    assert_eq!((progress.done(), progress.errors()), (12, 0));
    assert!(!report.stopped && !report.timed_out && report.script_error.is_none());
}

#[tokio::test]
async fn keep_alive_reuses_connections_and_its_absence_does_not() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"ok")).await;
    let request = saved(&store, get(&server.url)).await;
    go(&store, request.clone(), plan(40, 2, true)).await;
    // Reused, not one per request. The pool can open one more while a finished connection
    // is on its way back to it, so "exactly the concurrency" is not promised.
    assert!(
        server.connections() < 10,
        "{} connections",
        server.connections()
    );

    let fresh = LoadServer::start(|_, _| LoadReply::ok(b"ok")).await;
    let mut other = request;
    other.template.url.clone_from(&fresh.url);
    go(&store, other, plan(40, 2, false)).await;
    assert_eq!(fresh.connections(), 40);
}

#[tokio::test]
async fn every_request_gets_fresh_generated_values() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let mut template = get(&server.url);
    template.headers = vec![ParamItem::new("X-Id", "{{$uuid}}")];
    let request = saved(&store, template).await;
    go(&store, request, plan(10, 2, true)).await;
    let ids: HashSet<String> = server
        .requests()
        .iter()
        .map(|(_, r)| r.header("x-id").unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 10);
}

#[tokio::test]
async fn workers_start_spread_over_the_ramp_up() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_millis(300),
        ..LoadReply::ok(b"")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let ramped = LoadPlan::new(3, 3, None, Duration::from_millis(600), true).unwrap();
    go(&store, request, ramped).await;
    let arrivals: Vec<_> = server.requests().iter().map(|(at, _)| *at).collect();
    let spread = *arrivals.iter().max().unwrap() - *arrivals.iter().min().unwrap();
    // Workers start at 0, 200 and 400 ms; the first is busy until 300 ms.
    assert!(spread >= Duration::from_millis(150), "spread {spread:?}");
}

#[tokio::test]
async fn the_first_byte_is_timed_apart_from_the_body() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_millis(60),
        ..LoadReply::ok(b"late")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let report = go(&store, request, plan(2, 2, true)).await;
    let (total, first) = (
        report.summary.total.unwrap(),
        report.summary.first_byte.unwrap(),
    );
    assert!(
        total.min.saturating_sub(first.max) >= Duration::from_millis(40),
        "{total:?} {first:?}"
    );
}

#[tokio::test]
async fn stopping_ends_the_run_and_keeps_what_finished() {
    let (_dir, store) = open();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let server = LoadServer::start(move |index, _| {
        if index == 5 {
            trigger.cancel();
        }
        LoadReply::ok(b"ok")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let report = run(
        &store.handle(),
        request,
        plan(100_000, 2, true),
        cancel,
        Arc::new(Progress::default()),
    )
    .await
    .unwrap();
    assert!(report.stopped);
    assert!(report.summary.completed >= 4 && report.summary.completed < 100);
}

#[tokio::test]
async fn the_time_limit_ends_the_run() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_millis(20),
        ..LoadReply::ok(b"ok")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let limited = LoadPlan::new(
        1_000_000,
        2,
        Some(Duration::from_secs(1)),
        Duration::ZERO,
        true,
    )
    .unwrap();
    let report = go(&store, request, limited).await;
    assert!(report.timed_out && !report.stopped);
    assert!(report.summary.wall < Duration::from_secs(3));
    assert!(report.summary.completed > 0);
}

#[tokio::test]
async fn timeouts_and_failed_checks_count_as_errors() {
    let (_dir, store) = open();
    let slow = LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_secs(2),
        ..LoadReply::ok(b"late")
    })
    .await;
    let mut template = get(&slow.url);
    template.options.timeout = Some(0.2);
    let request = saved(&store, template).await;
    let report = go(&store, request, plan(2, 2, true)).await;
    assert_eq!(report.summary.error_kinds.get("Timeout"), Some(&2));
    assert_eq!(report.summary.errors, 2);

    let server = LoadServer::start(|_, _| LoadReply::ok(b"{\"id\": 1}")).await;
    let mut template = get(&server.url);
    template.checks = vec![
        Check::new(Selector::Status, Op::Equals, "201"),
        Check::new(Selector::Json("$.id".into()), Op::Equals, "1"),
    ];
    let request = saved(&store, template).await;
    let report = go(&store, request, plan(4, 2, true)).await;
    assert_eq!(report.summary.errors, 4);
    assert_eq!(
        (report.summary.checks.passed, report.summary.checks.failed),
        (4, 4),
        "the JSON check read the kept body"
    );
}

#[tokio::test]
async fn runs_record_no_history_and_write_no_captures() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"{\"token\": \"t\"}")).await;
    let mut template = get(&server.url);
    template.captures = vec![Capture::new("token", Selector::Json("$.token".into()))];
    let request = saved(&store, template).await;
    go(&store, request, plan(3, 1, true)).await;
    let (history, globals) = store
        .call(|db| Ok((executions::count(db)?, environments::globals(db)?)))
        .await
        .unwrap();
    assert_eq!(history, 0);
    assert!(globals.is_empty());
}

fn scripted(url: &str, before: &str, after: &str) -> RequestTemplate {
    RequestTemplate {
        method: "POST".into(),
        body: br#"{"n":1}"#.to_vec(),
        body_type: imposter_core::model::BodyType::Json,
        scripts: imposter_core::model::Scripts {
            before: before.into(),
            after: after.into(),
        },
        ..get(url)
    }
}

#[tokio::test]
async fn the_before_script_runs_for_every_request() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let template = scripted(
        &server.url,
        r#"request.set_header("X-Id", uuid());
request.set_header("X-Sig", hex(hmac_sha256("k", request.body)));"#,
        "",
    );
    let request = saved(&store, template).await;
    let report = go(&store, request, plan(8, 2, true)).await;
    assert_eq!(report.summary.completed, 8);
    let received = server.requests();
    let ids: HashSet<&str> = received
        .iter()
        .map(|(_, r)| r.header("x-id").unwrap())
        .collect();
    assert_eq!(ids.len(), 8, "each request ran the script afresh");
    assert!(received.iter().all(|(_, r)| r.header("x-sig")
        == Some("d2096d38d6daae75e745b2935b72afa721590494ec70b2477097f53a96f51330")));
}

#[tokio::test]
async fn after_script_tests_count_with_the_checks_and_set_nothing() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"{}")).await;
    let template = scripted(
        &server.url,
        "",
        r#"test("ok", response.status == 200);
test("created", response.status == 201);
vars.set("seen", "yes");"#,
    );
    let request = saved(&store, template).await;
    let report = go(&store, request, plan(4, 2, true)).await;
    let checks = report.summary.checks;
    assert_eq!((checks.passed, checks.failed), (4, 4));
    assert_eq!(
        report.summary.errors, 4,
        "a failed test makes the request an error"
    );
    let (history, globals) = store
        .call(|db| Ok((executions::count(db)?, environments::globals(db)?)))
        .await
        .unwrap();
    assert_eq!((history, globals.len()), (0, 0), "load runs write nothing");
}

#[tokio::test]
async fn an_after_script_error_counts_and_its_first_message_is_kept() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let request = saved(
        &store,
        scripted(&server.url, "", "let a = 1;\nthrow \"bad\";"),
    )
    .await;
    let report = go(&store, request, plan(3, 1, true)).await;
    assert_eq!(report.summary.checks.failed, 3);
    assert_eq!(
        report.script_error.as_deref(),
        Some("After-response script, line 2: bad")
    );
}

#[tokio::test]
async fn a_before_script_that_fails_up_front_stops_the_run_before_anything_is_sent() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    for before in ["throw \"no key\";", "let x = ;"] {
        let request = saved(&store, scripted(&server.url, before, "")).await;
        let result = run(
            &store.handle(),
            request,
            plan(5, 1, true),
            CancellationToken::new(),
            Arc::new(Progress::default()),
        )
        .await;
        assert!(
            matches!(&result, Err(ExecuteError::Script(m)) if m.starts_with("Before-sending script, line 1")),
            "{result:?}"
        );
    }
    assert_eq!(server.requests().len(), 0);
}

#[tokio::test]
async fn a_before_script_failing_on_some_requests_counts_them_as_script_errors() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    // Random by design: a script that always failed would stop the run at its check. The
    // check passes 3 times in 4, so retrying makes a spurious failure about 1e-30 likely,
    // and 200 requests all passing about 1e-25.
    let request = saved(
        &store,
        scripted(
            &server.url,
            r#"if random_int(1, 4) == 1 { throw "unlucky"; }"#,
            "",
        ),
    )
    .await;
    let report = loop {
        match run(
            &store.handle(),
            request.clone(),
            plan(200, 4, true),
            CancellationToken::new(),
            Arc::new(Progress::default()),
        )
        .await
        {
            Ok(report) => break report,
            Err(ExecuteError::Script(_)) => {}
            Err(other) => panic!("{other:?}"),
        }
    };
    let failed = report
        .summary
        .error_kinds
        .get("ScriptError")
        .copied()
        .unwrap_or(0);
    assert!(failed > 0 && failed < 200, "{failed}");
    assert_eq!(
        server.requests().len() as u64,
        200 - failed,
        "nothing sent for a failed script"
    );
    assert_eq!(
        report.script_error.as_deref(),
        Some("Before-sending script, line 1: unlucky")
    );
}

#[tokio::test]
async fn script_time_is_not_counted_as_latency() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let request = saved(
        &store,
        scripted(&server.url, "let i = 0; while i < 60000 { i += 1; }", ""),
    )
    .await;
    let report = go(&store, request, plan(4, 1, true)).await;
    let per_request = report.summary.wall / 4;
    let slowest = report.summary.total.unwrap().max;
    assert!(
        slowest * 2 < per_request,
        "the exchange ({slowest:?}) is a fraction of each request's time with its script ({per_request:?})"
    );
}

#[tokio::test]
async fn a_request_that_cannot_be_sent_stops_the_run_before_sending() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let request = saved(&store, get(&format!("{}/{{{{missing}}}}", server.url))).await;
    let result = run(
        &store.handle(),
        request,
        plan(5, 1, true),
        CancellationToken::new(),
        Arc::new(Progress::default()),
    )
    .await;
    assert!(matches!(result, Err(ExecuteError::Unresolved(names)) if names == ["missing"]));
    assert_eq!(server.requests().len(), 0);
}

#[tokio::test]
async fn a_run_dropped_half_way_shuts_its_runtime_down_quietly() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply {
        body_delay: Duration::from_millis(50),
        ..LoadReply::ok(b"ok")
    })
    .await;
    let request = saved(&store, get(&server.url)).await;
    let handle = store.handle();
    let running = tokio::spawn(async move {
        run(
            &handle,
            request,
            plan(100_000, 2, true),
            CancellationToken::new(),
            Arc::new(Progress::default()),
        )
        .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while server.requests().is_empty() && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    running.abort();
    let ended = running.await.unwrap_err();
    assert!(ended.is_cancelled(), "aborted, not panicked: {ended:?}");
}

#[tokio::test]
async fn a_collection_script_runs_for_every_request_of_a_load_run() {
    let (_dir, store) = open();
    let server = LoadServer::start(|_, _| LoadReply::ok(b"")).await;
    let url = server.url.clone();
    let request = store
        .call(move |db| {
            let collection = collections::create_collection(db, "API", "", &Variables::new())?;
            collections::set_collection_scripts(
                db,
                collection.id,
                &imposter_core::model::Scripts {
                    before: r#"request.set_header("X-Signed", uuid());"#.into(),
                    after: r#"test("ok", response.status == 200);"#.into(),
                },
            )?;
            collections::create_request(db, collection.id, None, &get(&url))
        })
        .await
        .unwrap();
    let report = go(&store, request, plan(6, 2, true)).await;
    let signed: HashSet<String> = server
        .requests()
        .iter()
        .map(|(_, r)| r.header("x-signed").unwrap().to_owned())
        .collect();
    assert_eq!(signed.len(), 6);
    assert_eq!(report.summary.checks.passed, 6);
}
