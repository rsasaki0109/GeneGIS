use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::StorageError;

/// Environment variable containing comma-separated remote host allowlist entries.
pub const REMOTE_ALLOWED_HOSTS_ENV: &str = "GENEGIS_REMOTE_ALLOWED_HOSTS";

/// Fail-closed network policy for untrusted catalog and cloud asset URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessPolicy {
    /// Exact hosts or `*.example.org` wildcard suffixes.
    pub allowed_hosts: Vec<String>,
    /// Permit localhost and loopback IPs for deterministic fixtures.
    pub allow_loopback: bool,
    /// Maximum bytes accepted for one response body.
    pub max_response_bytes: u64,
    /// End-to-end timeout for one HTTP call.
    pub timeout_ms: u64,
    /// Maximum redirects. Secure defaults use zero.
    pub max_redirects: u32,
}

impl Default for RemoteAccessPolicy {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RemoteAccessPolicy {
    /// Build the secure default, reading only hostnames from the allowlist environment variable.
    pub fn from_env() -> Self {
        let allowed_hosts = std::env::var(REMOTE_ALLOWED_HOSTS_ENV)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(|host| host.to_ascii_lowercase())
            .collect();
        Self {
            allowed_hosts,
            allow_loopback: true,
            max_response_bytes: 8 * 1024 * 1024,
            timeout_ms: 15_000,
            max_redirects: 0,
        }
    }

    /// Validate an HTTP(S) URL before opening a connection.
    pub fn validate_url(&self, value: &str) -> Result<(), StorageError> {
        let url = url::Url::parse(value)
            .map_err(|error| StorageError::UnsupportedScheme(format!("{value:?}: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(StorageError::UnsupportedScheme(url.scheme().into()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(StorageError::Http(
                "remote URL credentials are not permitted".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| StorageError::Http("remote URL has no host".into()))?
            .to_ascii_lowercase();
        if self.allow_loopback && is_loopback_host(&host) {
            return Ok(());
        }
        if self
            .allowed_hosts
            .iter()
            .any(|allowed| host_matches(&host, allowed))
        {
            return Ok(());
        }
        Err(StorageError::Http(format!(
            "remote host {host:?} is not allowlisted; set {REMOTE_ALLOWED_HOSTS_ENV}"
        )))
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn host_matches(host: &str, allowed: &str) -> bool {
    let allowed = allowed.trim().to_ascii_lowercase();
    if let Some(suffix) = allowed.strip_prefix("*.") {
        host != suffix && host.ends_with(&format!(".{suffix}"))
    } else {
        host == allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unlisted_remote_host() {
        let policy = RemoteAccessPolicy {
            allowed_hosts: vec![],
            allow_loopback: false,
            ..RemoteAccessPolicy::from_env()
        };
        assert!(policy.validate_url("https://example.com/data").is_err());
    }

    #[test]
    fn accepts_exact_wildcard_and_loopback_hosts() {
        let policy = RemoteAccessPolicy {
            allowed_hosts: vec!["data.example.org".into(), "*.cloud.example.org".into()],
            ..RemoteAccessPolicy::from_env()
        };
        assert!(policy.validate_url("https://data.example.org/a").is_ok());
        assert!(policy
            .validate_url("https://tiles.cloud.example.org/a")
            .is_ok());
        assert!(policy.validate_url("http://127.0.0.1:7812/a").is_ok());
        assert!(policy.validate_url("https://cloud.example.org/a").is_err());
    }

    #[test]
    fn rejects_embedded_credentials() {
        assert!(RemoteAccessPolicy::from_env()
            .validate_url("https://user:secret@example.org/a")
            .is_err());
    }
}
