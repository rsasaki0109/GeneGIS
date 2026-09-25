//! `genegis-mcp` — Model Context Protocol server for the GeneGIS toolkit.
//!
//! Speaks newline-delimited JSON-RPC 2.0 over stdio (MCP stdio transport).
//! The client model plans; GeneGIS validates, executes through Command +
//! Workflow Graph, and verifies. A plan that fails validation or any
//! independent check comes back as a tool error with the reason, so the
//! model can correct it — the model is never its own verifier.
//!
//! Layers persist in `GENEGIS_LAYER_DIR` (default `.genegis/layers`), the
//! same store the Workbench uses, so results appear in the データ分析 view.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use genegis_toolkit::execute::{run_plan, ToolkitPlan};
use genegis_toolkit::export::{export, ExportFormat, MapOptions};
use genegis_toolkit::import::ImportOptions;
use genegis_toolkit::place::{resolve_place, HttpFetcher, PlaceRequest};
use genegis_toolkit::planner::{plan_with_rules, PlannerContext};
use genegis_toolkit::samples::{load_nagoya_samples, samples_dir};
use genegis_toolkit::table::{self, ClassifyRequest, TableQuery};
use genegis_toolkit::{import_through_workflow, ops, LayerStore, ToolkitError};
use serde_json::{json, Value};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "GeneGIS verified spatial analysis. Workflow: \
1) list_layers (or import_layer / load_sample_data / resolve_place) to see data and field names; \
2) list_operations to see the operation catalog; \
3) build a plan {goal, steps:[{id, op, inputs:{role: layer_id or earlier step id}, params}]} and call run_plan. \
Distances need units (\"500 m\"). Population inside an area = spatial_join with area_weighted_sum. \
If run_plan returns an error, read it, fix the plan, and retry; never report an unverified number. \
Answer with the verified values, their units, and the workflow digest. \
If the loaded data cannot answer the question, say so instead of guessing.";

struct Server {
    store: LayerStore,
}

fn tools() -> Value {
    let plan_schema = json!({
        "type": "object",
        "required": ["goal", "steps"],
        "properties": {
            "goal": {"type": "string", "description": "the user's question"},
            "steps": {"type": "array", "items": {
                "type": "object", "required": ["id", "op"],
                "properties": {
                    "id": {"type": "string", "description": "[A-Za-z0-9_-]+, unique"},
                    "op": {"type": "string", "description": "operation name from list_operations"},
                    "inputs": {"type": "object", "additionalProperties": {"type": "string"}, "description": "role → layer id (lyr_…) or earlier step id"},
                    "params": {"type": "object"}
                }}},
            "output": {"type": "string", "description": "step id of the answer (default: last step)"},
            "assumptions": {"type": "array", "items": {"type": "string"}}
        }
    });
    json!([
        {"name": "list_layers", "description": "List loaded layers with IDs, geometry kind, CRS, feature count, fields (types and units), and provenance.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "list_operations", "description": "List the spatial operation catalog: names, input roles, and parameters.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "load_sample_data", "description": "Load the bundled Nagoya sample layers (wards with population, shelters, shops/facilities, flood zones, stations).",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "import_layer", "description": "Import a local GeoJSON, CSV/TSV, zipped Shapefile, GeoPackage, or GeoParquet file. If the CRS cannot be determined the error lists CRS options; retry with crs.",
         "inputSchema": {"type": "object", "required": ["path"], "properties": {
            "path": {"type": "string"}, "name": {"type": "string"}, "crs": {"type": "string", "description": "e.g. EPSG:6675"},
            "encoding": {"type": "string"}, "table": {"type": "string"}, "x_field": {"type": "string"}, "y_field": {"type": "string"},
            "license": {"type": "string"}, "attribution": {"type": "string"}}}},
        {"name": "describe_layer", "description": "Show a layer's schema, per-field statistics, and sample rows.",
         "inputSchema": {"type": "object", "required": ["layer_id"], "properties": {
            "layer_id": {"type": "string"}, "sample_rows": {"type": "integer", "default": 5}}}},
        {"name": "query_table", "description": "Filter/sort/page a layer's attributes. where uses SQL-like syntax: 人口 >= 100000 AND name LIKE '%中%'.",
         "inputSchema": {"type": "object", "required": ["layer_id"], "properties": {
            "layer_id": {"type": "string"}, "where": {"type": "string"}, "sort_by": {"type": "string"},
            "descending": {"type": "boolean"}, "offset": {"type": "integer"}, "limit": {"type": "integer"}}}},
        {"name": "suggest_plan", "description": "Ask GeneGIS's deterministic rule planner for a plan (not executed). Useful as a starting point; may decline.",
         "inputSchema": {"type": "object", "required": ["question"], "properties": {
            "question": {"type": "string"}, "point": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "[lon, lat] for 'this point'"}}}},
        {"name": "run_plan", "description": "Validate, execute through Command + Workflow Graph, and verify a plan. Returns step receipts with independent checks, the answer rows, and digests. Errors explain what to fix.",
         "inputSchema": {"type": "object", "required": ["plan"], "properties": {
            "plan": plan_schema, "keep_intermediate": {"type": "boolean", "default": false}}}},
        {"name": "resolve_place", "description": "Resolve a place name to a boundary (provider nominatim, OpenStreetMap) or a point (provider gsi, 国土地理院) and add it as a layer.",
         "inputSchema": {"type": "object", "required": ["query"], "properties": {
            "query": {"type": "string"}, "provider": {"type": "string", "enum": ["nominatim", "gsi"]}, "candidate": {"type": "integer"}}}},
        {"name": "export_layer", "description": "Write a layer to a file: geojson, csv, geopackage, geoparquet, or pdf (map with legend, scale, CRS, sources).",
         "inputSchema": {"type": "object", "required": ["layer_id", "format", "path"], "properties": {
            "layer_id": {"type": "string"}, "format": {"type": "string", "enum": ["geojson", "csv", "geopackage", "geoparquet", "pdf"]},
            "path": {"type": "string"}, "title": {"type": "string"}, "subtitle": {"type": "string"},
            "classify": {"type": "object", "description": "for pdf: {field, method: natural_breaks|quantile|equal_interval|categorical, classes}"}}}},
        {"name": "remove_layer", "description": "Remove a layer from the store.",
         "inputSchema": {"type": "object", "required": ["layer_id"], "properties": {"layer_id": {"type": "string"}}}}
    ])
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolkitError> {
    arg(args, key).ok_or_else(|| ToolkitError::parameter("tool", format!("missing argument {key}")))
}

fn layer_brief(summary: &genegis_toolkit::LayerSummary) -> Value {
    json!({
        "id": summary.id,
        "name": summary.name,
        "geometry": summary.geometry_kind,
        "crs": summary.crs,
        "crs_status": summary.crs_status,
        "features": summary.feature_count,
        "fields": summary.fields.iter().map(|f| json!({"name": f.name, "type": f.field_type, "unit": f.unit})).collect::<Vec<_>>(),
        "source": summary.provenance.source_uri,
        "attribution": summary.provenance.attribution,
        "derived_from_analysis": summary.provenance.format == "derived",
    })
}

impl Server {
    fn call(&mut self, name: &str, args: &Value) -> Result<Value, ToolkitError> {
        match name {
            "list_layers" => Ok(
                json!({"layers": self.store.list().iter().map(|s| layer_brief(&s.summary)).collect::<Vec<_>>()}),
            ),
            "list_operations" => serde_json::to_value(ops::catalog())
                .map_err(|e| ToolkitError::Command(e.to_string())),
            "load_sample_data" => {
                let mut ids = Vec::new();
                for (_, layer, receipt) in load_nagoya_samples(&samples_dir())? {
                    ids.push(
                        self.store
                            .insert(layer, json!({"kind": "import", "import": receipt}))?,
                    );
                }
                self.call("list_layers", &json!({})).map(|mut v| {
                    v["loaded"] = json!(ids);
                    v
                })
            }
            "import_layer" => {
                let path = PathBuf::from(required(args, "path")?);
                let bytes = std::fs::read(&path)?;
                let options = ImportOptions {
                    name: arg(args, "name").map(str::to_string),
                    crs: arg(args, "crs").map(str::to_string),
                    encoding: arg(args, "encoding").map(str::to_string),
                    table: arg(args, "table").map(str::to_string),
                    x_field: arg(args, "x_field").map(str::to_string),
                    y_field: arg(args, "y_field").map(str::to_string),
                    license: arg(args, "license").map(str::to_string),
                    attribution: arg(args, "attribution").map(str::to_string),
                    ..Default::default()
                };
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("upload")
                    .to_string();
                let (layer, receipt) = import_through_workflow(&filename, &bytes, &options)?;
                let summary = layer.summary();
                let receipt_json = serde_json::to_value(&receipt)
                    .map_err(|e| ToolkitError::Command(e.to_string()))?;
                self.store
                    .insert(layer, json!({"kind": "import", "import": receipt_json}))?;
                Ok(
                    json!({"layer": layer_brief(&summary), "report": receipt.report, "workflow_digest": receipt.workflow_digest}),
                )
            }
            "describe_layer" => {
                let layer = self.store.layer(required(args, "layer_id")?)?;
                let limit = args.get("sample_rows").and_then(Value::as_u64).unwrap_or(5) as usize;
                let stats: Vec<Value> = layer
                    .fields
                    .iter()
                    .filter_map(|f| table::field_stats(layer, &f.name).ok())
                    .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
                    .collect();
                let rows = table::query(
                    layer,
                    &TableQuery {
                        limit: Some(limit),
                        ..Default::default()
                    },
                )?;
                Ok(
                    json!({"layer": layer_brief(&layer.summary()), "field_stats": stats, "sample_rows": rows.rows}),
                )
            }
            "query_table" => {
                let layer = self.store.layer(required(args, "layer_id")?)?;
                let query = TableQuery {
                    filter: arg(args, "where").map(str::to_string),
                    sort_by: arg(args, "sort_by").map(str::to_string),
                    descending: args
                        .get("descending")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    offset: args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                    limit: args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize)
                        .or(Some(50)),
                };
                serde_json::to_value(table::query(layer, &query)?)
                    .map_err(|e| ToolkitError::Command(e.to_string()))
            }
            "suggest_plan" => {
                let point = args
                    .get("point")
                    .and_then(Value::as_array)
                    .and_then(|p| Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?]));
                let context = PlannerContext {
                    point,
                    selected_layer: None,
                };
                let result =
                    plan_with_rules(required(args, "question")?, &self.store.layers(), &context)?;
                serde_json::to_value(result).map_err(|e| ToolkitError::Command(e.to_string()))
            }
            "run_plan" => {
                let plan: ToolkitPlan =
                    serde_json::from_value(args.get("plan").cloned().unwrap_or(Value::Null))
                        .map_err(|e| {
                            ToolkitError::Plan(format!("plan does not match the schema: {e}"))
                        })?;
                let keep = args
                    .get("keep_intermediate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let run = run_plan(&plan, &self.store.layers())?;
                let receipt = run.receipt();
                let receipt_json = serde_json::to_value(&receipt)
                    .map_err(|e| ToolkitError::Command(e.to_string()))?;
                let mut output = run.output.clone();
                output.name = plan.goal.chars().take(40).collect();
                let output_id = self.store.insert(
                    output,
                    json!({"kind": "analysis", "run": receipt_json, "plan": plan, "origin": "mcp"}),
                )?;
                let mut intermediate = Vec::new();
                if keep {
                    for (step, layer) in &run.step_layers {
                        if run.steps.last().is_some_and(|s| &s.id == step) {
                            continue;
                        }
                        intermediate.push(self.store.insert(
                            layer.clone(),
                            json!({"kind": "intermediate", "step": step}),
                        )?);
                    }
                }
                let answer = self.store.layer(&output_id)?;
                let rows = table::query(
                    answer,
                    &TableQuery {
                        limit: Some(20),
                        ..Default::default()
                    },
                )?;
                let units: serde_json::Map<String, Value> = answer
                    .fields
                    .iter()
                    .filter_map(|f| f.unit.as_ref().map(|u| (f.name.clone(), json!(u))))
                    .collect();
                Ok(json!({
                    "output_layer_id": output_id,
                    "intermediate_layer_ids": intermediate,
                    "feature_count": answer.features.len(),
                    "units": units,
                    "rows": rows.rows,
                    "steps": receipt.steps,
                    "workflow_digest": receipt.workflow_digest,
                    "result_digest": receipt.result_digest,
                    "command_id": receipt.command_id,
                    "attribution": answer.provenance.attribution,
                    "license": answer.provenance.license,
                }))
            }
            "resolve_place" => {
                let request = PlaceRequest {
                    query: required(args, "query")?.to_string(),
                    provider: serde_json::from_value(
                        args.get("provider").cloned().unwrap_or(json!("nominatim")),
                    )
                    .map_err(|e| ToolkitError::parameter("resolve_place", e.to_string()))?,
                    candidate: args.get("candidate").and_then(Value::as_u64).unwrap_or(0) as usize,
                };
                let (layer, receipt) = resolve_place(&request, &HttpFetcher)?;
                let brief = layer_brief(&layer.summary());
                let receipt_json = serde_json::to_value(&receipt)
                    .map_err(|e| ToolkitError::Command(e.to_string()))?;
                self.store
                    .insert(layer, json!({"kind": "place", "place": receipt_json}))?;
                Ok(
                    json!({"layer": brief, "candidates": receipt.candidates, "chosen": receipt.chosen}),
                )
            }
            "export_layer" => {
                let id = required(args, "layer_id")?;
                let layer = self.store.layer(id)?.clone();
                let format: ExportFormat = serde_json::from_value(json!(required(args, "format")?))
                    .map_err(|e| ToolkitError::parameter("export_layer", e.to_string()))?;
                let classification = match args.get("classify") {
                    Some(spec) if spec.is_object() => {
                        let request: ClassifyRequest = serde_json::from_value(spec.clone())
                            .map_err(|e| {
                                ToolkitError::parameter("export_layer", format!("classify: {e}"))
                            })?;
                        Some(table::classify(&layer, &request)?)
                    }
                    _ => None,
                };
                let options = MapOptions {
                    title: arg(args, "title").map(str::to_string),
                    subtitle: arg(args, "subtitle").map(str::to_string),
                    classification,
                    color: None,
                };
                let file = export(&layer, format, &options)?;
                let path = PathBuf::from(required(args, "path")?);
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &file.bytes)?;
                Ok(json!({"path": path, "bytes": file.bytes.len(), "receipt": file.receipt}))
            }
            "remove_layer" => {
                let id = required(args, "layer_id")?;
                self.store.remove(id)?;
                Ok(json!({"removed": id}))
            }
            other => Err(ToolkitError::parameter(
                "tools/call",
                format!("unknown tool {other}"),
            )),
        }
    }

    fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(json!({}));
        // Notifications (no id) never get a response.
        let id = id?;
        let result = match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == requested)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "genegis", "title": "GeneGIS verified spatial analysis", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let outcome = self.call(name, &args);
                Ok(match outcome {
                    Ok(value) => json!({
                        "content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
                        "structuredContent": if value.is_object() { value } else { json!({"result": value}) },
                        "isError": false,
                    }),
                    Err(error) => json!({
                        "content": [{"type": "text", "text": format!("GeneGIS rejected this call: {error}")}],
                        "isError": true,
                    }),
                })
            }
            _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        })
    }
}

fn main() {
    let dir = std::env::var("GENEGIS_LAYER_DIR").unwrap_or_else(|_| ".genegis/layers".into());
    let store = match LayerStore::open(&dir) {
        Ok((store, warnings)) => {
            for warning in warnings {
                eprintln!("genegis-mcp: layer store warning: {warning}");
            }
            store
        }
        Err(error) => {
            eprintln!("genegis-mcp: layer store {dir} unavailable ({error}); using memory");
            LayerStore::in_memory()
        }
    };
    let mut server = Server { store };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(&message),
            Err(error) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": error.to_string()}}),
            ),
        };
        if let Some(response) = response {
            if writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
}
