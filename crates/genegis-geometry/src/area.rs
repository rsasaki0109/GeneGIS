use genegis_crs::{CoordinateUnit, Crs, JGD2011_EPSG, WGS84_EPSG};
use serde::{Deserialize, Serialize};

use crate::PolygonRing;

/// Area calculation method recorded in workflow provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaMethod {
    /// Shoelace on WGS84 degrees scaled to km² at feature centroid latitude.
    PlanarWgs84Approx,
    /// WGS84 ellipsoidal surface area integrated from geographic coordinates.
    EllipsoidalWgs84,
    /// Shoelace area from projected coordinates whose unit is metres.
    PlanarProjected,
}

/// Compute polygon area in km² from WGS84 lon/lat ring using local planar scaling.
///
/// Not geodesic — suitable for MVP demo with explicit CRS/method metadata.
pub fn planar_area_km2_wgs84(ring: &[(f64, f64)]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }

    let mean_lat = ring.iter().map(|(_, lat)| lat).sum::<f64>() / ring.len() as f64;
    let lat_rad = mean_lat.to_radians();
    let km_per_deg_lat = 111.32;
    let km_per_deg_lon = 111.32 * lat_rad.cos();

    let mut area = 0.0;
    for i in 0..ring.len() {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[(i + 1) % ring.len()];
        let x1m = x1 * km_per_deg_lon;
        let y1m = y1 * km_per_deg_lat;
        let x2m = x2 * km_per_deg_lon;
        let y2m = y2 * km_per_deg_lat;
        area += x1m * y2m - x2m * y1m;
    }

    (area.abs() * 0.5).max(0.0)
}

/// Compute polygon area in km² from projected metre coordinates.
///
/// This is the ordinary shoelace formula. It is valid only when the caller
/// has already supplied a projected CRS with metre axes; use
/// [`area_km2_for_crs`] to keep that precondition explicit.
pub fn planar_area_km2_meters(ring: &[(f64, f64)]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }

    let mut area_m2 = 0.0;
    for i in 0..ring.len() {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[(i + 1) % ring.len()];
        area_m2 += x1 * y2 - x2 * y1;
    }
    (area_m2.abs() * 0.5 / 1_000_000.0).max(0.0)
}

// WGS84 semi-major axis and eccentricity. The surface integral below is
// evaluated in radians and converted to km² at the final boundary.
const WGS84_A_METERS: f64 = 6_378_137.0;
const WGS84_INV_FLATTENING: f64 = 298.257_223_563;
const GAUSS_NODES: [f64; 8] = [
    -0.960_289_856_497_536_3,
    -0.796_666_477_413_626_7,
    -0.525_532_409_916_329,
    -0.183_434_642_495_649_8,
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const GAUSS_WEIGHTS: [f64; 8] = [
    0.101_228_536_290_376_3,
    0.222_381_034_453_374_5,
    0.313_706_645_877_887_3,
    0.362_683_783_378_362,
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

/// Compute the area of a WGS84 longitude/latitude ring on the reference
/// ellipsoid in km².
///
/// GeoJSON geographic edges are interpreted as straight segments in
/// longitude/latitude, which is the representation used by the bundled N03
/// boundaries. Green's theorem reduces the ellipsoidal surface integral to a
/// one-dimensional integral of the meridional area primitive. Eight-point
/// Gauss–Legendre quadrature makes the result stable for both small parcel
/// edges and large administrative rings without the latitude-scale shortcut
/// used by [`planar_area_km2_wgs84`].
pub fn ellipsoidal_area_km2_wgs84(ring: &[(f64, f64)]) -> f64 {
    if ring.len() < 3
        || ring.iter().any(|(lon, lat)| {
            !lon.is_finite()
                || !lat.is_finite()
                || !(-180.0..=180.0).contains(lon)
                || !(-90.0..=90.0).contains(lat)
        })
    {
        return 0.0;
    }

    let flattening = 1.0 / WGS84_INV_FLATTENING;
    let eccentricity_squared = flattening * (2.0 - flattening);
    let eccentricity = eccentricity_squared.sqrt();
    let scale = WGS84_A_METERS * WGS84_A_METERS * (1.0 - eccentricity_squared);
    let mut integral_m2 = 0.0;

    for i in 0..ring.len() {
        let (lon1, lat1) = ring[i];
        let (lon2, lat2) = ring[(i + 1) % ring.len()];
        let lon1 = lon1.to_radians();
        let lon2 = lon2.to_radians();
        let lat1 = lat1.to_radians();
        let lat2 = lat2.to_radians();
        let delta_lon = normalize_longitude_delta(lon2 - lon1);
        let midpoint = (lat1 + lat2) * 0.5;
        let half_delta_lat = (lat2 - lat1) * 0.5;

        // Integrate F(latitude(t)) dlongitude over t ∈ [-1, 1].
        let mut edge_integral = 0.0;
        for (node, weight) in GAUSS_NODES.iter().zip(GAUSS_WEIGHTS.iter()) {
            let latitude = midpoint + half_delta_lat * node;
            edge_integral += weight
                * meridional_surface_primitive(latitude, eccentricity, eccentricity_squared, scale);
        }
        integral_m2 += delta_lon * edge_integral;
    }

    (integral_m2.abs() * 0.5).max(0.0) / 1_000_000.0
}

/// Compute total WGS84 ellipsoidal area for one or more exterior rings.
pub fn ellipsoidal_area_km2_rings(rings: &[&[(f64, f64)]]) -> f64 {
    rings
        .iter()
        .map(|ring| ellipsoidal_area_km2_wgs84(ring))
        .sum()
}

/// Alias for [`ellipsoidal_area_km2_wgs84`] using the common geodesic-area
/// terminology used by workflow and plugin authors.
pub fn geodesic_area_km2_wgs84(ring: &[(f64, f64)]) -> f64 {
    ellipsoidal_area_km2_wgs84(ring)
}

/// Alias for [`ellipsoidal_area_km2_rings`] using geodesic-area terminology.
pub fn geodesic_area_km2_rings(rings: &[&[(f64, f64)]]) -> f64 {
    ellipsoidal_area_km2_rings(rings)
}

/// Compute area using the coordinate units declared by a known CRS.
///
/// Geographic WGS84 and JGD2011 coordinates use the ellipsoidal calculation;
/// known projected metre CRSs use metre-based shoelace area. Unsupported
/// geographic/projected definitions fail closed instead of silently treating
/// degrees as metres.
pub fn area_km2_for_crs(ring: &[(f64, f64)], crs: &Crs) -> Result<f64, AreaError> {
    let definition = crs
        .require_known()
        .map_err(|_| AreaError::UnsupportedCrs(crs.to_string()))?;
    for &(x, y) in ring {
        crs.validate_coordinate(x, y)
            .map_err(|err| AreaError::InvalidCoordinate(err.to_string()))?;
    }
    match definition.code {
        WGS84_EPSG | JGD2011_EPSG => Ok(ellipsoidal_area_km2_wgs84(ring)),
        _ if definition.unit == CoordinateUnit::Metres => Ok(planar_area_km2_meters(ring)),
        _ => Err(AreaError::UnsupportedCrs(crs.to_string())),
    }
}

/// Compute total area across exterior rings using one declared CRS.
pub fn area_km2_rings_for_crs(rings: &[&[(f64, f64)]], crs: &Crs) -> Result<f64, AreaError> {
    rings.iter().map(|ring| area_km2_for_crs(ring, crs)).sum()
}

/// Compute one polygon-part area, subtracting all interior rings (holes).
///
/// A `PolygonRing` deliberately keeps the exterior and holes together so a
/// format adapter cannot accidentally turn a lake or enclave into filled
/// land.  Each ring is evaluated with the same CRS-aware method and the
/// result is clamped at zero for malformed, fully-covered parts.
pub fn polygon_area_km2_for_crs(polygon: &PolygonRing, crs: &Crs) -> Result<f64, AreaError> {
    let exterior = area_km2_for_crs(polygon.exterior(), crs)?;
    let holes = polygon
        .holes()
        .iter()
        .map(|hole| area_km2_for_crs(hole, crs))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((exterior - holes.into_iter().sum::<f64>()).max(0.0))
}

/// Compute total area for all polygon parts in one feature, subtracting
/// holes within each part.
pub fn polygon_parts_area_km2_for_crs(
    polygons: &[PolygonRing],
    crs: &Crs,
) -> Result<f64, AreaError> {
    polygons
        .iter()
        .map(|polygon| polygon_area_km2_for_crs(polygon, crs))
        .sum()
}

fn normalize_longitude_delta(delta: f64) -> f64 {
    let two_pi = std::f64::consts::TAU;
    (delta + std::f64::consts::PI).rem_euclid(two_pi) - std::f64::consts::PI
}

fn meridional_surface_primitive(
    latitude: f64,
    eccentricity: f64,
    eccentricity_squared: f64,
    scale: f64,
) -> f64 {
    let sine = latitude.sin();
    let denominator = 1.0 - eccentricity_squared * sine * sine;
    scale * 0.5 * (sine / denominator + (eccentricity * sine).atanh() / eccentricity)
}

/// Errors raised when an area operation cannot honor CRS semantics.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AreaError {
    /// The CRS is not known by the built-in area implementation.
    #[error("unsupported CRS for area calculation: {0}")]
    UnsupportedCrs(String),
    /// A coordinate cannot be interpreted in the declared CRS.
    #[error("invalid coordinate for area calculation: {0}")]
    InvalidCoordinate(String),
}

/// Compute total area in km² across one or more exterior rings.
pub fn planar_area_km2_rings(rings: &[&[(f64, f64)]]) -> f64 {
    rings.iter().map(|ring| planar_area_km2_wgs84(ring)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_square_about_one_km2_at_nagoya_lat() {
        // ~0.009 deg ≈ 1 km at 35°N
        let ring = [
            (136.90, 35.15),
            (136.909, 35.15),
            (136.909, 35.159),
            (136.90, 35.159),
            (136.90, 35.15),
        ];
        let area = planar_area_km2_wgs84(&ring);
        assert!(area > 0.8 && area < 1.2, "area={area}");
    }

    #[test]
    fn ellipsoidal_area_is_stable_and_differs_from_degree_scaling() {
        let ring = [
            (136.90, 35.15),
            (136.909, 35.15),
            (136.909, 35.159),
            (136.90, 35.159),
            (136.90, 35.15),
        ];
        let planar = planar_area_km2_wgs84(&ring);
        let ellipsoidal = ellipsoidal_area_km2_wgs84(&ring);
        assert!(ellipsoidal > 0.8 && ellipsoidal < 1.2, "area={ellipsoidal}");
        assert!((ellipsoidal - planar).abs() > 1e-6);
        assert_eq!(
            area_km2_for_crs(&ring, &Crs::wgs84()).expect("area"),
            ellipsoidal
        );
    }

    #[test]
    fn projected_area_requires_metre_coordinates() {
        let ring = [
            (0.0, 0.0),
            (1_000.0, 0.0),
            (1_000.0, 1_000.0),
            (0.0, 1_000.0),
        ];
        let area = area_km2_for_crs(&ring, &Crs::nagoya_projected()).expect("area");
        assert!((area - 1.0).abs() < 1e-12);
        assert!(area_km2_for_crs(&ring, &Crs::epsg(9999)).is_err());
        assert!(area_km2_for_crs(
            &[(181.0, 35.0), (182.0, 35.0), (181.0, 36.0)],
            &Crs::wgs84()
        )
        .is_err());
    }

    #[test]
    fn polygon_area_subtracts_holes() {
        let polygon = PolygonRing::with_holes(
            vec![
                (0.0, 0.0),
                (0.01, 0.0),
                (0.01, 0.01),
                (0.0, 0.01),
                (0.0, 0.0),
            ],
            vec![vec![
                (0.002, 0.002),
                (0.002, 0.008),
                (0.008, 0.008),
                (0.008, 0.002),
                (0.002, 0.002),
            ]],
        );
        let exterior = area_km2_for_crs(polygon.exterior(), &Crs::wgs84()).expect("exterior");
        let hole = area_km2_for_crs(&polygon.holes()[0], &Crs::wgs84()).expect("hole");
        let actual = polygon_area_km2_for_crs(&polygon, &Crs::wgs84()).expect("polygon");
        assert!((actual - (exterior - hole)).abs() < 1e-12);
        assert!(actual < exterior);
    }
}
