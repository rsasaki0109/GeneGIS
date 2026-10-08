//! GeneGIS in the browser.
//!
//! The same toolkit the CLI, the Workbench and the MCP server use, compiled
//! without its native-only features: files are imported through Command +
//! Workflow Graph, questions are planned by the rule planner, plans run with
//! every per-step check, and a failed check returns its reason instead of an
//! answer. Nothing leaves the page — there is no server.
//!
//! [`Session`] is a plain Rust API exchanging JSON strings so it can be tested
//! natively; on `wasm32` the same methods are exported through `wasm-bindgen`.

#![deny(missing_docs)]

use genegis_toolkit::execute::ToolkitPlan;
use genegis_toolkit::planner::plan_with_rules;
use genegis_toolkit::samples::{load_nagoya_samples_with, SAMPLE_FILES};
use genegis_toolkit::table::{self, TableQuery};
use genegis_toolkit::{
    export, import_through_workflow, run_plan, ExportFormat, ImportOptions, LayerStore, MapOptions,
    PlannerContext, ToolkitError,
};
use serde_json::{json, Value};

/// Error message returned to the page.
pub type SessionResult<T> = Result<T, String>;

fn err(error: ToolkitError) -> String {
    error.to_string()
}

/// Bundled Nagoya samples, embedded so the page works offline.
fn sample_bytes(file: &str) -> genegis_toolkit::Result<Vec<u8>> {
    let bytes: &[u8] = match file {
        "nagoya-wards.geojson" => {
            include_bytes!("../../../examples/nagoya-population-density/data/nagoya-wards.geojson")
        }
        "nagoya-shelters.geojson" => include_bytes!(
            "../../../examples/nagoya-population-density/data/nagoya-shelters.geojson"
        ),
        "nagoya-pois.geojson" => {
            include_bytes!("../../../examples/nagoya-population-density/data/nagoya-pois.geojson")
        }
        "nagoya-flood-zones.geojson" => include_bytes!(
            "../../../examples/nagoya-population-density/data/nagoya-flood-zones.geojson"
        ),
        other => return Err(ToolkitError::UnknownReference(other.into())),
    };
    Ok(bytes.to_vec())
}

/// An in-memory analysis session (one per page).
#[derive(Default)]
pub struct Session {
    store: LayerStore,
}

impl Session {
    /// Empty session.
    pub fn new() -> Self {
        Self {
            store: LayerStore::in_memory(),
        }
    }

    /// Toolkit version and the sample files bundled into this build.
    pub fn about(&self) -> String {
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "samples": SAMPLE_FILES.iter().map(|s| s.1).collect::<Vec<_>>(),
            "native_features": genegis_toolkit::NATIVE,
        })
        .to_string()
    }

    /// Load the bundled Nagoya sample layers; returns the layer list.
    pub fn load_samples(&mut self) -> SessionResult<String> {
        for (_, layer, receipt) in load_nagoya_samples_with(sample_bytes).map_err(err)? {
            let receipt = serde_json::to_value(&receipt).map_err(|e| e.to_string())?;
            self.store
                .insert(layer, json!({"kind": "import", "import": receipt}))
                .map_err(err)?;
        }
        Ok(self.layers())
    }

    /// Import a dropped file (GeoJSON, CSV/TSV, zipped Shapefile).
    /// `options_json` is an [`ImportOptions`] object (`{}` for defaults).
    pub fn import(
        &mut self,
        filename: &str,
        bytes: &[u8],
        options_json: &str,
    ) -> SessionResult<String> {
        let options: ImportOptions = if options_json.trim().is_empty() {
            ImportOptions::default()
        } else {
            serde_json::from_str(options_json).map_err(|e| format!("import options: {e}"))?
        };
        let (layer, receipt) = import_through_workflow(filename, bytes, &options).map_err(err)?;
        let summary = layer.summary();
        let receipt = serde_json::to_value(&receipt).map_err(|e| e.to_string())?;
        let id = self
            .store
            .insert(layer, json!({"kind": "import", "import": receipt.clone()}))
            .map_err(err)?;
        Ok(json!({"id": id, "layer": summary, "receipt": receipt}).to_string())
    }

    /// Layer list (summaries with receipts).
    pub fn layers(&self) -> String {
        serde_json::to_string(&self.store.list()).unwrap_or_else(|_| "[]".into())
    }

    /// Plan a question with the rule planner (no network, no LLM).
    /// `context_json` may carry `{"point": [lon, lat], "selected_layer": id}`.
    pub fn suggest(&self, question: &str, context_json: &str) -> SessionResult<String> {
        let context = parse_context(context_json)?;
        let result = plan_with_rules(question, &self.store.layers(), &context).map_err(err)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    /// Validate and run a plan with every per-step check. The answer layer is
    /// added to the session; a failed check returns its reason instead.
    pub fn run(&mut self, plan_json: &str) -> SessionResult<String> {
        let plan: ToolkitPlan = serde_json::from_str(plan_json)
            .map_err(|e| format!("plan does not match the schema: {e}"))?;
        let run = run_plan(&plan, &self.store.layers()).map_err(err)?;
        let receipt = run.receipt();
        let receipt_json = serde_json::to_value(&receipt).map_err(|e| e.to_string())?;
        let mut output = run.output.clone();
        output.name = plan.goal.chars().take(40).collect();
        let output_id = self
            .store
            .insert(
                output,
                json!({"kind": "analysis", "run": receipt_json.clone(), "plan": plan, "origin": "browser"}),
            )
            .map_err(err)?;
        // An aggregate answer (e.g. a count) has no geometry; show the last
        // step layer that does, so the map highlights what was counted.
        let display = if run.output.features.iter().any(|f| f.geometry.is_some()) {
            None
        } else {
            run.steps
                .iter()
                .rev()
                .filter_map(|step| run.step_layers.get(&step.id))
                .find(|layer| layer.features.iter().any(|f| f.geometry.is_some()))
                .map(|layer| export(layer, ExportFormat::Geojson, &MapOptions::default()))
                .transpose()
                .map_err(err)?
                .map(|file| String::from_utf8_lossy(&file.bytes).into_owned())
        };
        let answer = self.store.layer(&output_id).map_err(err)?;
        let rows = table::query(
            answer,
            &TableQuery {
                limit: Some(50),
                ..Default::default()
            },
        )
        .map_err(err)?;
        Ok(json!({
            "output_id": output_id,
            "layer": answer.summary(),
            "receipt": receipt_json,
            "rows": rows,
            "display_geojson": display,
        })
        .to_string())
    }

    /// Plan with the rule planner, then run: one call for "ask a question".
    pub fn ask(&mut self, question: &str, context_json: &str) -> SessionResult<String> {
        let planned: Value = serde_json::from_str(&self.suggest(question, context_json)?)
            .map_err(|e| e.to_string())?;
        let plan = planned.get("plan").cloned().unwrap_or(Value::Null);
        let ran: Value =
            serde_json::from_str(&self.run(&plan.to_string())?).map_err(|e| e.to_string())?;
        Ok(json!({"planned": planned, "result": ran}).to_string())
    }

    /// Layer as RFC 7946 GeoJSON (EPSG:4326) for the map.
    pub fn geojson(&self, layer_id: &str) -> SessionResult<String> {
        let file = self.export_file(layer_id, "geojson")?;
        String::from_utf8(file.bytes).map_err(|e| e.to_string())
    }

    /// Export bytes in `geojson` or `csv` (formats needing native libraries
    /// fail with a reason).
    pub fn export(&self, layer_id: &str, format: &str) -> SessionResult<Vec<u8>> {
        Ok(self.export_file(layer_id, format)?.bytes)
    }

    /// Suggested filename for an export.
    pub fn export_name(&self, layer_id: &str, format: &str) -> SessionResult<String> {
        Ok(self.export_file(layer_id, format)?.filename)
    }

    /// Evidence for a layer: its receipt (import or run, with plan, checks
    /// and digests) as JSON.
    pub fn evidence(&self, layer_id: &str) -> SessionResult<String> {
        let stored = self
            .store
            .get(layer_id)
            .ok_or_else(|| format!("unknown layer {layer_id}"))?;
        Ok(json!({
            "layer": stored.layer.summary(),
            "receipt": stored.receipt,
            "generated_by": format!("genegis-wasm {}", env!("CARGO_PKG_VERSION")),
        })
        .to_string())
    }

    /// Remove a layer.
    pub fn remove(&mut self, layer_id: &str) -> SessionResult<()> {
        self.store.remove(layer_id).map_err(err)
    }

    fn export_file(
        &self,
        layer_id: &str,
        format: &str,
    ) -> SessionResult<genegis_toolkit::ExportFile> {
        let format: ExportFormat = serde_json::from_value(Value::String(format.into()))
            .map_err(|_| format!("unknown export format {format}"))?;
        let layer = self.store.layer(layer_id).map_err(err)?;
        export(layer, format, &MapOptions::default()).map_err(err)
    }
}

fn parse_context(context_json: &str) -> SessionResult<PlannerContext> {
    if context_json.trim().is_empty() {
        return Ok(PlannerContext::default());
    }
    let value: Value =
        serde_json::from_str(context_json).map_err(|e| format!("planner context: {e}"))?;
    let point = value
        .get("point")
        .and_then(Value::as_array)
        .and_then(|p| Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?]));
    Ok(PlannerContext {
        point,
        selected_layer: value
            .get("selected_layer")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    //! `wasm-bindgen` exports: the same methods, errors as JS exceptions.

    use wasm_bindgen::prelude::*;

    /// Browser session.
    #[wasm_bindgen]
    pub struct GeneGis {
        inner: super::Session,
    }

    #[wasm_bindgen]
    impl GeneGis {
        /// New empty session.
        #[wasm_bindgen(constructor)]
        pub fn new() -> GeneGis {
            GeneGis {
                inner: super::Session::new(),
            }
        }

        /// Version and bundled samples.
        pub fn about(&self) -> String {
            self.inner.about()
        }

        /// Load the bundled Nagoya samples.
        #[wasm_bindgen(js_name = loadSamples)]
        pub fn load_samples(&mut self) -> Result<String, JsError> {
            self.inner.load_samples().map_err(|e| JsError::new(&e))
        }

        /// Import a file.
        pub fn import(
            &mut self,
            filename: &str,
            bytes: &[u8],
            options_json: &str,
        ) -> Result<String, JsError> {
            self.inner
                .import(filename, bytes, options_json)
                .map_err(|e| JsError::new(&e))
        }

        /// Layer list.
        pub fn layers(&self) -> String {
            self.inner.layers()
        }

        /// Plan a question.
        pub fn suggest(&self, question: &str, context_json: &str) -> Result<String, JsError> {
            self.inner
                .suggest(question, context_json)
                .map_err(|e| JsError::new(&e))
        }

        /// Run a plan.
        pub fn run(&mut self, plan_json: &str) -> Result<String, JsError> {
            self.inner.run(plan_json).map_err(|e| JsError::new(&e))
        }

        /// Plan and run.
        pub fn ask(&mut self, question: &str, context_json: &str) -> Result<String, JsError> {
            self.inner
                .ask(question, context_json)
                .map_err(|e| JsError::new(&e))
        }

        /// GeoJSON for the map.
        pub fn geojson(&self, layer_id: &str) -> Result<String, JsError> {
            self.inner.geojson(layer_id).map_err(|e| JsError::new(&e))
        }

        /// Export bytes.
        pub fn export(&self, layer_id: &str, format: &str) -> Result<Vec<u8>, JsError> {
            self.inner
                .export(layer_id, format)
                .map_err(|e| JsError::new(&e))
        }

        /// Export filename.
        #[wasm_bindgen(js_name = exportName)]
        pub fn export_name(&self, layer_id: &str, format: &str) -> Result<String, JsError> {
            self.inner
                .export_name(layer_id, format)
                .map_err(|e| JsError::new(&e))
        }

        /// Evidence JSON.
        pub fn evidence(&self, layer_id: &str) -> Result<String, JsError> {
            self.inner.evidence(layer_id).map_err(|e| JsError::new(&e))
        }

        /// Remove a layer.
        pub fn remove(&mut self, layer_id: &str) -> Result<(), JsError> {
            self.inner.remove(layer_id).map_err(|e| JsError::new(&e))
        }
    }

    impl Default for GeneGis {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Session {
        let mut session = Session::new();
        session.load_samples().expect("samples");
        session
    }

    #[test]
    fn loads_the_bundled_samples() {
        let session = loaded();
        let layers: Value = serde_json::from_str(&session.layers()).unwrap();
        assert_eq!(layers.as_array().unwrap().len(), 5);
    }

    #[test]
    fn asks_a_question_end_to_end_with_checks_and_digests() {
        let mut session = loaded();
        let answer: Value =
            serde_json::from_str(&session.ask("区の人口密度", "").expect("ask")).unwrap();
        let receipt = &answer["result"]["receipt"];
        assert!(receipt["workflow_digest"].as_str().is_some());
        assert!(receipt["result_digest"].as_str().is_some());
        let steps = receipt["steps"].as_array().unwrap();
        assert!(!steps.is_empty());
        let output_id = answer["result"]["output_id"].as_str().unwrap();
        let geojson: Value = serde_json::from_str(&session.geojson(output_id).unwrap()).unwrap();
        assert_eq!(geojson["features"].as_array().unwrap().len(), 16);
        assert!(session
            .evidence(output_id)
            .unwrap()
            .contains("result_digest"));
        assert!(answer["result"]["display_geojson"].is_null());
    }

    #[test]
    fn aggregate_answers_carry_the_counted_features_for_display() {
        let mut session = loaded();
        let answer: Value = serde_json::from_str(
            &session
                .ask("駅から500m以内の避難所を数えて", "")
                .expect("ask"),
        )
        .unwrap();
        let display: Value =
            serde_json::from_str(answer["result"]["display_geojson"].as_str().unwrap()).unwrap();
        let count = answer["result"]["rows"]["rows"][0]["properties"]["count"].as_u64();
        assert_eq!(
            Some(display["features"].as_array().unwrap().len() as u64),
            count
        );
    }

    #[test]
    fn imports_a_dropped_geojson_and_exports_csv() {
        let mut session = Session::new();
        let file = br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"a"},"geometry":{"type":"Point","coordinates":[136.9,35.17]}}]}"#;
        let imported: Value =
            serde_json::from_str(&session.import("points.geojson", file, "{}").unwrap()).unwrap();
        let id = imported["id"].as_str().unwrap();
        let csv = session.export(id, "csv").unwrap();
        assert!(String::from_utf8_lossy(&csv).contains("POINT"));
        assert!(session.export_name(id, "csv").unwrap().ends_with(".csv"));
    }

    #[test]
    fn invalid_plan_returns_a_reason_instead_of_an_answer() {
        let mut session = loaded();
        let plan = r#"{"goal":"bad","steps":[{"id":"b","op":"buffer","inputs":{"input":"lyr_missing"},"params":{"distance":"500 m"}}]}"#;
        let error = session.run(plan).unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn native_only_formats_fail_with_a_reason() {
        let session = loaded();
        let layers: Value = serde_json::from_str(&session.layers()).unwrap();
        let id = layers[0]["id"].as_str().unwrap();
        let result = session.export(id, "geopackage");
        if genegis_toolkit::NATIVE {
            // Workspace builds unify features; the browser build never does.
            assert!(result.is_ok());
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("not available in this build"), "{error}");
        }
    }
}
