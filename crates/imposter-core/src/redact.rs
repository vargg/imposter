//! Taking known secrets out of what a headless run reports. Filtering by known values cannot
//! find every sensitive value a server returns; it removes the ones the run was given and
//! the credentials it sent.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use regex::Regex;

use crate::capture::Taken;
use crate::check::Verdict;
use crate::execution::{ExecutionRecord, Response};
use crate::model::{Auth, RequestTemplate};
use crate::settings::AppSettings;

/// What stands in for a removed value.
pub const MARK: &str = "«redacted»";

/// Headers whose whole value is a credential, by lowercase name.
pub const SECRET_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// Secrets shorter than this would remove ordinary text wherever it appears ("1", "dev"), so
/// a run refuses them as secret inputs.
pub const MIN_SECRET: usize = 4;

/// A stored credential as `show` may print it: as written when it is nothing but
/// `{{placeholders}}`, optionally after an auth scheme word (`Bearer {{token}}`); otherwise
/// [`MARK`], since a literal value is the secret itself. Empty stays empty.
pub fn shown(text: &str) -> String {
    let mut rest = text.to_owned();
    for placeholder in crate::variables::placeholders(text).into_iter().rev() {
        rest.replace_range(placeholder.range, " ");
    }
    let rest = rest.trim();
    let scheme = ["bearer", "basic", "digest", "token"]
        .iter()
        .any(|word| rest.eq_ignore_ascii_case(word));
    if text.is_empty() || ((rest.is_empty() || scheme) && text.contains("{{")) {
        text.to_owned()
    } else {
        MARK.to_owned()
    }
}

/// The values to remove, longest first so a secret containing another goes whole.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    needles: Vec<String>,
}

fn userinfo() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"([A-Za-z][A-Za-z0-9+.\-]*://)[^\s/?#@]+@")
            .expect("the userinfo pattern is a valid regular expression")
    })
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encoding of every byte outside RFC 3986's unreserved set, as URLs and forms
/// carry a value.
fn percent_encoded(value: &str, space_as_plus: bool) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' if space_as_plus => out.push('+'),
            other => {
                out.push('%');
                out.push(char::from(HEX[usize::from(other >> 4)]));
                out.push(char::from(HEX[usize::from(other & 0xF)]));
            }
        }
    }
    out
}

impl Redactor {
    /// Remove `value` and the forms it travels in: percent-encoded (URLs, forms) and base64
    /// (both alphabets, with and without padding). Empty values are ignored.
    pub fn secret(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        let bytes = value.as_bytes();
        let forms = [
            value.to_owned(),
            percent_encoded(value, false),
            percent_encoded(value, true),
            STANDARD.encode(bytes),
            STANDARD_NO_PAD.encode(bytes),
            URL_SAFE.encode(bytes),
            URL_SAFE_NO_PAD.encode(bytes),
        ];
        for form in forms {
            if !self.needles.contains(&form) {
                self.needles.push(form);
            }
        }
        self.needles
            .sort_by_key(|needle| std::cmp::Reverse(needle.len()));
    }

    /// Remove the credentials `template` sends and the client key password in `settings`.
    /// Basic credentials also go as the `user:password` pair a server could echo.
    pub fn credentials(&mut self, template: &RequestTemplate, settings: &AppSettings) {
        match template.auth.active() {
            Ok(Some(Auth::Basic(credentials))) => {
                self.secret(&credentials.password);
                if !credentials.password.is_empty() {
                    self.secret(&format!(
                        "{}:{}",
                        credentials.username, credentials.password
                    ));
                }
            }
            Ok(Some(Auth::Digest(credentials))) => self.secret(&credentials.password),
            Ok(Some(Auth::Bearer(token))) => self.secret(&token.token),
            Ok(None) | Err(_) => {}
        }
        if let Some(password) = &template.options.password {
            self.secret(password);
        }
        self.secret(&settings.password);
    }

    /// Whether nothing was given to remove.
    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// `text` with every known secret replaced by [`MARK`], and nothing else changed: what
    /// History keeps, where a `user:password@` the user typed is theirs to see.
    pub fn known(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for needle in &self.needles {
            if out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), MARK);
            }
        }
        out
    }

    /// `bytes` through [`Redactor::known`] when they are UTF-8; other bytes stay as they are.
    pub fn bytes(&self, bytes: &mut Vec<u8>) {
        if let Ok(text) = std::str::from_utf8(bytes) {
            let out = self.known(text);
            if out != text {
                *bytes = out.into_bytes();
            }
        }
    }

    /// Every field of `template` a variable can fill, as resolution walks them.
    pub fn template(&self, template: &mut RequestTemplate) {
        // A body that is not text is left as it is; the fields before it are done.
        let _ = crate::variables::visit_texts(template, &mut |text| *text = self.known(text));
    }

    /// Everything of `response` a server or a check could echo a value into: headers,
    /// body, reason, checks, captures and script results. A secret capture's value goes
    /// whole, however short.
    pub fn response(&self, response: &mut Response) {
        for (_, value) in &mut response.headers {
            *value = self.known(value);
        }
        self.bytes(&mut response.body);
        response.reason = self.known(&response.reason);
        for result in &mut response.checks {
            result.check.expected = self.known(&result.check.expected);
            if let Some(actual) = &mut result.actual {
                *actual = self.known(actual);
            }
            if let Verdict::Unchecked(reason) = &mut result.verdict {
                *reason = self.known(reason);
            }
        }
        for captured in &mut response.captures {
            if let Taken::Value(value) = &mut captured.taken {
                *value = if captured.secret {
                    MARK.to_owned()
                } else {
                    self.known(value)
                };
            }
        }
        if let Some(script) = &mut response.script {
            for test in &mut script.tests {
                test.message = self.known(&test.message);
            }
            script.error = self.known(&script.error);
        }
    }

    /// A History record without the secrets: the request as sent and the response.
    pub fn record(&self, record: &mut ExecutionRecord) {
        self.template(&mut record.resolved.template);
        self.template(&mut record.template.template);
        self.response(&mut record.response);
    }

    /// `text` with every known secret and any `user:password@` in a URL replaced by [`MARK`].
    pub fn text(&self, text: &str) -> String {
        let out = userinfo().replace_all(text, format!("${{1}}{MARK}@").as_str());
        self.known(&out)
    }

    /// A header as reported: credential headers lose their whole value.
    pub fn header(&self, name: &str, value: &str) -> (String, String) {
        let value = if SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            MARK.to_owned()
        } else {
            self.text(value)
        };
        (self.text(name), value)
    }

    /// Every string in `value`, keys included, through [`Redactor::text`].
    pub fn json(&self, value: &mut serde_json::Value) {
        use serde_json::Value;
        match value {
            Value::String(text) => *text = self.text(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.json(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.json(&mut item);
                    map.insert(self.text(&key), item);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RequestAuth;

    fn redactor(secrets: &[&str]) -> Redactor {
        let mut redactor = Redactor::default();
        for secret in secrets {
            redactor.secret(secret);
        }
        redactor
    }

    #[test]
    fn secrets_go_in_every_form_they_travel_in() {
        let redactor = redactor(&["s3cr et/+"]);
        for form in [
            "s3cr et/+",
            "s3cr%20et%2F%2B",
            "s3cr+et%2F%2B",
            &STANDARD.encode("s3cr et/+"),
            &URL_SAFE_NO_PAD.encode("s3cr et/+"),
        ] {
            assert_eq!(
                redactor.text(&format!("a {form} b")),
                "a «redacted» b",
                "{form}"
            );
        }
        assert_eq!(redactor.text("nothing here"), "nothing here");
    }

    #[test]
    fn the_longest_secret_goes_whole() {
        let redactor = redactor(&["abcd", "abcdefgh"]);
        assert_eq!(
            redactor.text("x abcdefgh y abcd"),
            "x «redacted» y «redacted»"
        );
    }

    #[test]
    fn url_passwords_and_credential_headers_go() {
        let redactor = Redactor::default();
        assert_eq!(
            redactor.text("see https://ann:pw@example.com/a?b=c@d and ftp://x@y"),
            "see https://«redacted»@example.com/a?b=c@d and ftp://«redacted»@y"
        );
        assert_eq!(
            redactor.header("Set-Cookie", "sid=1"),
            ("Set-Cookie".into(), MARK.into())
        );
        assert_eq!(
            redactor.header("x-trace", "1"),
            ("x-trace".into(), "1".into())
        );
    }

    #[test]
    fn credentials_of_the_active_auth_and_key_password_go() {
        let mut redactor = Redactor::default();
        let template = RequestTemplate {
            auth: RequestAuth::basic("ann", "hunter22"),
            ..RequestTemplate::default()
        };
        let settings = AppSettings {
            password: "keypass".into(),
            ..AppSettings::default()
        };
        redactor.credentials(&template, &settings);
        let echoed = format!("Basic {} hunter22 keypass", STANDARD.encode("ann:hunter22"));
        assert_eq!(
            redactor.text(&echoed),
            "Basic «redacted» «redacted» «redacted»"
        );

        let mut bearer = Redactor::default();
        bearer.credentials(
            &RequestTemplate {
                auth: RequestAuth::bearer("tok-123"),
                ..RequestTemplate::default()
            },
            &AppSettings::default(),
        );
        assert_eq!(bearer.text("tok-123"), MARK);
    }

    #[test]
    fn only_placeholders_are_shown_for_a_stored_credential() {
        assert_eq!(shown("{{token}}"), "{{token}}");
        assert_eq!(shown("Bearer {{token}}"), "Bearer {{token}}");
        assert_eq!(
            shown("{{user}}:{{pass}}"),
            MARK,
            "a literal colon is not a scheme"
        );
        assert_eq!(shown(" {{a}} {{b}} "), " {{a}} {{b}} ");
        assert_eq!(shown("hunter22"), MARK);
        assert_eq!(shown("Bearer abc123"), MARK);
        assert_eq!(shown("Bearer"), MARK, "a scheme alone is not a placeholder");
        assert_eq!(shown("{{token}}x"), MARK);
        assert_eq!(shown(""), "");
    }

    #[test]
    fn json_is_redacted_everywhere_keys_included() {
        let redactor = redactor(&["topsecret"]);
        let mut value = serde_json::json!({
            "a": ["topsecret", 1, null, {"topsecret": "x topsecret"}],
        });
        redactor.json(&mut value);
        assert_eq!(
            value,
            serde_json::json!({"a": ["«redacted»", 1, null, {"«redacted»": "x «redacted»"}]})
        );
    }

    #[test]
    fn a_record_loses_known_secrets_and_keeps_everything_else() {
        use crate::capture::Captured;
        use crate::check::{Check, CheckResult, Op};
        use crate::execution::ExecutionRecord;
        use crate::model::{BodyType, ParamItem, Request};
        use crate::select::Selector;

        let mut resolved = RequestTemplate {
            url: "https://user:pass@x/items?key=sk-12345".into(),
            headers: vec![ParamItem::new("x-api-key", "sk-12345")],
            body_type: BodyType::Json,
            body: br#"{"key": "sk-12345"}"#.to_vec(),
            ..RequestTemplate::default()
        };
        resolved.auth.bearer_token = Some(crate::model::BearerToken {
            token: "sk-12345".into(),
        });
        let mut record = ExecutionRecord {
            timestamp: crate::execution::Timestamp(0),
            request_id: None,
            template: Request::default(),
            settings: AppSettings::default(),
            environment_id: None,
            environment: String::new(),
            via: crate::execution::Via::default(),
            resolved: Request {
                template: resolved,
                ..Request::default()
            },
            response: Response {
                headers: vec![("x-echo".into(), "sk-12345".into())],
                body: br#"{"token": "tok-999", "echo": "sk-12345"}"#.to_vec(),
                checks: vec![CheckResult {
                    check: Check::new(Selector::Status, Op::Equals, "sk-12345"),
                    verdict: Verdict::Failed,
                    actual: Some("sk-12345".into()),
                }],
                captures: vec![
                    Captured {
                        variable: "token".into(),
                        taken: Taken::Value("t1".into()),
                        target: "Dev secrets".into(),
                        secret: true,
                    },
                    Captured {
                        variable: "id".into(),
                        taken: Taken::Value("7".into()),
                        target: "Dev".into(),
                        secret: false,
                    },
                ],
                ..Response::default()
            },
        };
        redactor(&["sk-12345"]).record(&mut record);
        let template = &record.resolved.template;
        assert_eq!(
            template.url, "https://user:pass@x/items?key=«redacted»",
            "userinfo is the user's"
        );
        assert_eq!(template.headers[0].value, MARK);
        assert_eq!(template.body, r#"{"key": "«redacted»"}"#.as_bytes());
        assert_eq!(template.auth.bearer_token.as_ref().unwrap().token, MARK);
        let response = &record.response;
        assert_eq!(response.headers[0].1, MARK);
        assert_eq!(
            response.body,
            r#"{"token": "tok-999", "echo": "«redacted»"}"#.as_bytes()
        );
        assert_eq!(response.checks[0].check.expected, MARK);
        assert_eq!(response.checks[0].actual.as_deref(), Some(MARK));
        assert_eq!(
            response.captures[0].taken,
            Taken::Value(MARK.into()),
            "short, but a secret"
        );
        assert_eq!(response.captures[1].taken, Taken::Value("7".into()));
    }
}
