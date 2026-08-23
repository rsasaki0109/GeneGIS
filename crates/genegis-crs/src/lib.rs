//! Coordinate reference system and spatial metadata contracts for GeneGIS.

#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, ops::Deref, str::FromStr};

/// EPSG identifier for WGS 84 longitude/latitude coordinates.
pub const WGS84_EPSG: u32 = 4326;

/// EPSG identifier for JGD2011 geographic 2D coordinates.
pub const JGD2011_EPSG: u32 = 6668;

/// EPSG identifier for JGD2011 / Japan Plane Rectangular CS VII.
///
/// Zone VII is the local projected CRS covering Nagoya and Aichi. It is
/// retained as a named definition even though the MVP area calculation uses
/// ellipsoidal WGS 84 area directly from the source coordinates.
pub const NAGOYA_PROJECTED_EPSG: u32 = 6675;

/// EPSG identifier for Web Mercator Auxiliary Sphere.
pub const WEB_MERCATOR_EPSG: u32 = 3857;

/// EPSG identifier for WGS 84 / UTM zone 17N.
pub const UTM_17N_EPSG: u32 = 32617;

/// EPSG identifier for WGS 84 / UTM zone 54N.
pub const UTM_54N_EPSG: u32 = 32654;

/// Coordinate system family described by an EPSG definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsKind {
    /// Longitude/latitude or another angular coordinate system.
    Geographic,
    /// A coordinate system whose axes are linear distances.
    Projected,
    /// An EPSG code known syntactically but not in the built-in registry.
    Unknown,
}

/// Unit used by coordinate axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateUnit {
    /// Decimal degrees, used by EPSG:4326 and EPSG:6668.
    Degrees,
    /// Metres, used by projected Japanese plane coordinate systems.
    Metres,
    /// Unit not known because the EPSG definition is not in the registry.
    Unknown,
}

impl CoordinateUnit {
    /// Return the stable contract spelling used in workflow and receipt JSON.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Degrees => "degrees",
            Self::Metres => "metres",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for CoordinateUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Short compatibility alias for callers that refer to coordinate units as
/// simply `Unit`.
pub type Unit = CoordinateUnit;

/// A normalized authority/code pair for a coordinate reference system.
///
/// The type deliberately stores the authority and numeric code rather than a
/// free-form WKT/PROJ string. This gives all workflow inputs a stable,
/// serializable identity while allowing unknown future EPSG codes to be
/// represented and rejected by [`Crs::require_known`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Crs {
    authority: String,
    code: u32,
}

impl Crs {
    /// Construct a normalized authority/code pair.
    ///
    /// The built-in registry currently resolves EPSG definitions; other
    /// authorities remain representable and fail closed at `require_known`.
    pub fn new(authority: impl Into<String>, code: u32) -> Self {
        Self {
            authority: authority.into().trim().to_ascii_uppercase(),
            code,
        }
    }

    /// Construct an EPSG CRS from its numeric code.
    pub fn epsg(code: u32) -> Self {
        Self::new("EPSG", code)
    }

    /// Return the WGS 84 geographic CRS used by GeoJSON.
    pub fn wgs84() -> Self {
        Self::epsg(WGS84_EPSG)
    }

    /// Return the JGD2011 geographic 2D CRS.
    pub fn jgd2011() -> Self {
        Self::epsg(JGD2011_EPSG)
    }

    /// Return the local projected CRS for Nagoya/Aichi.
    pub fn nagoya_projected() -> Self {
        Self::epsg(NAGOYA_PROJECTED_EPSG)
    }

    /// Parse an EPSG identifier such as `EPSG:4326` or `epsg::4326`.
    ///
    /// OGC URN spelling (`urn:ogc:def:crs:EPSG::4326`) is accepted as well.
    /// Unknown numeric codes are retained so callers can produce a useful
    /// validation error with [`Crs::require_known`].
    pub fn parse(value: &str) -> Result<Self, CrsError> {
        let normalized = value.trim();
        let code = normalized
            .strip_prefix("urn:ogc:def:crs:")
            .unwrap_or(normalized);
        let code = code
            .strip_prefix("EPSG::")
            .or_else(|| code.strip_prefix("epsg::"))
            .or_else(|| code.strip_prefix("EPSG:"))
            .or_else(|| code.strip_prefix("epsg:"))
            .ok_or_else(|| CrsError::InvalidIdentifier(value.to_string()))?;
        let code = code
            .parse::<u32>()
            .map_err(|_| CrsError::InvalidIdentifier(value.to_string()))?;
        Ok(Self::epsg(code))
    }

    /// Return the authority name (`EPSG`).
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// Return the numeric authority code.
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// Return the canonical `AUTHORITY:CODE` spelling.
    pub fn identifier(&self) -> String {
        format!("{}:{}", self.authority, self.code)
    }

    /// Return the built-in definition, if this CRS is known.
    pub fn definition(&self) -> Option<CrsDefinition> {
        if self.authority != "EPSG" {
            return None;
        }
        match self.code {
            WGS84_EPSG => Some(CrsDefinition {
                code: WGS84_EPSG,
                name: "WGS 84",
                kind: CrsKind::Geographic,
                unit: CoordinateUnit::Degrees,
            }),
            JGD2011_EPSG => Some(CrsDefinition {
                code: JGD2011_EPSG,
                name: "JGD2011",
                kind: CrsKind::Geographic,
                unit: CoordinateUnit::Degrees,
            }),
            NAGOYA_PROJECTED_EPSG => Some(CrsDefinition {
                code: NAGOYA_PROJECTED_EPSG,
                name: "JGD2011 / Japan Plane Rectangular CS VII",
                kind: CrsKind::Projected,
                unit: CoordinateUnit::Metres,
            }),
            WEB_MERCATOR_EPSG => Some(CrsDefinition {
                code: WEB_MERCATOR_EPSG,
                name: "WGS 84 / Pseudo-Mercator",
                kind: CrsKind::Projected,
                unit: CoordinateUnit::Metres,
            }),
            UTM_17N_EPSG => Some(CrsDefinition {
                code: UTM_17N_EPSG,
                name: "WGS 84 / UTM zone 17N",
                kind: CrsKind::Projected,
                unit: CoordinateUnit::Metres,
            }),
            UTM_54N_EPSG => Some(CrsDefinition {
                code: UTM_54N_EPSG,
                name: "WGS 84 / UTM zone 54N",
                kind: CrsKind::Projected,
                unit: CoordinateUnit::Metres,
            }),
            _ => None,
        }
    }

    /// Return the CRS family, or [`CrsKind::Unknown`] for an unregistered code.
    pub fn kind(&self) -> CrsKind {
        self.definition()
            .map(|definition| definition.kind)
            .unwrap_or(CrsKind::Unknown)
    }

    /// Return the coordinate axis unit, or [`CoordinateUnit::Unknown`].
    pub fn coordinate_unit(&self) -> CoordinateUnit {
        self.definition()
            .map(|definition| definition.unit)
            .unwrap_or(CoordinateUnit::Unknown)
    }

    /// Return the coordinate axis unit (short alias for [`Crs::coordinate_unit`]).
    pub fn unit(&self) -> CoordinateUnit {
        self.coordinate_unit()
    }

    /// Return whether this CRS uses angular longitude/latitude axes.
    pub fn is_geographic(&self) -> bool {
        self.kind() == CrsKind::Geographic
    }

    /// Return whether this CRS uses projected linear axes.
    pub fn is_projected(&self) -> bool {
        self.kind() == CrsKind::Projected
    }

    /// Require that this identifier exists in GeneGIS's built-in registry.
    pub fn require_known(&self) -> Result<CrsDefinition, CrsError> {
        self.definition()
            .ok_or_else(|| CrsError::Unsupported(self.identifier()))
    }

    /// Validate a coordinate against this CRS's axis domain.
    pub fn validate_coordinate(&self, x: f64, y: f64) -> Result<(), CrsError> {
        let definition = self.require_known()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(CrsError::NonFiniteCoordinate { x, y });
        }
        if definition.code == WGS84_EPSG || definition.code == JGD2011_EPSG {
            if !(-180.0..=180.0).contains(&x) || !(-90.0..=90.0).contains(&y) {
                return Err(CrsError::CoordinateOutOfRange { x, y });
            }
        }
        Ok(())
    }
}

impl FromStr for Crs {
    type Err = CrsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl From<Crs> for String {
    fn from(crs: Crs) -> Self {
        crs.identifier()
    }
}

impl TryFrom<String> for Crs {
    type Error = CrsError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl fmt::Display for Crs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.identifier())
    }
}

/// Descriptive alias used by API consumers that prefer the long CRS name.
pub type CoordinateReferenceSystem = Crs;

/// The small built-in EPSG definition used for contract validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrsDefinition {
    /// EPSG numeric code.
    pub code: u32,
    /// Human-readable EPSG name.
    pub name: &'static str,
    /// Coordinate system family.
    pub kind: CrsKind,
    /// Coordinate axis unit.
    pub unit: CoordinateUnit,
}

/// Stable version or revision identifier supplied by a source adapter.
///
/// A source version is intentionally separate from a retrieval timestamp. A
/// version such as a census release or a provider revision is stable across
/// replays, while the time at which an adapter fetched it is an execution
/// event. The transparent representation keeps existing JSON consumers
/// reading a string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceVersion(String);

impl SourceVersion {
    /// Construct a normalized source version value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().trim().to_string())
    }

    /// Return the version as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return whether this value is empty and should not be recorded.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SourceVersion {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SourceVersion {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for SourceVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for SourceVersion {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for SourceVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// State of content-digest verification for a source snapshot.
///
/// `Unknown` is intentional: an external URI without a declared checksum is
/// not treated as verified. `Declared` means a provider supplied a checksum,
/// but the current adapter did not download the complete content and could
/// not compare it. Local files and complete downloads use `Verified` after
/// computing SHA-256. `Mismatch` records a failed comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumVerification {
    /// The observed bytes matched the declared checksum, or were hashed from
    /// a local source with no separate expected value.
    Verified,
    /// A checksum was declared but has not been compared with complete bytes.
    Declared,
    /// No checksum was available for this source snapshot.
    Unknown,
    /// Observed bytes did not match the declared checksum.
    Mismatch,
}

impl Default for ChecksumVerification {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ChecksumVerification {
    /// Return the stable contract spelling used in JSON and checks.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Declared => "declared",
            Self::Unknown => "unknown",
            Self::Mismatch => "mismatch",
        }
    }

    /// Return whether the source bytes were actually checked.
    pub const fn is_verified(self) -> bool {
        matches!(self, Self::Verified)
    }
}

impl fmt::Display for ChecksumVerification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A source identity that can be attached to spatial results and receipts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMetadata {
    /// Stable catalog/dataset identifier, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_id: Option<String>,
    /// URI or path used to read the source.
    pub uri: String,
    /// Declared license or usage attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Content digest, preferably a SHA-256 value with an explicit prefix.
    ///
    /// This field is retained for API compatibility. For a readable local
    /// source it is the observed digest; for an external or unreadable source
    /// it falls back to the declared expected digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// Digest declared by a catalog or provider as the expected content.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "declared_checksum"
    )]
    pub expected_checksum: Option<String>,
    /// Digest computed from the bytes actually read, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_checksum: Option<String>,
    /// Retrieval instant in RFC 3339 form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
    /// Stable provider revision, release, or fixture version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_version: Option<SourceVersion>,
    /// Whether the checksum relationship is known, merely declared, verified,
    /// or mismatched.
    #[serde(default, alias = "checksum_verification")]
    pub checksum_status: ChecksumVerification,
}

impl SourceMetadata {
    /// Construct source metadata from a URI or local path.
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            ..Self::default()
        }
    }

    /// Build a source snapshot from a URI and known catalog metadata.
    ///
    /// Local paths are hashed immediately so a fixture receipt contains the
    /// digest of the bytes actually read. For HTTP(S) and other external URIs
    /// the expected checksum is retained as `Declared`; without one the state
    /// is explicitly `Unknown`. No retrieval timestamp is generated here.
    pub fn from_uri(
        uri: impl Into<String>,
        expected_checksum: Option<&str>,
        source_version: Option<&str>,
    ) -> Self {
        let uri = uri.into();
        let mut source = Self::new(uri.clone());
        source.source_version = source_version
            .filter(|version| !version.trim().is_empty())
            .map(SourceVersion::new);

        let expected = expected_checksum
            .map(str::trim)
            .filter(|checksum| !checksum.is_empty())
            .map(ToOwned::to_owned);
        source.expected_checksum = expected.clone();

        if let Some(path) = local_path(&uri) {
            match sha256_file(&path) {
                Ok(actual) => {
                    source.checksum = Some(actual.clone());
                    source.observed_checksum = Some(actual.clone());
                    source.checksum_status = match expected.as_deref() {
                        Some(expected) if normalize_sha256(expected) == Some(actual.clone()) => {
                            ChecksumVerification::Verified
                        }
                        Some(_) => ChecksumVerification::Mismatch,
                        None => ChecksumVerification::Verified,
                    };
                }
                Err(_) => {
                    source.checksum = expected.clone();
                    source.checksum_status = if expected.is_some() {
                        ChecksumVerification::Declared
                    } else {
                        ChecksumVerification::Unknown
                    };
                }
            }
        } else {
            source.checksum = expected.clone();
            source.checksum_status = if expected.is_some() {
                ChecksumVerification::Declared
            } else {
                ChecksumVerification::Unknown
            };
        }

        source
    }

    /// Attach a retrieval event supplied by a source adapter.
    ///
    /// This method is deliberately explicit so replaying a known snapshot
    /// does not call the clock or change a workflow/input digest.
    pub fn with_retrieved_at(mut self, retrieved_at: impl Into<String>) -> Self {
        self.retrieved_at = Some(retrieved_at.into());
        self
    }

    /// Return whether the source checksum was actually verified.
    pub fn checksum_verified(&self) -> bool {
        self.checksum_status.is_verified()
    }

    /// Return the checksum verification state (long-form compatibility name).
    pub fn checksum_verification(&self) -> ChecksumVerification {
        self.checksum_status
    }
}

/// Source snapshot name used by workflow and provenance APIs.
///
/// This alias preserves the original `SourceMetadata` API while making the
/// snapshot role explicit at call sites.
pub type SourceSnapshot = SourceMetadata;

fn local_path(uri: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = uri.strip_prefix("file://") {
        return Some(std::path::PathBuf::from(path));
    }
    if uri.contains("://") {
        return None;
    }
    Some(std::path::PathBuf::from(uri))
}

fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn normalize_sha256(value: &str) -> Option<String> {
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(format!("sha256:{}", hex.to_ascii_lowercase()))
    } else {
        None
    }
}

/// CRS, coordinate units, and source identity carried by a spatial value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpatialMetadata {
    /// Coordinate reference system of the value.
    pub crs: Crs,
    /// Explicit coordinate-axis units derived from the CRS.
    pub coordinate_unit: CoordinateUnit,
    /// Source identity and attribution.
    pub source: SourceMetadata,
}

impl SpatialMetadata {
    /// Construct metadata and derive the coordinate unit from the CRS.
    pub fn new(crs: Crs, source: SourceMetadata) -> Result<Self, CrsError> {
        let definition = crs.require_known()?;
        Ok(Self {
            crs,
            coordinate_unit: definition.unit,
            source,
        })
    }
}

/// Errors raised while parsing or validating CRS and spatial metadata.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CrsError {
    /// Input is not a supported EPSG identifier spelling.
    #[error("invalid CRS identifier: {0}")]
    InvalidIdentifier(String),
    /// The authority/code pair is not in the built-in registry.
    #[error("unsupported CRS: {0}")]
    Unsupported(String),
    /// A coordinate contains NaN or infinity.
    #[error("non-finite coordinate ({x}, {y})")]
    NonFiniteCoordinate {
        /// X or longitude value.
        x: f64,
        /// Y or latitude value.
        y: f64,
    },
    /// A geographic coordinate lies outside the longitude/latitude domain.
    #[error("coordinate out of range ({x}, {y})")]
    CoordinateOutOfRange {
        /// X or longitude value.
        x: f64,
        /// Y or latitude value.
        y: f64,
    },
}

/// Placeholder version marker retained for compatibility with Phase 0 clients.
pub const PHASE: &str = "0-foundation";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_epsg_identifiers() {
        let variants = [
            "EPSG:4326",
            "epsg:4326",
            "EPSG::4326",
            "urn:ogc:def:crs:EPSG::4326",
        ];
        for value in variants {
            let crs = Crs::parse(value).expect("parse CRS");
            assert_eq!(crs.identifier(), "EPSG:4326");
            assert_eq!(crs.coordinate_unit(), CoordinateUnit::Degrees);
        }
    }

    #[test]
    fn rejects_unknown_and_invalid_crs_at_validation_boundary() {
        assert!(Crs::parse("WGS84").is_err());
        assert!(Crs::epsg(999_999).require_known().is_err());
        assert!(Crs::wgs84().validate_coordinate(181.0, 35.0).is_err());
        assert!(Crs::wgs84().validate_coordinate(137.0, 35.0).is_ok());
        assert_eq!(Crs::nagoya_projected().code(), NAGOYA_PROJECTED_EPSG);
        assert_eq!(Crs::epsg(32617).coordinate_unit(), CoordinateUnit::Metres);
        assert_eq!(Crs::new("epsg", 4326).identifier(), "EPSG:4326");
        assert!(Crs::new("OGC", 84).require_known().is_err());
    }

    #[test]
    fn serializes_spatial_metadata_without_losing_source_identity() {
        let mut source = SourceMetadata::new("file:///data/wards.geojson");
        source.dataset_id = Some("nagoya-wards-density".into());
        source.license = Some("MLIT N03".into());
        source.source_version = Some(SourceVersion::new("nagoya-2020-census-final-n03-v2"));
        let metadata = SpatialMetadata::new(Crs::wgs84(), source).expect("metadata");
        let json = serde_json::to_value(&metadata).expect("serialize");
        assert_eq!(json["crs"], "EPSG:4326");
        assert_eq!(json["coordinate_unit"], "degrees");
        assert_eq!(json["source"]["dataset_id"], "nagoya-wards-density");
        assert_eq!(
            json["source"]["source_version"],
            "nagoya-2020-census-final-n03-v2"
        );
        assert_eq!(json["source"]["checksum_status"], "unknown");
    }

    #[test]
    fn hashes_local_fixture_and_verifies_declared_checksum() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        );
        let source = SourceMetadata::from_uri(
            path,
            Some("sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e"),
            Some("nagoya-2020-census-final-n03-v2"),
        );
        assert_eq!(source.checksum_status, ChecksumVerification::Verified);
        assert!(source.checksum_verified());
        assert_eq!(
            source.checksum.as_deref(),
            Some("sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e")
        );
        assert_eq!(
            source.expected_checksum.as_deref(),
            Some("sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e")
        );
        assert_eq!(source.observed_checksum, source.checksum);
        assert_eq!(
            source.source_version.as_ref().map(SourceVersion::as_str),
            Some("nagoya-2020-census-final-n03-v2")
        );
        assert!(source.retrieved_at.is_none());
    }

    #[test]
    fn external_uri_without_checksum_is_explicitly_unknown() {
        let source = SourceMetadata::from_uri("https://example.test/data.geojson", None, None);
        assert_eq!(source.checksum, None);
        assert_eq!(source.expected_checksum, None);
        assert_eq!(source.observed_checksum, None);
        assert_eq!(source.checksum_status, ChecksumVerification::Unknown);
        assert!(!source.checksum_verified());
        assert!(source.retrieved_at.is_none());
    }

    #[test]
    fn external_declared_checksum_is_retained_until_bytes_are_observed() {
        let expected = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let source = SourceMetadata::from_uri(
            "https://example.test/data.geojson",
            Some(expected),
            Some("remote-v1"),
        );
        assert_eq!(source.checksum, Some(expected.into()));
        assert_eq!(source.expected_checksum, Some(expected.into()));
        assert_eq!(source.observed_checksum, None);
        assert_eq!(source.checksum_status, ChecksumVerification::Declared);
    }

    #[test]
    fn records_local_checksum_mismatch_without_marking_it_verified() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        );
        let source = SourceMetadata::from_uri(path, Some("sha256:00"), Some("bad-release"));
        assert_eq!(source.checksum_status, ChecksumVerification::Mismatch);
        assert_eq!(source.expected_checksum.as_deref(), Some("sha256:00"));
        assert_eq!(
            source.observed_checksum.as_deref(),
            Some("sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e")
        );
        assert_eq!(source.checksum, source.observed_checksum);
        assert!(!source.checksum_verified());
    }
}
