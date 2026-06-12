//! Plugin SDK version contract shared by hosts and WASM plugins.

/// Current GeneGIS plugin API version supported by this crate.
pub const PLUGIN_API_VERSION: &str = "0.1.0";

/// Well-known manifest filename searched beside a plugin bundle.
pub const MANIFEST_FILENAME: &str = "genegis.plugin.json";

/// Returns true when `manifest_api_version` is compatible with [`PLUGIN_API_VERSION`].
///
/// Phase 4 alpha accepts an exact match on the major.minor contract (`0.1`).
pub fn is_api_compatible(manifest_api_version: &str) -> bool {
    compatible_major_minor(manifest_api_version, PLUGIN_API_VERSION)
}

fn compatible_major_minor(manifest: &str, host: &str) -> bool {
    fn major_minor(version: &str) -> Option<(&str, &str)> {
        let (major, rest) = version.split_once('.')?;
        let (minor, _) = rest.split_once('.').unwrap_or((rest, ""));
        Some((major, minor))
    }

    match (major_minor(manifest), major_minor(host)) {
        (Some(left), Some(right)) => left == right,
        _ => manifest == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_api_version() {
        assert!(is_api_compatible("0.1.0"));
        assert!(is_api_compatible("0.1.5"));
    }

    #[test]
    fn rejects_different_major_minor() {
        assert!(!is_api_compatible("0.2.0"));
        assert!(!is_api_compatible("1.0.0"));
    }
}
