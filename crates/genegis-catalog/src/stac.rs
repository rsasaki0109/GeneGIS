use std::collections::HashMap;

use genegis_geometry::BoundingBox;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dataset::DatasetRecord;

/// Minimal STAC 1.0 Item (alpha subset for catalog export).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacItem {
    pub stac_version: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub id: String,
    pub geometry: Value,
    pub bbox: [f64; 4],
    pub properties: Value,
    pub assets: HashMap<String, StacAsset>,
    pub links: Vec<StacLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacAsset {
    pub href: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub roles: Vec<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacLink {
    pub rel: String,
    pub href: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl DatasetRecord {
    /// Export this catalog record as a STAC 1.0 Item JSON object.
    pub fn to_stac_item(&self) -> StacItem {
        let bbox = bbox_to_array(self.bbox);
        StacItem {
            stac_version: "1.0.0".into(),
            item_type: "Feature".into(),
            id: self.id.clone(),
            geometry: bbox_polygon_geometry(bbox),
            bbox,
            properties: stac_properties(self),
            assets: stac_assets(self),
            links: vec![StacLink {
                rel: "self".into(),
                href: format!("catalog://{}", self.id),
                media_type: Some("application/json".into()),
            }],
        }
    }
}

fn bbox_to_array(bbox: BoundingBox) -> [f64; 4] {
    [bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y]
}

fn bbox_polygon_geometry(bbox: [f64; 4]) -> Value {
    let [min_x, min_y, max_x, max_y] = bbox;
    json!({
        "type": "Polygon",
        "coordinates": [[
            [min_x, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [min_x, max_y],
            [min_x, min_y],
        ]]
    })
}

fn stac_properties(record: &DatasetRecord) -> Value {
    json!({
        "title": record.title,
        "description": record.description,
        "genegis:format": record.format.kind,
        "genegis:crs": record.crs,
        "genegis:license": record.license,
        "genegis:tags": record.tags,
    })
}

fn stac_assets(record: &DatasetRecord) -> HashMap<String, StacAsset> {
    let mut assets = HashMap::new();
    assets.insert(
        record.format.kind.clone(),
        StacAsset {
            href: record.uri.clone(),
            media_type: record.format.media_type.clone(),
            roles: vec!["data".into()],
            title: Some(record.title.clone()),
        },
    );
    assets
}

#[cfg(test)]
mod tests {
    use crate::catalog::{alpha_catalog, NAGOYA_WARDS_DENSITY_ID};

    #[test]
    fn nagoya_exports_stac_item() {
        let catalog = alpha_catalog();
        let record = catalog
            .require(NAGOYA_WARDS_DENSITY_ID)
            .expect("record");
        let item = record.to_stac_item();
        assert_eq!(item.stac_version, "1.0.0");
        assert_eq!(item.id, NAGOYA_WARDS_DENSITY_ID);
        assert!(item.assets.contains_key("geojson"));
        assert_eq!(item.bbox[0] < item.bbox[2], true);
    }
}
