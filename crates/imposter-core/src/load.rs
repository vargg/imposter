//! Load runs, as `ab` and JMeter's Aggregate Report shape them: what a run is asked to do
//! ([`LoadPlan`]), what each request measured ([`Sample`]), and what the run adds up to
//! ([`Summary`]). Pure: the times come in measured.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::check::{CheckResult, Verdict};

pub const MAX_REQUESTS: u32 = 1_000_000;
pub const MAX_CONCURRENCY: u16 = 256;
pub const MAX_TIME_LIMIT: Duration = Duration::from_hours(1);
pub const MAX_RAMP_UP: Duration = Duration::from_mins(10);

/// The "served within" rows of `ab`'s report, in percent.
pub const SERVED_WITHIN: [u8; 9] = [50, 66, 75, 80, 90, 95, 98, 99, 100];

/// Why a plan cannot run; each names the field to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("Requests must be between 1 and 1,000,000")]
    Requests,
    #[error("Concurrency must be between 1 and 256, and no more than the requests")]
    Concurrency,
    #[error("The time limit must be between 1 second and 1 hour")]
    TimeLimit,
    #[error("Ramp-up must be at most 10 minutes, and shorter than the time limit")]
    RampUp,
}

/// What a load run does: `requests` in total, by `concurrency` workers, each starting at
/// its point in the ramp-up, until all are sent or the time limit passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadPlan {
    requests: u32,
    concurrency: u16,
    time_limit: Option<Duration>,
    ramp_up: Duration,
    keep_alive: bool,
}

impl LoadPlan {
    pub fn new(
        requests: u32,
        concurrency: u16,
        time_limit: Option<Duration>,
        ramp_up: Duration,
        keep_alive: bool,
    ) -> Result<Self, PlanError> {
        if !(1..=MAX_REQUESTS).contains(&requests) {
            return Err(PlanError::Requests);
        }
        if !(1..=MAX_CONCURRENCY).contains(&concurrency) || u32::from(concurrency) > requests {
            return Err(PlanError::Concurrency);
        }
        if time_limit.is_some_and(|t| t < Duration::from_secs(1) || t > MAX_TIME_LIMIT) {
            return Err(PlanError::TimeLimit);
        }
        if ramp_up > MAX_RAMP_UP || time_limit.is_some_and(|t| ramp_up >= t) {
            return Err(PlanError::RampUp);
        }
        Ok(Self {
            requests,
            concurrency,
            time_limit,
            ramp_up,
            keep_alive,
        })
    }

    pub const fn requests(&self) -> u32 {
        self.requests
    }

    pub const fn concurrency(&self) -> u16 {
        self.concurrency
    }

    pub const fn time_limit(&self) -> Option<Duration> {
        self.time_limit
    }

    pub const fn ramp_up(&self) -> Duration {
        self.ramp_up
    }

    pub const fn keep_alive(&self) -> bool {
        self.keep_alive
    }

    /// When worker `worker` (from 0) starts after the run does: spread evenly over the
    /// ramp-up, the first at once, as JMeter starts its threads.
    pub fn start_offset(&self, worker: u16) -> Duration {
        self.ramp_up * u32::from(worker) / u32::from(self.concurrency)
    }
}

/// How one request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleOutcome {
    Status(u16),
    /// A transport error, by its stable kind (`Timeout`, `ConnectError`…).
    Error(&'static str),
}

/// Check verdicts, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckTally {
    pub passed: u64,
    pub failed: u64,
    pub unchecked: u64,
}

impl CheckTally {
    pub fn of(results: &[CheckResult]) -> Self {
        let mut tally = Self::default();
        for result in results {
            match result.verdict {
                Verdict::Passed => tally.passed += 1,
                Verdict::Failed => tally.failed += 1,
                Verdict::Unchecked(_) => tally.unchecked += 1,
            }
        }
        tally
    }

    fn add(&mut self, other: Self) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.unchecked += other.unchecked;
    }
}

/// What one request measured. Times are microseconds, so a million samples stay small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// From sending to the last byte of the body.
    pub total_us: u32,
    /// From sending to the response's head (JMeter's "latency"); none without a response.
    pub first_byte_us: Option<u32>,
    pub outcome: SampleOutcome,
    /// Body bytes received.
    pub bytes: u64,
    pub checks: CheckTally,
}

impl Sample {
    /// Whether it counts as an error: a transport error, or a response failing a check,
    /// as JMeter counts a failed assertion.
    pub const fn is_error(&self) -> bool {
        matches!(self.outcome, SampleOutcome::Error(_)) || self.checks.failed > 0
    }
}

/// Microseconds in a `u32`, saturating (about 71 minutes, past any time limit).
pub fn micros(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(u32::MAX)
}

/// Latency figures over a set of times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Latency {
    pub min: Duration,
    pub mean: Duration,
    pub p50: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
}

/// One bar of the latency histogram: the responses that took less than `upper` and at
/// least the bucket before's upper bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub upper: Duration,
    pub count: u64,
}

/// What a run adds up to. Latency counts responses only: a refused connection takes no
/// time worth averaging, and is counted under errors instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Requests that finished, with a response or an error.
    pub completed: u64,
    pub wall: Duration,
    pub per_second: f64,
    pub bytes: u64,
    pub bytes_per_second: f64,
    /// Transport errors and responses failing a check.
    pub errors: u64,
    pub total: Option<Latency>,
    pub first_byte: Option<Latency>,
    /// `ab`'s table: the time within which each percentage of responses was served.
    pub served_within: Vec<(u8, Duration)>,
    pub statuses: BTreeMap<u16, u64>,
    pub error_kinds: BTreeMap<&'static str, u64>,
    pub checks: CheckTally,
    pub histogram: Vec<Bucket>,
}

impl Summary {
    pub fn from_samples(samples: &[Sample], wall: Duration) -> Self {
        let mut statuses = BTreeMap::new();
        let mut error_kinds = BTreeMap::new();
        let mut checks = CheckTally::default();
        let mut totals = Vec::with_capacity(samples.len());
        let mut first_bytes = Vec::with_capacity(samples.len());
        let mut bytes = 0_u64;
        for sample in samples {
            bytes += sample.bytes;
            checks.add(sample.checks);
            match sample.outcome {
                SampleOutcome::Status(status) => {
                    *statuses.entry(status).or_insert(0) += 1;
                    totals.push(sample.total_us);
                    first_bytes.extend(sample.first_byte_us);
                }
                SampleOutcome::Error(kind) => *error_kinds.entry(kind).or_insert(0) += 1,
            }
        }
        totals.sort_unstable();
        first_bytes.sort_unstable();
        let completed = samples.len() as u64;
        let seconds = wall.as_secs_f64();
        let rate = |count: u64| {
            #[allow(clippy::cast_precision_loss)] // counts far below 2^52
            let count = count as f64;
            if seconds > 0.0 { count / seconds } else { 0.0 }
        };
        Self {
            completed,
            wall,
            per_second: rate(completed),
            bytes,
            bytes_per_second: rate(bytes),
            errors: samples.iter().filter(|s| s.is_error()).count() as u64,
            total: latency(&totals),
            first_byte: latency(&first_bytes),
            served_within: if totals.is_empty() {
                Vec::new()
            } else {
                SERVED_WITHIN
                    .iter()
                    .map(|p| (*p, at_percentile(&totals, *p)))
                    .collect()
            },
            statuses,
            error_kinds,
            checks,
            histogram: histogram(&totals),
        }
    }

    /// Errors as a share of completed requests, from 0 to 1.
    pub fn error_rate(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)] // counts far below 2^52
        if self.completed == 0 {
            0.0
        } else {
            self.errors as f64 / self.completed as f64
        }
    }
}

fn us(micros: u32) -> Duration {
    Duration::from_micros(u64::from(micros))
}

/// The nearest-rank percentile of sorted, non-empty `values`.
fn at_percentile(sorted: &[u32], percent: u8) -> Duration {
    let n = sorted.len();
    let rank = (n * usize::from(percent)).div_ceil(100).max(1);
    us(sorted[rank.min(n) - 1])
}

fn latency(sorted: &[u32]) -> Option<Latency> {
    let (first, last) = (sorted.first()?, sorted.last()?);
    let sum: u64 = sorted.iter().map(|v| u64::from(*v)).sum();
    Some(Latency {
        min: us(*first),
        mean: Duration::from_micros(sum / sorted.len() as u64),
        p50: at_percentile(sorted, 50),
        p90: at_percentile(sorted, 90),
        p95: at_percentile(sorted, 95),
        p99: at_percentile(sorted, 99),
        max: us(*last),
    })
}

/// Bucket bounds in milliseconds, 1-2-5 steps; the last bucket holds everything slower.
const BOUNDS_MS: [u64; 13] = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000];

/// Responses counted into log-scale buckets, from the first non-empty one to the last.
fn histogram(sorted: &[u32]) -> Vec<Bucket> {
    let mut buckets: Vec<Bucket> = BOUNDS_MS
        .iter()
        .map(|ms| Bucket {
            upper: Duration::from_millis(*ms),
            count: 0,
        })
        .chain(std::iter::once(Bucket {
            upper: Duration::MAX,
            count: 0,
        }))
        .collect();
    for value in sorted {
        let at = us(*value);
        if let Some(bucket) = buckets.iter_mut().find(|b| at < b.upper) {
            bucket.count += 1;
        }
    }
    let first = buckets.iter().position(|b| b.count > 0);
    let last = buckets.iter().rposition(|b| b.count > 0);
    match (first, last) {
        (Some(first), Some(last)) => buckets[first..=last].to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn response(total_ms: u32, status: u16) -> Sample {
        Sample {
            total_us: total_ms * 1000,
            first_byte_us: Some(total_ms * 500),
            outcome: SampleOutcome::Status(status),
            bytes: 100,
            checks: CheckTally::default(),
        }
    }

    #[test]
    fn plans_are_checked() {
        let plan = |requests, concurrency, limit: Option<u64>, ramp: u64| {
            LoadPlan::new(
                requests,
                concurrency,
                limit.map(Duration::from_secs),
                Duration::from_secs(ramp),
                true,
            )
        };
        assert!(plan(100, 10, None, 0).is_ok());
        assert!(plan(1, 1, Some(1), 0).is_ok());
        assert_eq!(plan(0, 1, None, 0), Err(PlanError::Requests));
        assert_eq!(plan(1_000_001, 1, None, 0), Err(PlanError::Requests));
        assert_eq!(plan(10, 0, None, 0), Err(PlanError::Concurrency));
        assert_eq!(plan(10, 11, None, 0), Err(PlanError::Concurrency));
        assert_eq!(plan(1000, 257, None, 0), Err(PlanError::Concurrency));
        assert_eq!(plan(10, 1, Some(0), 0), Err(PlanError::TimeLimit));
        assert_eq!(plan(10, 1, Some(3601), 0), Err(PlanError::TimeLimit));
        assert_eq!(plan(10, 1, Some(10), 10), Err(PlanError::RampUp));
        assert_eq!(plan(10, 1, None, 601), Err(PlanError::RampUp));
    }

    #[test]
    fn workers_start_spread_over_the_ramp_up() {
        let plan = LoadPlan::new(100, 4, None, Duration::from_secs(2), true).unwrap();
        let starts: Vec<Duration> = (0..4).map(|w| plan.start_offset(w)).collect();
        assert_eq!(starts, [ms(0), ms(500), ms(1000), ms(1500)]);
        let at_once = LoadPlan::new(100, 4, None, Duration::ZERO, true).unwrap();
        assert_eq!(at_once.start_offset(3), Duration::ZERO);
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let samples: Vec<Sample> = (1..=100).map(|ms| response(ms, 200)).collect();
        let summary = Summary::from_samples(&samples, Duration::from_secs(2));
        let total = summary.total.unwrap();
        assert_eq!(
            (
                total.min, total.p50, total.p90, total.p95, total.p99, total.max
            ),
            (ms(1), ms(50), ms(90), ms(95), ms(99), ms(100))
        );
        assert_eq!(total.mean, Duration::from_micros(50_500));
        assert_eq!(summary.first_byte.unwrap().p50, ms(25));
        assert_eq!(summary.served_within[0], (50, ms(50)));
        assert_eq!(summary.served_within[1], (66, ms(66)));
        assert_eq!(*summary.served_within.last().unwrap(), (100, ms(100)));
        assert!((summary.per_second - 50.0).abs() < 1e-9);
        assert!((summary.bytes_per_second - 5000.0).abs() < 1e-9);

        let one = Summary::from_samples(&[response(7, 200)], Duration::from_secs(1));
        let only = one.total.unwrap();
        assert_eq!((only.p50, only.p99, only.max), (ms(7), ms(7), ms(7)));
    }

    #[test]
    fn errors_count_transport_failures_and_failed_checks() {
        let mut failing = response(10, 200);
        failing.checks = CheckTally {
            passed: 1,
            failed: 1,
            unchecked: 0,
        };
        let refused = Sample {
            total_us: 300,
            first_byte_us: None,
            outcome: SampleOutcome::Error("ConnectError"),
            bytes: 0,
            checks: CheckTally::default(),
        };
        let samples = [response(10, 200), response(20, 503), failing, refused];
        let summary = Summary::from_samples(&samples, Duration::from_secs(1));
        assert_eq!(summary.completed, 4);
        assert_eq!(summary.errors, 2);
        assert!((summary.error_rate() - 0.5).abs() < 1e-9);
        assert_eq!(summary.statuses, BTreeMap::from([(200, 2), (503, 1)]));
        assert_eq!(summary.error_kinds, BTreeMap::from([("ConnectError", 1)]));
        assert_eq!(summary.checks.failed, 1);
        assert_eq!(
            summary.total.unwrap().min,
            ms(10),
            "the refusal is not a latency"
        );
    }

    #[test]
    fn nothing_but_errors_has_no_latency() {
        let refused = Sample {
            total_us: 300,
            first_byte_us: None,
            outcome: SampleOutcome::Error("ConnectError"),
            bytes: 0,
            checks: CheckTally::default(),
        };
        let summary = Summary::from_samples(&[refused], Duration::ZERO);
        assert_eq!((summary.total, summary.first_byte), (None, None));
        assert!(summary.served_within.is_empty() && summary.histogram.is_empty());
        assert!(summary.per_second.abs() < f64::EPSILON);
        let empty = Summary::from_samples(&[], Duration::from_secs(1));
        assert!(empty.error_rate().abs() < f64::EPSILON);
    }

    #[test]
    fn the_histogram_spans_the_used_buckets() {
        let samples = [response(3, 200), response(4, 200), response(15, 200)];
        let summary = Summary::from_samples(&samples, Duration::from_secs(1));
        let bars: Vec<(Duration, u64)> = summary
            .histogram
            .iter()
            .map(|b| (b.upper, b.count))
            .collect();
        assert_eq!(bars, [(ms(5), 2), (ms(10), 0), (ms(20), 1)]);
        let slow = Summary::from_samples(&[response(20_000, 200)], Duration::from_secs(30));
        assert_eq!(slow.histogram[0].upper, Duration::MAX);
        assert_eq!(micros(Duration::from_secs(5000)), u32::MAX);
    }
}
