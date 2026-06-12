use genegis_geometry::PolygonRing;
use geo_types::{Coord, Geometry, Polygon};

use crate::error::VectorError;

pub fn geo_geometry_to_rings(geometry: &Geometry) -> Result<Vec<PolygonRing>, VectorError> {
    match geometry {
        Geometry::Polygon(polygon) => Ok(vec![polygon_to_ring(polygon)?]),
        Geometry::MultiPolygon(multi) => {
            let mut rings = Vec::new();
            for polygon in &multi.0 {
                rings.push(polygon_to_ring(polygon)?);
            }
            if rings.is_empty() {
                return Err(VectorError::UnsupportedGeometry("empty multipolygon".into()));
            }
            Ok(rings)
        }
        other => Err(VectorError::UnsupportedGeometry(format!(
            "{other:?} not supported in MVP"
        ))),
    }
}

fn polygon_to_ring(polygon: &Polygon<f64>) -> Result<PolygonRing, VectorError> {
    let exterior = line_string_to_coords(polygon.exterior())?;
    Ok(PolygonRing::new(exterior))
}

fn line_string_to_coords(line: &geo_types::LineString<f64>) -> Result<Vec<(f64, f64)>, VectorError> {
    if line.0.len() < 4 {
        return Err(VectorError::UnsupportedGeometry(
            "polygon ring needs at least 4 positions".into(),
        ));
    }
    Ok(line
        .0
        .iter()
        .map(|Coord { x, y, .. }| (*x, *y))
        .collect())
}
