//! Plugin SDK version contract shared by hosts and WASM plugins.

/// Current GeneGIS plugin API version supported by this crate.
pub const PLUGIN_API_VERSION: &str = "1.0.0";

/// Well-known manifest filename searched beside a plugin bundle.
pub const MANIFEST_FILENAME: &str = "genegis.plugin.json";

/// Returns true when `manifest_api_version` is compatible with [`PLUGIN_API_VERSION`].
///
/// SDK v1 accepts releases with the same non-zero major version.
pub fn is_api_compatible(manifest_api_version: &str) -> bool {
    compatible_major(manifest_api_version, PLUGIN_API_VERSION)
}

fn compatible_major(manifest: &str, host: &str) -> bool {
    fn major(version: &str) -> Option<u64> {
        let value = version.split('.').next()?;
        value.parse().ok()
    }

    matches!((major(manifest), major(host)), (Some(left), Some(right)) if left > 0 && left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_api_version() {
        assert!(is_api_compatible("1.0.0"));
        assert!(is_api_compatible("1.7.5"));
    }

    #[test]
    fn rejects_different_major_minor() {
        assert!(!is_api_compatible("0.2.0"));
        assert!(!is_api_compatible("2.0.0"));
    }
}
