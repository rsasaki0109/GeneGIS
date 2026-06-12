use serde::{Deserialize, Serialize};

/// Area calculation method recorded in workflow provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaMethod {
    /// Shoelace on WGS84 degrees scaled to km² at feature centroid latitude.
    PlanarWgs84Approx,
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
}
