//! Responses and the self-contained history records of executions.

use serde::{Deserialize, Serialize};

use crate::ids::{EnvironmentId, RequestId};
use crate::model::Request;
use crate::settings::AppSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Response,
    TransportError,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    /// In received order, duplicates kept.
    pub headers: Vec<(String, String)>,
    #[serde(with = "crate::base64_bytes")]
    pub body: Vec<u8>,
    /// Monotonic duration of the exchange, in seconds.
    pub elapsed: f64,
    pub outcome: Outcome,
    /// For failures: a short machine-readable kind ("Timeout", "Connect", …).
    pub error_kind: String,
    /// The body was cut at the capture limit.
    pub truncated: bool,
    /// Set when the response arrived but recording it in history failed.
    pub history_error: String,
}

impl Response {
    pub fn failure(outcome: Outcome, kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            outcome,
            error_kind: kind.into(),
            reason: message.into(),
            ..Self::default()
        }
    }
}

/// Milliseconds since the Unix epoch, UTC.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    pub fn to_rfc3339(self) -> String {
        let millis = self.0.rem_euclid(1000);
        let seconds = self.0.div_euclid(1000);
        let (days, day_seconds) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
            day_seconds / 3600,
            day_seconds % 3600 / 60,
            day_seconds % 60
        )
    }
}

/// Howard Hinnant's algorithm: days since 1970-01-01 to a proleptic Gregorian date.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// One execution, independent of the request it came from (no foreign key): deleting the
/// request keeps its history. Upload contents are never part of it, only their paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub timestamp: Timestamp,
    pub request_id: Option<RequestId>,
    /// What the user had in the editor.
    pub template: Request,
    /// What was sent, after variables and inherited settings were applied.
    pub resolved: Request,
    pub settings: AppSettings,
    pub environment_id: Option<EnvironmentId>,
    pub environment: String,
    pub response: Response,
}

impl ExecutionRecord {
    /// An unsaved draft of the request as it was; restoring never sends.
    pub fn restore(&self) -> Request {
        Request {
            id: None,
            ..self.template.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_format_as_utc() {
        assert_eq!(Timestamp(0).to_rfc3339(), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            Timestamp(951_782_400_123).to_rfc3339(),
            "2000-02-29T00:00:00.123Z"
        );
        assert_eq!(
            Timestamp(1_793_000_000_000).to_rfc3339(),
            "2026-10-26T07:33:20.000Z"
        );
        assert_eq!(Timestamp(-1).to_rfc3339(), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn restore_detaches_identity() {
        let record = ExecutionRecord {
            timestamp: Timestamp(0),
            request_id: Some(RequestId(4)),
            template: Request {
                id: Some(RequestId(4)),
                ..Request::default()
            },
            resolved: Request::default(),
            settings: AppSettings::default(),
            environment_id: None,
            environment: "Global".into(),
            response: Response::default(),
        };
        let draft = record.restore();
        assert_eq!(draft.id, None);
        assert_eq!(draft.template, record.template.template);
    }
}
