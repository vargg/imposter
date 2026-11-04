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
    assert!(!report.stopped && !report.timed_out);
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
