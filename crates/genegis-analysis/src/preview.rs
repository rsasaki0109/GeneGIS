use genegis_catalog::{
    alpha_catalog, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_GEOPARQUET_ID, REMOTE_COG_DEMO_ID,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowExecution,
    WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent, WorkflowExecutor,
};
use genegis_crs::{CoordinateUnit, Crs, SourceSnapshot};
use genegis_geometry::PolygonRing;
use genegis_render::{
    run_choropleth_window, run_scene3d_window, BuildingLod1, ChoroplethMap, OrbitCamera, Scene3d,
    ScenePoi, SceneSource,
};
use genegis_storage::{GpuFrameMetrics, IoReceipt};
use genegis_style::ColorRgba;
use genegis_workflow::{scene3d_copc_lod1_template, GeoWorkflow};
use serde::{Deserialize, Serialize};

use crate::error::AnalysisError;
use crate::live_dashboard::{
    build_scene3d_dashboard, canonical_scene_result_digest, LiveDashboard,
};
use crate::nagoya::run_nagoya_population_density_from_catalog;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lod1BuildingDataset {
    schema_version: String,
    crs: Crs,
    vertical_unit: String,
    buildings: Vec<BuildingLod1>,
    #[serde(default)]
    pois: Vec<ScenePoi>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scene3dLaunchReceipt {
    pub command_id: String,
    pub workflow_digest: String,
    pub result_digest: String,
    pub point_count: usize,
    pub building_count: usize,
    pub poi_count: usize,
    pub dashboard: LiveDashboard,
}

pub fn attach_scene3d_gpu_evidence(
    mut receipt: IoReceipt,
    benchmark: &genegis_render::Scene3dBenchmark,
) -> IoReceipt {
    receipt.gpu = Some(GpuFrameMetrics {
        adapter: benchmark.adapter.clone(),
        backend: benchmark.backend.clone(),
        upload_bytes: benchmark.upload_bytes,
        upload_ns: benchmark.upload_ns,
        first_frame_ns: benchmark.first_frame_ns,
        steady_state_fps: benchmark.steady_state_fps,
    });
    receipt
}

struct Scene3dPreviewExecutor {
    scene: Scene3d,
}

impl WorkflowExecutor for Scene3dPreviewExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        self.scene
            .validate()
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = canonical_scene_result_digest(&self.scene)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let scene = self.scene.clone();
        std::thread::Builder::new()
            .name("genegis-scene3d-preview".into())
            .spawn(move || {
                if let Err(error) = run_scene3d_window(scene) {
                    eprintln!("3D GPU preview failed: {error}");
                }
            })
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({
                "kind": "scene3d_webgpu",
                "point_count": self.scene.points.len(),
                "building_count": self.scene.buildings.len(),
                "poi_count": self.scene.pois.len(),
                "crs": self.scene.crs.identifier(),
                "coordinate_unit": self.scene.coordinate_unit,
                "vertical_unit": self.scene.vertical_unit,
            }),
            evidence: serde_json::json!({
                "command_id": context.command_id,
                "workflow_digest": context.workflow_digest,
                "sources": self.scene.sources,
            }),
            events: self
                .scene
                .sources
                .iter()
                .map(|source| WorkflowExecutionEvent {
                    kind: "source_read".into(),
                    source_uri: Some(source.snapshot.uri.clone()),
                    observed_at: context.command_timestamp,
                    details: serde_json::json!({"role": source.role}),
                })
                .collect(),
        })
    }
}

pub fn launch_scene3d_preview(
    copc_path: &str,
    buildings_path: &str,
    crs: Crs,
) -> Result<Scene3dLaunchReceipt, AnalysisError> {
    if !copc_path.to_ascii_lowercase().ends_with(".copc.laz") {
        return Err(AnalysisError::Message(
            "3D scene point source must be a .copc.laz file".into(),
        ));
    }
    if crs.coordinate_unit() != CoordinateUnit::Metres {
        return Err(AnalysisError::Message(
            "3D scene CRS must use metre axes".into(),
        ));
    }
    let cloud = genegis_pointcloud::read_point_cloud_path(copc_path)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let building_bytes = std::fs::read(buildings_path)
        .map_err(|error| AnalysisError::Message(format!("read LOD1 buildings: {error}")))?;
    let building_data: Lod1BuildingDataset = serde_json::from_slice(&building_bytes)
        .map_err(|error| AnalysisError::Message(format!("parse LOD1 buildings: {error}")))?;
    if building_data.schema_version != "0.1.0"
        || building_data.crs != crs
        || building_data.vertical_unit != "metres"
    {
        return Err(AnalysisError::Message(
            "LOD1 building CRS, schema version, or vertical unit does not match the scene".into(),
        ));
    }
    let bounds = cloud
        .bounds()
        .ok_or_else(|| AnalysisError::Message("COPC point source is empty".into()))?;
    let target = [
        (bounds[0] + bounds[3]) * 0.5,
        (bounds[1] + bounds[4]) * 0.5,
        (bounds[2] + bounds[5]) * 0.5,
    ];
    let radius = (bounds[3] - bounds[0])
        .max(bounds[4] - bounds[1])
        .max(bounds[5] - bounds[2])
        .max(1.0)
        * 1.8;
    let copc_source = SourceSnapshot::new(path_source_uri(copc_path)?);
    let building_source = SourceSnapshot::new(path_source_uri(buildings_path)?);
    let scene = Scene3d {
        schema_version: "0.1.0".into(),
        crs: crs.clone(),
        coordinate_unit: CoordinateUnit::Metres,
        vertical_unit: "metres".into(),
        sources: vec![
            SceneSource {
                id: "copc".into(),
                role: "point_cloud".into(),
                snapshot: copc_source.clone(),
            },
            SceneSource {
                id: "lod1-buildings".into(),
                role: "measured_building_height".into(),
                snapshot: building_source.clone(),
            },
        ],
        point_source_id: "copc".into(),
        points: cloud.points,
        buildings: building_data
            .buildings
            .into_iter()
            .map(|mut building| {
                building.height_source_id = "lod1-buildings".into();
                building
            })
            .collect(),
        pois: building_data
            .pois
            .into_iter()
            .map(|mut poi| {
                poi.source_id = "lod1-buildings".into();
                poi
            })
            .collect(),
        camera: OrbitCamera {
            target,
            yaw_degrees: 30.0,
            pitch_degrees: 35.0,
            radius,
        },
    };
    scene
        .validate()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let workflow =
        scene3d_copc_lod1_template(copc_source.clone(), building_source.clone(), crs.clone());
    let workflow_digest = workflow
        .stable_digest()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(copc_source.clone())
    .with_source_snapshot(building_source.clone())
    .with_input_snapshot(InputSnapshot::new("copc", copc_source).with_crs(crs.clone()))
    .with_input_snapshot(InputSnapshot::new("buildings", building_source).with_crs(crs));
    let command_id = envelope.id;
    let point_count = scene.points.len();
    let building_count = scene.buildings.len();
    let poi_count = scene.pois.len();
    let executor = Scene3dPreviewExecutor { scene };
    let mut project = Project::new("3D preview");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let result = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let result_digest = result
        .result_digest
        .ok_or_else(|| AnalysisError::Message("3D workflow returned no digest".into()))?;
    let dashboard = build_scene3d_dashboard(
        &executor.scene,
        genegis_core::WorkflowDigest::new(workflow_digest.clone()),
        result_digest.clone(),
    )
    .map_err(|error| AnalysisError::Message(error.to_string()))?;
    dashboard
        .verify(&executor.scene)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(Scene3dLaunchReceipt {
        command_id: command_id.to_string(),
        workflow_digest,
        result_digest,
        point_count,
        building_count,
        poi_count,
        dashboard,
    })
}

fn path_source_uri(path: &str) -> Result<String, AnalysisError> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| AnalysisError::Message(format!("resolve source path: {error}")))?;
    Ok(format!(
        "file:///{}",
        path.to_string_lossy().replace('\\', "/")
    ))
}

/// Build the Nagoya choropleth map for GPU rendering.
pub fn nagoya_choropleth_map() -> Result<ChoroplethMap, AnalysisError> {
    let analysis = run_nagoya_population_density_from_catalog()?;
    let mut map = ChoroplethMap::default();
    for feature in &analysis.features {
        map.push_feature(feature.rings.clone(), feature.color);
    }
    Ok(map)
}

/// Build a raster preview map from a COG pixel window (Phase 8 beta).
pub fn cog_raster_preview_map(uri: &str) -> Result<ChoroplethMap, AnalysisError> {
    let cols = 32u32;
    let rows = 32u32;
    let pixels = genegis_raster::read_cog_window_uri(uri, 0, 0, rows, cols)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;

    let mut map = ChoroplethMap::default();
    let cell_w = 1.0 / cols as f64;
    let cell_h = 1.0 / rows as f64;

    for row in 0..rows {
        for col in 0..cols {
            let value = pixels[(row * cols + col) as usize];
            let g = value as f32 / 255.0;
            let color = ColorRgba::new(g, g * 0.85, g * 0.7, 1.0);
            let x0 = col as f64 * cell_w;
            let y0 = row as f64 * cell_h;
            let ring = PolygonRing::new(vec![
                (x0, y0),
                (x0 + cell_w, y0),
                (x0 + cell_w, y0 + cell_h),
                (x0, y0 + cell_h),
                (x0, y0),
            ]);
            map.push_feature(vec![ring], color);
        }
    }

    Ok(map)
}

/// Launch the native WebGPU choropleth preview on a background thread.
pub fn spawn_nagoya_gpu_preview() -> Result<(), AnalysisError> {
    spawn_gpu_map(nagoya_choropleth_map())
}

/// Launch a WebGPU raster preview for a COG URI.
pub fn spawn_cog_gpu_preview(uri: &str) -> Result<(), AnalysisError> {
    let uri = uri.to_string();
    spawn_gpu_map(cog_raster_preview_map(&uri))
}

/// Launch workflow-aware GPU preview (Nagoya choropleth or COG raster grid).
pub fn spawn_gpu_preview_for_workflow(workflow_id: &str) -> Result<String, AnalysisError> {
    match workflow_id {
        "nagoya-density" => {
            spawn_nagoya_gpu_preview()?;
            Ok("WebGPU choropleth preview launched".into())
        }
        "remote-cog-demo" => {
            let uri = alpha_catalog()
                .require(REMOTE_COG_DEMO_ID)
                .map_err(|err| AnalysisError::Message(err.to_string()))?
                .uri
                .clone();
            spawn_cog_gpu_preview(&uri)?;
            Ok("WebGPU raster preview launched (remote COG window)".into())
        }
        "local-cog-demo" => {
            let uri = alpha_catalog()
                .require(LOCAL_COG_DEMO_ID)
                .map_err(|err| AnalysisError::Message(err.to_string()))?
                .uri
                .clone();
            spawn_cog_gpu_preview(&uri)?;
            Ok("WebGPU raster preview launched (local COG window)".into())
        }
        "nagoya-geoparquet-density" => {
            let analysis = crate::nagoya::run_nagoya_population_density_for_dataset(
                NAGOYA_WARDS_GEOPARQUET_ID,
            )?;
            let mut map = ChoroplethMap::default();
            for feature in &analysis.features {
                map.push_feature(feature.rings.clone(), feature.color);
            }
            spawn_gpu_map(Ok(map))?;
            Ok("WebGPU choropleth preview launched (GeoParquet density)".into())
        }
        other => Err(AnalysisError::Message(format!(
            "GPU preview not supported for workflow {other}"
        ))),
    }
}

fn spawn_gpu_map(build: Result<ChoroplethMap, AnalysisError>) -> Result<(), AnalysisError> {
    std::thread::Builder::new()
        .name("genegis-gpu-preview".into())
        .spawn(move || match build {
            Ok(map) => run_choropleth_window(map),
            Err(err) => eprintln!("GPU preview failed: {err}"),
        })
        .map_err(|err| AnalysisError::Message(format!("failed to spawn GPU preview: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nagoya_choropleth_map_has_sixteen_wards() {
        let map = nagoya_choropleth_map().expect("map");
        assert_eq!(map.features.len(), 16);
    }

    #[test]
    fn local_cog_preview_map_has_grid_cells() {
        let uri = alpha_catalog()
            .require(LOCAL_COG_DEMO_ID)
            .expect("record")
            .uri
            .clone();
        let map = cog_raster_preview_map(&uri).expect("map");
        assert_eq!(map.features.len(), 32 * 32);
    }
}
