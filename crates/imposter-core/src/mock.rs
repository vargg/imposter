//! Local mock responses, static or computed by a route's script. Configuration is independent
//! of request collections.

use serde::{Deserialize, Serialize};

use crate::model::{ParamItem, is_method_token};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockConfig {
    pub port: u16,
    pub routes: Vec<MockRoute>,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            routes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockRoute {
    pub enabled: bool,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub headers: Vec<ParamItem>,
    pub body: String,
    /// A Rhai script computing the response from the request, starting from the static
    /// one; empty for a static route.
    pub script: String,
}

impl MockRoute {
    pub fn is_scripted(&self) -> bool {
        !self.script.trim().is_empty()
    }
}

/// What a scripted route is given about the request. Bodies are read only for these.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MockRequest {
    pub method: String,
    pub path: String,
    /// The raw query, without `?`.
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The longest delay a script may ask for.
pub const MAX_MOCK_DELAY: std::time::Duration = std::time::Duration::from_secs(10);

/// What the server answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MockReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Waited before answering, at most [`MAX_MOCK_DELAY`].
    pub delay: std::time::Duration,
}

impl MockReply {
    /// The route's static answer: where a script starts.
    pub fn from_route(route: &MockRoute) -> Self {
        Self {
            status: route.status,
            headers: route
                .headers
                .iter()
                .filter(|h| h.is_enabled)
                .map(|h| (h.name.clone(), h.value.clone()))
                .collect(),
            body: route.body.as_bytes().to_vec(),
            delay: std::time::Duration::ZERO,
        }
    }

    /// Why the reply cannot be sent, as a script must be told: a status out of range or a
    /// header the server cannot or will not send.
    pub fn problem(&self) -> Option<String> {
        if !(200..=599).contains(&self.status) {
            return Some(format!("Status {} is not between 200 and 599", self.status));
        }
        if self.delay > MAX_MOCK_DELAY {
            return Some("A delay is at most 10,000 ms".to_owned());
        }
        self.headers
            .iter()
            .find_map(|(name, value)| header_problem(name, value).map(|p| format!("{name}: {p}")))
    }
}

/// What a scripted route's responder made of a request: the answer (a 500 naming the
/// error when the script failed), the error, and what the script printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Responded {
    pub reply: MockReply,
    pub error: Option<String>,
    pub console: Vec<String>,
}

impl Responded {
    /// A plain-text 500 saying why the script gave no answer.
    pub fn failed(error: String, console: Vec<String>) -> Self {
        Self {
            reply: MockReply {
                status: 500,
                headers: vec![(
                    "Content-Type".to_owned(),
                    "text/plain; charset=utf-8".to_owned(),
                )],
                body: error.clone().into_bytes(),
                delay: std::time::Duration::ZERO,
            },
            error: Some(error),
            console,
        }
    }
}

/// One answered request, for the Mocks screen's log. Never a body: logs end up in bug
/// reports and screenshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockLogEntry {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub elapsed: std::time::Duration,
    /// Why a script gave no answer.
    pub error: Option<String>,
    pub console: Vec<String>,
}

/// Why a response header cannot be sent: names are tokens, values have no line breaks, and
/// the server sets the framing headers itself.
pub fn header_problem(name: &str, value: &str) -> Option<&'static str> {
    if !is_method_token(name) {
        Some("Header names are one word without spaces")
    } else if value.bytes().any(|b| (b < 32 && b != b'\t') || b == 127) {
        Some("Header values can't contain line breaks")
    } else if [
        "content-length",
        "transfer-encoding",
        "connection",
        "trailer",
        "upgrade",
    ]
    .iter()
    .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        Some("The server sets this header itself")
    } else {
        None
    }
}

impl Default for MockRoute {
    fn default() -> Self {
        Self {
            enabled: true,
            method: "GET".into(),
            path: "/".into(),
            status: 200,
            headers: Vec::new(),
            body: String::new(),
            script: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Route {route}: {reason}")]
pub struct MockConfigError {
    /// Counted from 1, as the screen names routes.
    pub route: usize,
    pub reason: String,
}

/// The part of a route a problem belongs to, so the screen can show it beside that field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteField {
    Method,
    Path,
    Status,
    /// A response header, by its index in the route's list.
    Header(usize),
    /// The response script (it does not compile); found by the caller, which has the
    /// script engine.
    Script,
    /// The route as a whole (it clashes with another).
    Route,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteProblem {
    /// Index in `MockConfig::routes`.
    pub route: usize,
    pub field: RouteField,
    pub reason: String,
}

/// Why `path` cannot be a route path, in words that say what to write instead.
fn path_problem(path: &str) -> Option<&'static str> {
    if !path.starts_with('/') {
        Some("Paths start with /")
    } else if path.contains(' ') {
        Some("Paths can't contain spaces; write %20")
    } else if path.contains(['?', '#']) {
        Some("Leave out ? and #: routes match the path only")
    } else if path.bytes().any(|b| !b.is_ascii() || b < b' ' || b == 127) {
        Some("Encode other characters, for example é as %C3%A9")
    } else {
        None
    }
}

impl MockConfig {
    /// The first problem, as the server and saving refuse a configuration.
    pub fn validate(&self) -> Result<(), MockConfigError> {
        match self.problems().into_iter().next() {
            Some(problem) => Err(MockConfigError {
                route: problem.route + 1,
                reason: problem.reason,
            }),
            None => Ok(()),
        }
    }

    /// Every problem of every route, in route order, for showing beside the fields.
    pub fn problems(&self) -> Vec<RouteProblem> {
        let mut problems = Vec::new();
        let mut keys = std::collections::BTreeSet::new();
        for (index, route) in self.routes.iter().enumerate() {
            let mut problem = |field, reason: &str| {
                problems.push(RouteProblem {
                    route: index,
                    field,
                    reason: reason.to_owned(),
                });
            };
            if !is_method_token(&route.method) {
                problem(
                    RouteField::Method,
                    "Methods are one word without spaces, such as GET",
                );
            }
            if let Some(reason) = path_problem(&route.path) {
                problem(RouteField::Path, reason);
            }
            if !(200..=599).contains(&route.status) {
                problem(RouteField::Status, "Status must be between 200 and 599");
            }
            for (row, header) in route.headers.iter().enumerate() {
                if !header.is_enabled {
                    continue;
                }
                if let Some(reason) = header_problem(&header.name, &header.value) {
                    problem(RouteField::Header(row), reason);
                }
            }
            if route.enabled && !keys.insert((&route.method, &route.path)) {
                problems.push(RouteProblem {
                    route: index,
                    field: RouteField::Route,
                    reason: format!(
                        "Another enabled route already answers {} {}",
                        route.method, route.path
                    ),
                });
            }
        }
        problems
    }

    pub fn matching_route(&self, method: &str, path: &str) -> Option<&MockRoute> {
        self.matching_index(method, path)
            .map(|index| &self.routes[index])
    }

    /// The index of the enabled route answering `method` and `path`.
    pub fn matching_index(&self, method: &str, path: &str) -> Option<usize> {
        self.routes
            .iter()
            .position(|route| route.enabled && route.method == method && route.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_problem_is_reported_beside_its_field() {
        let config = MockConfig {
            routes: vec![
                MockRoute {
                    method: "GET POST".into(),
                    path: "/orders bad path".into(),
                    status: 700,
                    headers: vec![
                        ParamItem::new("ok", "fine"),
                        ParamItem::new("Content-Length", "1"),
                    ],
                    ..MockRoute::default()
                },
                MockRoute::default(),
                MockRoute::default(),
            ],
            ..Default::default()
        };
        let found = config.problems();
        let problems: Vec<(usize, RouteField, &str)> = found
            .iter()
            .map(|p| (p.route, p.field, p.reason.as_str()))
            .collect();
        assert_eq!(
            problems.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(),
            [
                (0, RouteField::Method),
                (0, RouteField::Path),
                (0, RouteField::Status),
                (0, RouteField::Header(1)),
                (2, RouteField::Route),
            ]
        );
        assert_eq!(problems[1].2, "Paths can't contain spaces; write %20");
        assert_eq!(problems[4].2, "Another enabled route already answers GET /");
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "Route 1: Methods are one word without spaces, such as GET",
            "saving and the server refuse with the first"
        );
        for (path, reason) in [
            ("x", "Paths start with /"),
            ("/a?b", "Leave out ? and #: routes match the path only"),
            ("/é", "Encode other characters, for example é as %C3%A9"),
        ] {
            assert_eq!(path_problem(path), Some(reason), "{path}");
        }
        assert_eq!(path_problem("/a%20b/c"), None);
    }

    #[test]
    fn invalid_rules_and_duplicate_enabled_keys_are_rejected() {
        let route = MockRoute::default();
        let config = |route| MockConfig {
            routes: vec![route],
            ..Default::default()
        };
        config(route.clone()).validate().unwrap();
        for method in ["", "GET POST", "G\nET"] {
            assert!(
                config(MockRoute {
                    method: method.into(),
                    ..route.clone()
                })
                .validate()
                .is_err()
            );
        }
        for path in ["", "relative", "/a?b", "/a#b", "/a b", "/é", "/\r"] {
            assert!(
                config(MockRoute {
                    path: path.into(),
                    ..route.clone()
                })
                .validate()
                .is_err()
            );
        }
        for status in [0, 101, 199, 600] {
            assert!(
                config(MockRoute {
                    status,
                    ..route.clone()
                })
                .validate()
                .is_err()
            );
        }
        for (name, value) in [
            ("bad name", "x"),
            ("X", "x\r\ny"),
            ("Content-Length", "1"),
            ("Transfer-Encoding", "chunked"),
            ("Connection", "close"),
            ("Trailer", "X"),
            ("Upgrade", "websocket"),
        ] {
            assert!(
                config(MockRoute {
                    headers: vec![ParamItem::new(name, value)],
                    ..route.clone()
                })
                .validate()
                .is_err()
            );
        }
        let mut config = MockConfig {
            routes: vec![route.clone(), route],
            ..Default::default()
        };
        assert!(config.validate().is_err());
        config.routes[1].enabled = false;
        config.validate().unwrap();
        assert!(config.matching_route("GET", "/").is_some());
        assert!(config.matching_route("POST", "/").is_none());
        assert!(config.matching_route("GET", "/other").is_none());
    }
}
