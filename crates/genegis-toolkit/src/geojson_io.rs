//! GeoJSON reading and writing.

use std::collections::BTreeMap;

use geo_types::Geometry;
use geojson::{Feature as GjFeature, FeatureCollection, GeoJson, JsonObject};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::import::{coordinates_look_geographic, ImportOptions, ImportReport};
use crate::layer::{CrsStatus, Feature, Layer};
use crate::proj;

/// Read GeoJSON (FeatureCollection, Feature, or bare Geometry).
pub fn read(text: &str, options: &ImportOptions, report: &mut ImportReport) -> Result<Layer> {
    let parsed: GeoJson = text
        .parse()
        .map_err(|e: geojson::Error| ToolkitError::import("geojson", e.to_string()))?;
    let (features, foreign) = match parsed {
        GeoJson::FeatureCollection(FeatureCollection {
            features,
            foreign_members,
            ..
        }) => (features, foreign_members),
        GeoJson::Feature(feature) => (vec![feature], None),
        GeoJson::Geometry(geometry) => (
            vec![GjFeature {
                geometry: Some(geometry),
                ..Default::default()
            }],
            None,
        ),
    };

    let declared = foreign.as_ref().and_then(declared_crs);
    let name = foreign
        .as_ref()
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut layer = Layer::new(
        options
            .name
            .clone()
            .or(name)
            .unwrap_or_else(|| "geojson".into()),
        "EPSG:4326",
        CrsStatus::FormatDefault,
    );
    for (index, feature) in features.into_iter().enumerate() {
        let geometry =
            match &feature.geometry {
                Some(g) => Some(Geometry::<f64>::try_from(&g.value).map_err(|e| {
                    ToolkitError::import("geojson", format!("feature {index}: {e}"))
                })?),
                None => None,
            };
        let mut properties: BTreeMap<String, Value> =
            feature.properties.unwrap_or_default().into_iter().collect();
        if let Some(id) = feature.id {
            let id = match id {
                geojson::feature::Id::String(s) => Value::from(s),
                geojson::feature::Id::Number(n) => Value::Number(n),
            };
            properties.entry("id".into()).or_insert(id);
        }
        layer.features.push(Feature {
            id: index as u64,
            geometry,
            properties,
        });
    }

    if let Some(user) = &options.crs {
        layer.crs = proj::lookup(user)?.id;
        layer.crs_status = CrsStatus::UserSupplied;
    } else if let Some(declared) = declared {
        layer.crs = proj::lookup(&declared)?.id;
        layer.crs_status = CrsStatus::Declared;
        report
            .notes
            .push(format!("legacy GeoJSON crs member declared {}", layer.crs));
    } else if !coordinates_look_geographic(&layer) {
        return Err(ToolkitError::CrsRequired(
            "GeoJSON coordinates are outside the longitude/latitude range and no CRS is declared; \
             RFC 7946 requires EPSG:4326, so choose the source CRS explicitly"
                .into(),
        ));
    } else {
        report
            .notes
            .push("CRS taken from RFC 7946 (GeoJSON is EPSG:4326)".into());
    }
    layer.refresh_schema();
    Ok(layer)
}

fn declared_crs(members: &JsonObject) -> Option<String> {
    match members.get("crs")? {
        Value::String(s) => Some(s.clone()),
        Value::Object(obj) => obj
            .get("properties")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

/// Write a layer as GeoJSON. RFC 7946 output is always EPSG:4326; set
/// `keep_crs` to emit the native CRS with a legacy `crs` member instead.
pub fn write(layer: &Layer, keep_crs: bool) -> Result<String> {
    let layer = if keep_crs {
        layer.clone()
    } else {
        layer.reprojected(&proj::lookup_epsg(4326)?)?
    };
    let features = layer
        .features
        .iter()
        .map(|feature| GjFeature {
            bbox: None,
            geometry: feature
                .geometry
                .as_ref()
                .map(|g| geojson::Geometry::new(geojson::Value::from(g))),
            id: None,
            properties: Some(
                feature
                    .properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            foreign_members: None,
        })
        .collect();
    let mut foreign = JsonObject::new();
    foreign.insert("name".into(), Value::from(layer.name.clone()));
    if keep_crs && layer.crs != "EPSG:4326" {
        foreign.insert(
            "crs".into(),
            serde_json::json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:{}", layer.crs.replace(':', "::"))}}),
        );
    }
    let collection = FeatureCollection {
        bbox: None,
        features,
        foreign_members: Some(foreign),
    };
    Ok(GeoJson::FeatureCollection(collection).to_string())
}

/// Convert a GeoJSON geometry value (as JSON) into a geo geometry.
pub fn geometry_from_json(value: &Value) -> Result<Geometry<f64>> {
    let geometry = geojson::Geometry::from_json_value(value.clone())
        .map_err(|e| ToolkitError::import("geojson", e.to_string()))?;
    Geometry::<f64>::try_from(&geometry.value)
        .map_err(|e| ToolkitError::import("geojson", e.to_string()))
}

/// Convert a geo geometry to a GeoJSON geometry JSON value.
pub fn geometry_to_json(geometry: &Geometry<f64>) -> Value {
    serde_json::to_value(geojson::Geometry::new(geojson::Value::from(geometry)))
        .unwrap_or(Value::Null)
}
