//! `/api/gis/*` — general-purpose GIS endpoints backed by `genegis-toolkit`.
//!
//! Every mutating endpoint (import, analysis, place resolution, CRS
//! assignment) runs through Command + Workflow Graph inside the toolkit and
//! returns its receipt; read endpoints (table, stats, pick, export) operate
//! on content-addressed layers whose digests are part of every response.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use genegis_toolkit::{
    execute::{run_plan, PlanStep, ToolkitPlan},
    export::{export, ExportFormat, MapOptions},
    geojson_io,
    import::{ImportFormat, ImportOptions},
    import_through_workflow, ops,
    place::{resolve_place, HttpFetcher, PlaceRequest},
    planner::{self, LlmConfig, PlannerContext, PlannerMode},
    table::{self, ClassifyRequest, TableQuery},
    LayerStore, ToolkitError,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// Shared GIS state.
pub struct GisState {
    store: Mutex<LayerStore>,
    llm: LlmConfig,
    samples_dir: PathBuf,
}

type Shared = Arc<GisState>;

/// Build the GIS router.
pub fn router(layer_dir: PathBuf, samples_dir: PathBuf) -> Router {
    let (store, warnings) = LayerStore::open(&layer_dir).unwrap_or_else(|error| {
        eprintln!(
            "Layer store {} unavailable ({error}); using memory",
            layer_dir.display()
        );
        (LayerStore::in_memory(), Vec::new())
    });
    for warning in warnings {
        eprintln!("Layer store warning: {warning}");
    }
    let state = Arc::new(GisState {
        store: Mutex::new(store),
        llm: LlmConfig::from_env(),
        samples_dir,
    });
    Router::new()
        .route("/api/gis/layers", get(list_layers))
        .route(
            "/api/gis/import",
            post(import_layer).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route("/api/gis/samples", post(load_samples))
        .route("/api/gis/layers/{id}", get(get_layer).delete(delete_layer))
        .route("/api/gis/layers/{id}/geojson", get(layer_geojson))
        .route("/api/gis/layers/{id}/table", get(layer_table))
        .route("/api/gis/layers/{id}/stats", get(layer_stats))
        .route("/api/gis/layers/{id}/classify", post(classify_layer))
        .route("/api/gis/layers/{id}/style", post(style_layer))
        .route("/api/gis/layers/{id}/rename", post(rename_layer))
        .route("/api/gis/layers/{id}/crs", post(assign_layer_crs))
        .route("/api/gis/layers/{id}/export", get(export_layer))
        .route("/api/gis/pick", post(pick))
        .route("/api/gis/operations", get(operations))
        .route("/api/gis/plan", post(plan_prompt))
        .route("/api/gis/run", post(run))
        .route("/api/gis/ask", post(ask))
        .route("/api/gis/place", post(place))
        .with_state(state)
}

fn ok(result: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({"ok": true, "error": null, "result": result})),
    )
        .into_response()
}

fn fail(error: ToolkitError) -> Response {
    let status = match &error {
        ToolkitError::UnknownReference(_) => StatusCode::NOT_FOUND,
        ToolkitError::CrsRequired(_) | ToolkitError::Unresolved(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        ToolkitError::Provider(_) => StatusCode::BAD_GATEWAY,
        ToolkitError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    let needs_crs = matches!(error, ToolkitError::CrsRequired(_));
    (
        status,
        Json(json!({
            "ok": false,
            "error": error.to_string(),
            "needs_crs": needs_crs,
            "crs_options": if needs_crs { crs_options() } else { Value::Null },
            "result": null,
        })),
    )
        .into_response()
}

fn crs_options() -> Value {
    let mut options = vec![
        json!({"id": "EPSG:4326", "name": "WGS 84（経緯度）"}),
        json!({"id": "EPSG:6668", "name": "JGD2011（経緯度）"}),
        json!({"id": "EPSG:3857", "name": "Web メルカトル"}),
    ];
    for epsg in 6669..=6687 {
        if let Ok(info) = genegis_toolkit::proj::lookup_epsg(epsg) {
            options.push(json!({"id": info.id, "name": info.name}));
        }
    }
    for epsg in [32651, 32652, 32653, 32654, 32655] {
        if let Ok(info) = genegis_toolkit::proj::lookup_epsg(epsg) {
            options.push(json!({"id": info.id, "name": info.name}));
        }
    }
    Value::Array(options)
}

fn with_store<T>(
    state: &Shared,
    f: impl FnOnce(&mut LayerStore) -> Result<T, ToolkitError>,
) -> Result<T, ToolkitError> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| ToolkitError::Command("layer store lock poisoned".into()))?;
    f(&mut store)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ToolkitError> + Send + 'static,
) -> Result<T, ToolkitError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ToolkitError::Command(format!("worker failed: {e}")))?
}

async fn list_layers(State(state): State<Shared>) -> Response {
    match with_store(&state, |store| Ok(store.list())) {
        Ok(list) => ok(json!({"layers": list, "llm_ready": state.llm.ready()})),
        Err(e) => fail(e),
    }
}

async fn get_layer(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    match with_store(&state, |store| {
        let stored = store
            .get(&id)
            .ok_or_else(|| ToolkitError::UnknownReference(id.clone()))?;
        Ok(
            json!({"layer": stored.layer.summary(), "receipt": stored.receipt, "style": stored.style}),
        )
    }) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

async fn delete_layer(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    match with_store(&state, |store| store.remove(&id)) {
        Ok(()) => ok(json!({"removed": id})),
        Err(e) => fail(e),
    }
}

#[derive(Debug, Deserialize)]
struct ImportQuery {
    filename: String,
    name: Option<String>,
    format: Option<ImportFormat>,
    crs: Option<String>,
    x_field: Option<String>,
    y_field: Option<String>,
    wkt_field: Option<String>,
    encoding: Option<String>,
    table: Option<String>,
    license: Option<String>,
    attribution: Option<String>,
}

fn blank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

async fn import_layer(
    State(state): State<Shared>,
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> Response {
    let options = ImportOptions {
        name: blank(query.name),
        format: query.format,
        crs: blank(query.crs),
        x_field: blank(query.x_field),
        y_field: blank(query.y_field),
        wkt_field: blank(query.wkt_field),
        encoding: blank(query.encoding),
        table: blank(query.table),
        license: blank(query.license),
        attribution: blank(query.attribution),
    };
    let filename = query.filename;
    let imported = blocking(move || import_through_workflow(&filename, &body, &options)).await;
    match imported.and_then(|(layer, receipt)| {
        let receipt_json =
            serde_json::to_value(&receipt).map_err(|e| ToolkitError::Command(e.to_string()))?;
        let id = with_store(&state, |store| {
            store.insert(layer, json!({"kind": "import", "import": receipt_json}))
        })?;
        Ok(json!({"id": id, "receipt": receipt}))
    }) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

async fn load_samples(State(state): State<Shared>) -> Response {
    let dir = state.samples_dir.clone();
    let result = blocking(move || genegis_toolkit::samples::load_nagoya_samples(&dir)).await;
    match result.and_then(|loaded| {
        with_store(&state, |store| {
            let mut ids = Vec::new();
            for (_, layer, receipt) in loaded {
                let receipt = serde_json::to_value(&receipt)
                    .map_err(|e| ToolkitError::Command(e.to_string()))?;
                ids.push(store.insert(layer, json!({"kind": "import", "import": receipt}))?);
            }
            Ok(ids)
        })
    }) {
        Ok(ids) => ok(json!({"ids": ids})),
        Err(e) => fail(e),
    }
}

async fn layer_geojson(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    let layer = match with_store(&state, |store| store.layer(&id).cloned()) {
        Ok(layer) => layer,
        Err(e) => return fail(e),
    };
    let result = blocking(move || {
        let wgs84 = genegis_toolkit::proj::lookup_epsg(4326)?;
        let mut display = layer.reprojected(&wgs84)?;
        for feature in &mut display.features {
            feature
                .properties
                .insert("__id".into(), Value::from(feature.id));
        }
        geojson_io::write(&display, true)
    })
    .await;
    match result {
        Ok(text) => ([(header::CONTENT_TYPE, "application/geo+json")], text).into_response(),
        Err(e) => fail(e),
    }
}

async fn layer_table(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(query): Query<TableQuery>,
) -> Response {
    match with_store(&state, |store| table::query(store.layer(&id)?, &query)) {
        Ok(page) => ok(serde_json::to_value(page).unwrap_or(Value::Null)),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct StatsQuery {
    field: String,
}

async fn layer_stats(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(query): Query<StatsQuery>,
) -> Response {
    match with_store(&state, |store| {
        table::field_stats(store.layer(&id)?, &query.field)
    }) {
        Ok(stats) => ok(serde_json::to_value(stats).unwrap_or(Value::Null)),
        Err(e) => fail(e),
    }
}

async fn classify_layer(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(request): Json<ClassifyRequest>,
) -> Response {
    match with_store(&state, |store| {
        let classification = table::classify(store.layer(&id)?, &request)?;
        let value = serde_json::to_value(&classification)
            .map_err(|e| ToolkitError::Command(e.to_string()))?;
        let mut style = store
            .get(&id)
            .map(|s| s.style.clone())
            .unwrap_or(Value::Null);
        if !style.is_object() {
            style = json!({});
        }
        style["classification"] = value.clone();
        store.set_style(&id, style)?;
        Ok(value)
    }) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

async fn style_layer(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(style): Json<Value>,
) -> Response {
    match with_store(&state, |store| store.set_style(&id, style)) {
        Ok(()) => ok(json!({"id": id})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct RenameRequest {
    name: String,
}

async fn rename_layer(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Response {
    match with_store(&state, |store| store.rename(&id, &body.name)) {
        Ok(()) => ok(json!({"id": id})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct CrsRequest {
    crs: String,
}

async fn assign_layer_crs(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<CrsRequest>,
) -> Response {
    let plan = ToolkitPlan {
        goal: format!("{id} の座標参照系を {} と指定", body.crs),
        steps: vec![PlanStep {
            id: "assign".into(),
            op: "assign_crs".into(),
            inputs: BTreeMap::from([("layer".to_string(), id.clone())]),
            params: json!({"crs": body.crs}),
        }],
        output: None,
        assumptions: vec!["利用者が座標参照系を確認して指定した".into()],
    };
    run_and_store(&state, plan, false, Some(id)).await
}

#[derive(Deserialize)]
struct ExportQuery {
    format: ExportFormat,
    title: Option<String>,
    subtitle: Option<String>,
}

async fn export_layer(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let found = with_store(&state, |store| {
        let stored = store
            .get(&id)
            .ok_or_else(|| ToolkitError::UnknownReference(id.clone()))?;
        Ok((stored.layer.clone(), stored.style.clone()))
    });
    let (layer, style) = match found {
        Ok(found) => found,
        Err(e) => return fail(e),
    };
    let options = MapOptions {
        title: blank(query.title),
        subtitle: blank(query.subtitle),
        classification: style
            .get("classification")
            .and_then(|c| serde_json::from_value(c.clone()).ok()),
        color: style
            .get("color")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    let format = query.format;
    match blocking(move || export(&layer, format, &options)).await {
        Ok(file) => {
            let receipt = serde_json::to_string(&file.receipt).unwrap_or_default();
            let disposition = format!(
                "attachment; filename=\"{}\"; filename*=UTF-8''{}",
                file.filename
                    .chars()
                    .map(|c| if c.is_ascii() { c } else { '_' })
                    .collect::<String>(),
                percent(&file.filename)
            );
            (
                [
                    (header::CONTENT_TYPE, file.media_type.to_string()),
                    (header::CONTENT_DISPOSITION, disposition),
                    (
                        header::HeaderName::from_static("x-genegis-export-receipt"),
                        percent(&receipt),
                    ),
                ],
                file.bytes,
            )
                .into_response()
        }
        Err(e) => fail(e),
    }
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct PickRequest {
    lon: f64,
    lat: f64,
    #[serde(default = "default_tolerance")]
    tolerance_m: f64,
    #[serde(default)]
    layers: Vec<String>,
}

fn default_tolerance() -> f64 {
    30.0
}

async fn pick(State(state): State<Shared>, Json(body): Json<PickRequest>) -> Response {
    let layers = match with_store(&state, |store| Ok(store.layers())) {
        Ok(layers) => layers,
        Err(e) => return fail(e),
    };
    let result = blocking(move || {
        let mut hits = Vec::new();
        for (id, layer) in &layers {
            if !body.layers.is_empty() && !body.layers.contains(id) {
                continue;
            }
            for hit in table::pick(layer, body.lon, body.lat, body.tolerance_m, 5)? {
                hits.push(json!({"layer_id": id, "layer_name": layer.name, "feature": hit, "units": layer.fields.iter().filter_map(|f| f.unit.as_ref().map(|u| (f.name.clone(), u.clone()))).collect::<BTreeMap<_, _>>()}));
            }
        }
        Ok(hits)
    })
    .await;
    match result {
        Ok(hits) => ok(json!({"hits": hits})),
        Err(e) => fail(e),
    }
}

async fn operations() -> Response {
    ok(serde_json::to_value(ops::catalog()).unwrap_or(Value::Null))
}

#[derive(Deserialize)]
struct PromptRequest {
    prompt: String,
    #[serde(default)]
    context: PlannerContext,
    #[serde(default)]
    mode: PlannerMode,
    #[serde(default)]
    keep_intermediate: bool,
}

async fn plan_prompt(State(state): State<Shared>, Json(body): Json<PromptRequest>) -> Response {
    let layers = match with_store(&state, |store| Ok(store.layers())) {
        Ok(layers) => layers,
        Err(e) => return fail(e),
    };
    let llm = state.llm.clone();
    match blocking(move || planner::plan(&body.prompt, &layers, &body.context, body.mode, &llm))
        .await
    {
        Ok(result) => ok(serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct RunRequest {
    plan: ToolkitPlan,
    #[serde(default)]
    keep_intermediate: bool,
}

async fn run(State(state): State<Shared>, Json(body): Json<RunRequest>) -> Response {
    run_and_store(&state, body.plan, body.keep_intermediate, None).await
}

async fn run_and_store(
    state: &Shared,
    plan: ToolkitPlan,
    keep_intermediate: bool,
    replaces: Option<String>,
) -> Response {
    let layers = match with_store(state, |store| Ok(store.layers())) {
        Ok(layers) => layers,
        Err(e) => return fail(e),
    };
    let plan_json = serde_json::to_value(&plan).unwrap_or(Value::Null);
    let outcome = blocking(move || run_plan(&plan, &layers)).await;
    match outcome.and_then(|run| store_run(state, run, plan_json, keep_intermediate, replaces)) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

fn store_run(
    state: &Shared,
    run: genegis_toolkit::ToolkitRun,
    plan: Value,
    keep_intermediate: bool,
    replaces: Option<String>,
) -> Result<Value, ToolkitError> {
    let receipt =
        serde_json::to_value(run.receipt()).map_err(|e| ToolkitError::Command(e.to_string()))?;
    let workflow =
        serde_json::to_value(&run.workflow).map_err(|e| ToolkitError::Command(e.to_string()))?;
    let output_step = run.steps.last().map(|s| s.id.clone()).unwrap_or_default();
    with_store(state, |store| {
        let mut name = run.workflow.goal.chars().take(40).collect::<String>();
        if name.trim().is_empty() {
            name = run.output.name.clone();
        }
        let mut output = run.output.clone();
        if let Some(previous) = replaces.as_ref().and_then(|id| store.get(id)) {
            output.name = previous.layer.name.clone();
        } else {
            output.name = name;
        }
        let output_id = store.insert(
            output,
            json!({"kind": "analysis", "run": receipt, "plan": plan}),
        )?;
        let mut intermediate = Vec::new();
        if keep_intermediate {
            for (step, layer) in &run.step_layers {
                if *step == output_step {
                    continue;
                }
                let mut layer = layer.clone();
                layer.name = format!(
                    "{} · {step}",
                    run.workflow.goal.chars().take(24).collect::<String>()
                );
                intermediate.push(store.insert(layer, json!({"kind": "intermediate", "step": step, "workflow_digest": run.workflow_digest}))?);
            }
        }
        if let Some(old) = replaces {
            if old != output_id {
                store.remove(&old)?;
            }
        }
        let table = table::query(
            store.layer(&output_id)?,
            &TableQuery {
                limit: Some(50),
                ..Default::default()
            },
        )?;
        Ok(json!({
            "output_id": output_id,
            "intermediate_ids": intermediate,
            "receipt": receipt,
            "workflow": workflow,
            "preview": table,
        }))
    })
}

async fn ask(State(state): State<Shared>, Json(body): Json<PromptRequest>) -> Response {
    let layers = match with_store(&state, |store| Ok(store.layers())) {
        Ok(layers) => layers,
        Err(e) => return fail(e),
    };
    let llm = state.llm.clone();
    let prompt = body.prompt.clone();
    let planned = blocking(move || {
        let planned = planner::plan(&prompt, &layers, &body.context, body.mode, &llm)?;
        let run = run_plan(&planned.plan, &layers)?;
        Ok((planned, run))
    })
    .await;
    match planned.and_then(|(planned, run)| {
        let plan_json = serde_json::to_value(&planned.plan).unwrap_or(Value::Null);
        let mut stored = store_run(&state, run, plan_json, body.keep_intermediate, None)?;
        stored["planner"] = serde_json::to_value(&planned).unwrap_or(Value::Null);
        Ok(stored)
    }) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

async fn place(State(state): State<Shared>, Json(request): Json<PlaceRequest>) -> Response {
    let resolved = blocking(move || resolve_place(&request, &HttpFetcher)).await;
    match resolved.and_then(|(layer, receipt)| {
        let receipt =
            serde_json::to_value(&receipt).map_err(|e| ToolkitError::Command(e.to_string()))?;
        let id = with_store(&state, |store| {
            store.insert(layer, json!({"kind": "place", "place": receipt}))
        })?;
        Ok(json!({"id": id, "receipt": receipt}))
    }) {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}
