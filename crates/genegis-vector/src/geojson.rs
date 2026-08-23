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
        _ => return Err(VectorError::GeoJson("expected FeatureCollection".into())),
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
                Some(ref props) => serde_json::to_value(props).unwrap_or(serde_json::Value::Null),
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

fn geometry_to_rings(
    geometry: &Geometry,
) -> Result<Vec<genegis_geometry::PolygonRing>, VectorError> {
    match &geometry.value {
        GeoValue::Polygon(polygon) => Ok(vec![polygon_to_ring(polygon, "polygon")?]),
        GeoValue::MultiPolygon(multi) => {
            let mut rings = Vec::new();
            for (index, polygon) in multi.iter().enumerate() {
                rings.push(polygon_to_ring(polygon, &format!("multipolygon {index}"))?);
            }
            if rings.is_empty() {
                return Err(VectorError::UnsupportedGeometry(
                    "empty multipolygon".into(),
                ));
            }
            Ok(rings)
        }
        other => Err(VectorError::UnsupportedGeometry(format!(
            "{other:?} not supported in MVP"
        ))),
    }
}

fn polygon_to_ring(
    polygon: &[Vec<geojson::Position>],
    context: &str,
) -> Result<genegis_geometry::PolygonRing, VectorError> {
    let exterior = polygon
        .first()
        .ok_or_else(|| VectorError::UnsupportedGeometry(format!("empty {context}")))?;
    let coords = positions_to_lon_lat(exterior)?;
    let holes = polygon
        .iter()
        .skip(1)
        .map(|hole| positions_to_lon_lat(hole))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(genegis_geometry::PolygonRing::with_holes(coords, holes))
}

fn positions_to_lon_lat(positions: &[geojson::Position]) -> Result<Vec<(f64, f64)>, VectorError> {
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
        let text =
            include_str!("../../../examples/nagoya-population-density/data/nagoya-wards.geojson");
        let ds = read_geojson_str(text).expect("parse");
        assert_eq!(ds.feature_count(), 16);
    }

    #[test]
    fn rejects_missing_geometry_and_unsupported_geometry() {
        let missing =
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{}}]}"#;
        let error = read_geojson_str(missing).expect_err("missing geometry must fail closed");
        assert!(error.to_string().contains("geometry"), "error={error}");

        let line = r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]}}]}"#;
        let error = read_geojson_str(line).expect_err("line geometry must fail closed");
        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn preserves_multipart_polygons_and_holes() {
        let text = r#"
        {
          "type":"FeatureCollection",
          "features":[
            {"type":"Feature","properties":{"ward_code":"hole"},"geometry":
              {"type":"Polygon","coordinates":[
                [[0,0],[10,0],[10,10],[0,10],[0,0]],
                [[2,2],[2,4],[4,4],[4,2],[2,2]]
              ]}},
            {"type":"Feature","properties":{"ward_code":"multi"},"geometry":
              {"type":"MultiPolygon","coordinates":[
                [[[20,20],[21,20],[21,21],[20,21],[20,20]]],
                [[[22,22],[23,22],[23,23],[22,23],[22,22]]]
              ]}}
          ]
        }
        "#;
        let dataset = read_geojson_str(text).expect("multipart fixture");
        assert_eq!(dataset.features[0].rings.len(), 1);
        assert_eq!(dataset.features[0].rings[0].holes().len(), 1);
        assert_eq!(dataset.features[1].rings.len(), 2);
        assert!(dataset.features[1]
            .rings
            .iter()
            .all(|ring| ring.holes().is_empty()));
    }
}
