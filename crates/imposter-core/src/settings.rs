//! Application defaults and their resolution against per-request options.

use serde::{Deserialize, Serialize};

use crate::model::Options;

/// Client credentials for one HTTPS host and port. An empty key uses the certificate file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostIdentity {
    pub host: String,
    pub port: u16,
    pub certificate_path: String,
    pub key_file: String,
    pub password: String,
}

impl Default for HostIdentity {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 443,
            certificate_path: String::new(),
            key_file: String::new(),
            password: String::new(),
        }
    }
}

/// The same normalized spelling for DNS names, URL hosts, and IPv4/IPv6 literals.
pub fn normalized_host(host: &str) -> Option<String> {
    let host = host.trim_end_matches('.');
    let literal = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        return Some(ip.to_string());
    }
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || "/:@?#*[]\\".contains(c))
    {
        return None;
    }
    ::url::Host::parse(host).ok().map(|h| h.to_string())
}

/// Persisted defaults. Field names and units match the Python app (`download_limit` in KiB).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Seconds.
    pub timeout: f64,
    pub follow_redirects: bool,
    pub verify_ssl: bool,
    pub attach_cookies: bool,
    pub proxy_url: String,
    pub ca_bundle: String,
    pub certificate_path: String,
    pub key_file: String,
    pub password: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub host_identities: Vec<HostIdentity>,
    /// KiB of response body to capture; `0` captures everything.
    pub download_limit: u64,
    pub autosave: bool,
    pub history_max_entries: u64,
    pub history_max_bytes: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            timeout: 30.0,
            follow_redirects: true,
            verify_ssl: true,
            attach_cookies: true,
            proxy_url: String::new(),
            ca_bundle: String::new(),
            certificate_path: String::new(),
            key_file: String::new(),
            password: String::new(),
            host_identities: Vec::new(),
            download_limit: 5120,
            autosave: true,
            history_max_entries: 1000,
            history_max_bytes: 250 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("Timeout must be a positive number of seconds")]
    Timeout,
    #[error("History must keep at least one entry")]
    HistoryEntries,
    #[error("History storage must be at least 1 KiB")]
    HistoryBytes,
    #[error("Client certificate entries need a valid host, a nonzero port, and a certificate path")]
    HostIdentity,
    #[error("Only one client certificate entry is allowed per host and port")]
    DuplicateHostIdentity,
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(self.timeout > 0.0 && self.timeout.is_finite()) {
            return Err(SettingsError::Timeout);
        }
        if self.history_max_entries < 1 {
            return Err(SettingsError::HistoryEntries);
        }
        if self.history_max_bytes < 1024 {
            return Err(SettingsError::HistoryBytes);
        }
        let mut targets = std::collections::BTreeSet::new();
        for identity in &self.host_identities {
            let host = normalized_host(&identity.host).ok_or(SettingsError::HostIdentity)?;
            if identity.port == 0 || identity.certificate_path.trim().is_empty() {
                return Err(SettingsError::HostIdentity);
            }
            if !targets.insert((host, identity.port)) {
                return Err(SettingsError::DuplicateHostIdentity);
            }
        }
        Ok(())
    }

    pub fn identity_for_url(&self, url: &str) -> Option<&HostIdentity> {
        let url = ::url::Url::parse(url).ok()?;
        if url.scheme() != "https" {
            return None;
        }
        let host = normalized_host(url.host_str()?)?;
        let port = url.port_or_known_default()?;
        self.host_identities.iter().find(|identity| {
            identity.port == port && normalized_host(&identity.host).as_ref() == Some(&host)
        })
    }

    /// A host identity replaces the global credential set; individual request fields then win.
    #[must_use]
    pub fn resolve_for_url(&self, url: &str, options: &Options) -> Self {
        let mut defaults = self.clone();
        if let Some(identity) = self.identity_for_url(url) {
            defaults
                .certificate_path
                .clone_from(&identity.certificate_path);
            defaults.key_file.clone_from(&identity.key_file);
            defaults.password.clone_from(&identity.password);
        }
        defaults.resolve(options)
    }

    /// Apply request overrides. `None` inherits; an explicit value, even `false` or `""`, wins.
    #[must_use]
    pub fn resolve(&self, options: &Options) -> Self {
        let mut out = self.clone();
        let text = |target: &mut String, value: &Option<String>| {
            if let Some(value) = value {
                target.clone_from(value);
            }
        };
        if let Some(value) = options.follow_redirects {
            out.follow_redirects = value;
        }
        if let Some(value) = options.verify_ssl {
            out.verify_ssl = value;
        }
        if let Some(value) = options.attach_cookies {
            out.attach_cookies = value;
        }
        if let Some(value) = options.timeout {
            out.timeout = value;
        }
        if let Some(value) = options.download_limit {
            out.download_limit = value;
        }
        text(&mut out.proxy_url, &options.proxy_url);
        text(&mut out.ca_bundle, &options.ca_bundle);
        text(&mut out.certificate_path, &options.certificate_path);
        text(&mut out.key_file, &options.key_file);
        text(&mut out.password, &options.password);
        out
    }

    /// Every transport setting as an explicit override, so inherited paths can be resolved
    /// against the same variable snapshot as the request itself.
    pub fn as_options(&self) -> Options {
        Options {
            follow_redirects: Some(self.follow_redirects),
            verify_ssl: Some(self.verify_ssl),
            attach_cookies: Some(self.attach_cookies),
            proxy_url: Some(self.proxy_url.clone()),
            timeout: Some(self.timeout),
            ca_bundle: Some(self.ca_bundle.clone()),
            certificate_path: Some(self.certificate_path.clone()),
            key_file: Some(self.key_file.clone()),
            password: Some(self.password.clone()),
            download_limit: Some(self.download_limit),
        }
    }

    /// The capture limit in bytes, or `None` for no limit.
    pub fn download_limit_bytes(&self) -> Option<u64> {
        (self.download_limit > 0).then(|| self.download_limit.saturating_mul(1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_identities_normalize_and_override_as_a_set_then_by_field() {
        let identity = HostIdentity {
            host: "EXAMPLE.com.".into(),
            certificate_path: "host.pem".into(),
            ..Default::default()
        };
        let mut settings = AppSettings {
            certificate_path: "global.pem".into(),
            key_file: "global.key".into(),
            password: "global password".into(),
            host_identities: vec![identity.clone()],
            ..Default::default()
        };
        settings.validate().unwrap();
        let resolved = settings.resolve_for_url(
            "https://example.com/path",
            &Options {
                password: Some("override".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            (
                &*resolved.certificate_path,
                &*resolved.key_file,
                &*resolved.password
            ),
            ("host.pem", "", "override")
        );
        assert_eq!(
            settings
                .resolve_for_url("https://example.com:8443", &Options::default())
                .certificate_path,
            "global.pem"
        );
        assert_eq!(
            settings
                .resolve_for_url("http://example.com:443", &Options::default())
                .certificate_path,
            "global.pem"
        );
        assert_eq!(
            settings
                .resolve_for_url("https://other.com", &Options::default())
                .certificate_path,
            "global.pem"
        );
        assert_eq!(
            settings
                .resolve_for_url(
                    "https://example.com",
                    &Options {
                        certificate_path: Some(String::new()),
                        ..Default::default()
                    }
                )
                .certificate_path,
            ""
        );
        settings.host_identities.push(HostIdentity {
            host: "example.com".into(),
            ..identity
        });
        assert_eq!(
            settings.validate(),
            Err(SettingsError::DuplicateHostIdentity)
        );
        settings.host_identities[1].port = 0;
        assert_eq!(settings.validate(), Err(SettingsError::HostIdentity));
        for host in [
            "",
            "*.example.com",
            "https://example.com",
            "host:443",
            "host name",
            "user@host",
        ] {
            assert_eq!(normalized_host(host), None, "{host}");
        }
        assert_eq!(normalized_host("[0:0:0:0:0:0:0:1]"), Some("::1".into()));
        assert_eq!(normalized_host("127.0.0.1"), Some("127.0.0.1".into()));
        let old: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.host_identities, []);
    }

    #[test]
    fn defaults_match_the_spec() {
        let settings = AppSettings::default();
        assert!((settings.timeout - 30.0).abs() < f64::EPSILON);
        assert!(settings.follow_redirects && settings.verify_ssl && settings.attach_cookies);
        assert_eq!(settings.proxy_url, "");
        assert_eq!(settings.download_limit_bytes(), Some(5 * 1024 * 1024));
        assert_eq!(
            (settings.history_max_entries, settings.history_max_bytes),
            (1000, 250 * 1024 * 1024)
        );
        settings.validate().unwrap();
    }

    #[test]
    fn explicit_empty_and_false_override_defaults() {
        let base = AppSettings {
            proxy_url: "http://p".into(),
            ..AppSettings::default()
        };
        let resolved = base.resolve(&Options {
            proxy_url: Some(String::new()),
            verify_ssl: Some(false),
            download_limit: Some(0),
            ..Options::default()
        });
        assert_eq!(resolved.proxy_url, "");
        assert!(!resolved.verify_ssl);
        assert_eq!(resolved.download_limit_bytes(), None);
        assert_eq!(base.resolve(&Options::default()), base);
        assert_eq!(AppSettings::default().resolve(&base.as_options()), base);
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let check = |settings: AppSettings| settings.validate();
        assert_eq!(
            check(AppSettings {
                timeout: 0.0,
                ..AppSettings::default()
            }),
            Err(SettingsError::Timeout)
        );
        assert_eq!(
            check(AppSettings {
                timeout: f64::NAN,
                ..AppSettings::default()
            }),
            Err(SettingsError::Timeout)
        );
        assert_eq!(
            check(AppSettings {
                history_max_entries: 0,
                ..AppSettings::default()
            }),
            Err(SettingsError::HistoryEntries)
        );
        assert_eq!(
            check(AppSettings {
                history_max_bytes: 1023,
                ..AppSettings::default()
            }),
            Err(SettingsError::HistoryBytes)
        );
    }
}
