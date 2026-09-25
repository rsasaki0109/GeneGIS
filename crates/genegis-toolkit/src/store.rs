//! Content-addressed layer store with optional on-disk persistence.
//!
//! Layers are keyed by their content-derived ID, so re-importing the same
//! data is idempotent and a stored layer can never silently change under a
//! workflow that pinned its digest. Persistence is lossless JSON; every file
//! is re-verified against its ID when loaded.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::geojson_io::{geometry_from_json, geometry_to_json};
use crate::layer::{CrsStatus, Feature, Field, Layer, LayerProvenance, LayerSummary};

/// A stored layer with its receipt and view settings.
#[derive(Debug, Clone)]
pub struct StoredLayer {
    /// The layer.
    pub layer: Layer,
    /// Receipt of the command that produced it (import, analysis, place).
    pub receipt: Value,
    /// UI style (classification, colour, visibility).
    pub style: Value,
}

/// Listing entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSummary {
    /// Layer summary.
    #[serde(flatten)]
    pub summary: LayerSummary,
    /// Receipt.
    pub receipt: Value,
    /// Style.
    pub style: Value,
}

/// Layer store.
#[derive(Debug, Default)]
pub struct LayerStore {
    dir: Option<PathBuf>,
    layers: BTreeMap<String, StoredLayer>,
    order: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct PersistedFeature {
    id: u64,
    geometry: Value,
    properties: BTreeMap<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct Persisted {
    format: String,
    id: String,
    name: String,
    crs: String,
    crs_status: CrsStatus,
    fields: Vec<Field>,
    provenance: LayerProvenance,
    receipt: Value,
    style: Value,
    features: Vec<PersistedFeature>,
}

impl LayerStore {
    /// In-memory store.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Store persisted under `dir`; existing layers are loaded and verified.
    /// Returns the store and warnings for files that failed verification.
    pub fn open(dir: impl Into<PathBuf>) -> Result<(Self, Vec<String>)> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let mut store = Self {
            dir: Some(dir.clone()),
            ..Self::default()
        };
        let mut warnings = Vec::new();
        let mut entries: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "json")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("lyr_"))
            })
            .map(|p| {
                (
                    std::fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH),
                    p,
                )
            })
            .collect();
        entries.sort();
        for (_, path) in entries {
            match load(&path) {
                Ok((id, stored)) => {
                    store.order.push(id.clone());
                    store.layers.insert(id, stored);
                }
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
        Ok((store, warnings))
    }

    /// Insert (or replace) a layer; returns its ID.
    pub fn insert(&mut self, layer: Layer, receipt: Value) -> Result<String> {
        let id = layer.id();
        let style = self
            .layers
            .get(&id)
            .map(|s| s.style.clone())
            .unwrap_or(Value::Null);
        let stored = StoredLayer {
            layer,
            receipt,
            style,
        };
        self.persist(&id, &stored)?;
        if !self.layers.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.layers.insert(id.clone(), stored);
        Ok(id)
    }

    /// Get a stored layer.
    pub fn get(&self, id: &str) -> Option<&StoredLayer> {
        self.layers.get(id)
    }

    /// Layer lookup, with an error for unknown IDs.
    pub fn layer(&self, id: &str) -> Result<&Layer> {
        self.layers
            .get(id)
            .map(|s| &s.layer)
            .ok_or_else(|| ToolkitError::UnknownReference(format!("layer {id}")))
    }

    /// All layers keyed by ID (for planning and execution).
    pub fn layers(&self) -> BTreeMap<String, Layer> {
        self.layers
            .iter()
            .map(|(k, v)| (k.clone(), v.layer.clone()))
            .collect()
    }

    /// Summaries in insertion order.
    pub fn list(&self) -> Vec<StoredSummary> {
        self.order
            .iter()
            .filter_map(|id| self.layers.get(id))
            .map(|s| StoredSummary {
                summary: s.layer.summary(),
                receipt: s.receipt.clone(),
                style: s.style.clone(),
            })
            .collect()
    }

    /// Rename a layer (names are not part of the content digest).
    pub fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        let mut stored = self
            .layers
            .get(id)
            .cloned()
            .ok_or_else(|| ToolkitError::UnknownReference(id.into()))?;
        stored.layer.name = name.trim().to_string();
        self.persist(id, &stored)?;
        self.layers.insert(id.to_string(), stored);
        Ok(())
    }

    /// Replace a layer's view style.
    pub fn set_style(&mut self, id: &str, style: Value) -> Result<()> {
        let mut stored = self
            .layers
            .get(id)
            .cloned()
            .ok_or_else(|| ToolkitError::UnknownReference(id.into()))?;
        stored.style = style;
        self.persist(id, &stored)?;
        self.layers.insert(id.to_string(), stored);
        Ok(())
    }

    /// Remove a layer.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        if self.layers.remove(id).is_none() {
            return Err(ToolkitError::UnknownReference(id.into()));
        }
        self.order.retain(|x| x != id);
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{id}.json"));
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    fn persist(&self, id: &str, stored: &StoredLayer) -> Result<()> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let layer = &stored.layer;
        let persisted = Persisted {
            format: "genegis-layer-store-v1".into(),
            id: id.to_string(),
            name: layer.name.clone(),
            crs: layer.crs.clone(),
            crs_status: layer.crs_status,
            fields: layer.fields.clone(),
            provenance: layer.provenance.clone(),
            receipt: stored.receipt.clone(),
            style: stored.style.clone(),
            features: layer
                .features
                .iter()
                .map(|f| PersistedFeature {
                    id: f.id,
                    geometry: f
                        .geometry
                        .as_ref()
                        .map(geometry_to_json)
                        .unwrap_or(Value::Null),
                    properties: f.properties.clone(),
                })
                .collect(),
        };
        let path = dir.join(format!("{id}.json"));
        let temp = dir.join(format!("{id}.json.tmp"));
        std::fs::write(
            &temp,
            serde_json::to_vec(&persisted).map_err(|e| ToolkitError::Command(e.to_string()))?,
        )?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

fn load(path: &std::path::Path) -> Result<(String, StoredLayer)> {
    let bytes = std::fs::read(path)?;
    let persisted: Persisted =
        serde_json::from_slice(&bytes).map_err(|e| ToolkitError::import("store", e.to_string()))?;
    let mut layer = Layer::new(persisted.name, persisted.crs, persisted.crs_status);
    layer.fields = persisted.fields;
    layer.provenance = persisted.provenance;
    for f in persisted.features {
        layer.features.push(Feature {
            id: f.id,
            geometry: if f.geometry.is_null() {
                None
            } else {
                Some(geometry_from_json(&f.geometry)?)
            },
            properties: f.properties,
        });
    }
    if layer.id() != persisted.id {
        return Err(ToolkitError::Verification(format!(
            "stored layer {} no longer matches its content digest",
            persisted.id
        )));
    }
    Ok((
        persisted.id,
        StoredLayer {
            layer,
            receipt: persisted.receipt,
            style: persisted.style,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{import_bytes, ImportOptions};

    #[test]
    fn persists_losslessly_and_rejects_tampering() {
        let dir = std::env::temp_dir().join(format!("genegis-store-{}", uuid::Uuid::new_v4()));
        let (mut store, warnings) = LayerStore::open(&dir).unwrap();
        assert!(warnings.is_empty());
        let csv = "name,lon,lat,v,code\nA,136.9,35.1,1.5,01101\n";
        let (layer, _) = import_bytes("a.csv", csv.as_bytes(), &ImportOptions::default()).unwrap();
        let id = store
            .insert(layer.clone(), serde_json::json!({"kind": "import"}))
            .unwrap();
        store.rename(&id, "地点A").unwrap();
        drop(store);

        let (reopened, warnings) = LayerStore::open(&dir).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let stored = reopened.get(&id).unwrap();
        assert_eq!(stored.layer.digest(), layer.digest());
        assert_eq!(stored.layer.name, "地点A");

        // Tamper with the file: the store refuses it.
        let path = dir.join(format!("{id}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("01101", "99999");
        std::fs::write(&path, text).unwrap();
        let (tampered, warnings) = LayerStore::open(&dir).unwrap();
        assert!(tampered.get(&id).is_none());
        assert_eq!(warnings.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::execute::run_plan;
    use crate::import::{import_bytes, ImportOptions};
    use crate::planner::{plan_with_rules, PlannerContext};

    #[test]
    fn analysis_outputs_survive_a_restart() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        ))
        .unwrap();
        let (wards, _) = import_bytes(
            "wards.geojson",
            &bytes,
            &ImportOptions {
                name: Some("区".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let layers = BTreeMap::from([(wards.id(), wards)]);
        let planned = plan_with_rules("区の人口密度", &layers, &PlannerContext::default()).unwrap();
        let output = run_plan(&planned.plan, &layers).unwrap().output;
        let dir = std::env::temp_dir().join(format!("genegis-store-{}", uuid::Uuid::new_v4()));
        let (mut store, _) = LayerStore::open(&dir).unwrap();
        let id = store.insert(output.clone(), Value::Null).unwrap();
        drop(store);
        // Floats must survive JSON exactly (serde_json `float_roundtrip`),
        // otherwise the reloaded layer would fail digest verification.
        let (reopened, warnings) = LayerStore::open(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(reopened.get(&id).unwrap().layer.digest(), output.digest());
    }
}
