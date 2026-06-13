use std::collections::HashMap;

use genegis_geometry::BoundingBox;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dataset::DatasetRecord;

/// Default GeneGIS alpha STAC Collection id.
pub const ALPHA_STAC_COLLECTION_ID: &str = "genegis-alpha";

/// Minimal STAC 1.0 Collection (alpha subset for catalog browse).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacCollection {
    pub stac_version: String,
    #[serde(rename = "type")]
    pub collection_type: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub license: String,
    pub extent: Value,
    pub links: Vec<StacLink>,
    #[serde(default)]
    pub summaries: Value,
}

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

impl StacCollection {
    pub fn summary_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "license": self.license,
            "item_count": self.summaries.get("item_count"),
            "item_ids": self.summaries.get("item_ids"),
        })
    }
}

/// Browse the alpha catalog as a STAC Collection with linked item ids.
pub fn browse_alpha_stac_collection(catalog: &crate::Catalog) -> StacCollection {
    let records = catalog.list();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let item_ids: Vec<String> = records.iter().map(|record| record.id.clone()).collect();

    for record in &records {
        min_x = min_x.min(record.bbox.min.x);
        min_y = min_y.min(record.bbox.min.y);
        max_x = max_x.max(record.bbox.max.x);
        max_y = max_y.max(record.bbox.max.y);
    }

    StacCollection {
        stac_version: "1.0.0".into(),
        collection_type: "Collection".into(),
        id: ALPHA_STAC_COLLECTION_ID.into(),
        title: "GeneGIS Alpha Catalog".into(),
        description: "Bundled MVP datasets for Nagoya density and COG metadata workflows.".into(),
        license: "Mixed open data + GeneGIS smoke fixtures".into(),
        extent: json!({
            "spatial": { "bbox": [[min_x, min_y, max_x, max_y]] },
            "temporal": { "interval": [[null, null]] }
        }),
        links: vec![StacLink {
            rel: "self".into(),
            href: format!("catalog://{ALPHA_STAC_COLLECTION_ID}"),
            media_type: Some("application/json".into()),
        }],
        summaries: json!({
            "item_count": item_ids.len(),
            "item_ids": item_ids,
            "formats": records.iter().map(|record| record.format.kind.clone()).collect::<Vec<_>>(),
        }),
    }
}

/// Bind a catalog dataset to a STAC Item for planner / audit export.
pub fn bind_stac_item(
    catalog: &crate::Catalog,
    item_id: &str,
) -> Result<StacItem, crate::CatalogError> {
    catalog.require(item_id).map(|record| record.to_stac_item())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{alpha_catalog, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID, REMOTE_COG_DEMO_ID};

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

    #[test]
    fn alpha_stac_collection_lists_catalog_items() {
        let catalog = alpha_catalog();
        let collection = browse_alpha_stac_collection(&catalog);
        assert_eq!(collection.id, ALPHA_STAC_COLLECTION_ID);
        assert_eq!(
            collection.summaries.get("item_count").and_then(Value::as_u64),
            Some(3)
        );
        let ids = collection
            .summaries
            .get("item_ids")
            .and_then(Value::as_array)
            .expect("item_ids");
        assert!(ids.iter().any(|id| id.as_str() == Some(NAGOYA_WARDS_DENSITY_ID)));
        assert!(ids.iter().any(|id| id.as_str() == Some(REMOTE_COG_DEMO_ID)));
        assert!(ids.iter().any(|id| id.as_str() == Some(LOCAL_COG_DEMO_ID)));
    }

    #[test]
    fn bind_stac_item_returns_catalog_item() {
        let item = bind_stac_item(&alpha_catalog(), LOCAL_COG_DEMO_ID).expect("item");
        assert_eq!(item.id, LOCAL_COG_DEMO_ID);
        assert!(item.assets.contains_key("cog"));
    }
}
