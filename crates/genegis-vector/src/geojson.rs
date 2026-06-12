use geojson::{FeatureCollection, GeoJson, Geometry, Value as GeoValue};

use crate::dataset::{FeatureRecord, VectorDataset};
use crate::error::VectorError;
use genegis_geometry::BoundingBox;

pub fn read_geojson_str(text: &str) -> Result<VectorDataset, VectorError> {
    let geo: GeoJson = text
        .parse()
        .map_err(|e: geojson::Error| VectorError::GeoJson(e.to_string()))?;

    let collection = match geo {
        GeoJson::FeatureCollection(fc) => fc,
        _ => {
            return Err(VectorError::GeoJson(
                "expected FeatureCollection".into(),
            ))
        }
    };

    parse_collection(collection)
}

pub fn read_geojson_path(path: &str) -> Result<VectorDataset, VectorError> {
    let text = std::fs::read_to_string(path)?;
    read_geojson_str(&text)
}

fn parse_collection(collection: FeatureCollection) -> Result<VectorDataset, VectorError> {
    let name = collection
        .foreign_members
        .as_ref()
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("unnamed")
        .to_string();

    let crs = collection
        .foreign_members
        .as_ref()
        .and_then(|m| m.get("crs"))
        .and_then(|v| v.as_str())
        .unwrap_or("EPSG:4326")
        .to_string();

    let mut features = Vec::new();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for (idx, feature) in collection.features.into_iter().enumerate() {
        let geometry = feature.geometry.ok_or_else(|| {
            VectorError::UnsupportedGeometry(format!("feature {idx} has no geometry"))
        })?;
        let rings = geometry_to_rings(&geometry)?;

        for ring in &rings {
            for (x, y) in ring.exterior() {
                min_x = min_x.min(*x);
                min_y = min_y.min(*y);
                max_x = max_x.max(*x);
                max_y = max_y.max(*y);
            }
        }

        features.push(FeatureRecord {
            id: idx,
            properties: match feature.properties {
                Some(ref props) => {
                    serde_json::to_value(props).unwrap_or(serde_json::Value::Null)
                }
                None => serde_json::Value::Null,
            },
            rings,
        });
    }

    let bbox = if features.is_empty() {
        BoundingBox::new(0.0, 0.0, 0.0, 0.0)
    } else {
        BoundingBox::new(min_x, min_y, max_x, max_y)
    };

    Ok(VectorDataset {
        name,
        crs,
        features,
        bbox,
    })
}

fn geometry_to_rings(geometry: &Geometry) -> Result<Vec<genegis_geometry::PolygonRing>, VectorError> {
    match &geometry.value {
        GeoValue::Polygon(polygon) => {
            let exterior = polygon
                .first()
                .ok_or_else(|| VectorError::UnsupportedGeometry("empty polygon".into()))?;
            let coords = positions_to_lon_lat(exterior)?;
            Ok(vec![genegis_geometry::PolygonRing::new(coords)])
        }
        GeoValue::MultiPolygon(multi) => {
            let mut rings = Vec::new();
            for polygon in multi {
                let exterior = polygon
                    .first()
                    .ok_or_else(|| VectorError::UnsupportedGeometry("empty polygon".into()))?;
                let coords = positions_to_lon_lat(exterior)?;
                rings.push(genegis_geometry::PolygonRing::new(coords));
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

fn positions_to_lon_lat(
    positions: &[geojson::Position],
) -> Result<Vec<(f64, f64)>, VectorError> {
    positions
        .iter()
        .map(|p| {
            if p.len() < 2 {
                Err(VectorError::GeoJson("position needs lon/lat".into()))
            } else {
                Ok((p[0], p[1]))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_demo_collection() {
        let text = include_str!(
            "../../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        );
        let ds = read_geojson_str(text).expect("parse");
        assert_eq!(ds.feature_count(), 16);
    }
}
