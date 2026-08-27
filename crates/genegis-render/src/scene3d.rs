//! Contract-first WebGPU scene for COPC points and LOD1 buildings.

use std::{
    collections::BTreeSet,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use bytemuck::{Pod, Zeroable};
use genegis_crs::{CoordinateUnit, Crs, CrsKind, SourceSnapshot};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

use crate::RenderCanvas;

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.025,
    g: 0.043,
    b: 0.065,
    a: 1.0,
};
const GPU_COMPLETION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSource {
    pub id: String,
    pub role: String,
    pub snapshot: SourceSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingLod1 {
    pub id: String,
    pub footprint: Vec<[f64; 2]>,
    pub base_z: f64,
    pub height: f64,
    #[serde(default)]
    pub height_source_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePoi {
    pub id: String,
    pub position: [f64; 3],
    pub category: String,
    #[serde(default)]
    pub source_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitCamera {
    pub target: [f64; 3],
    pub yaw_degrees: f32,
    pub pitch_degrees: f32,
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene3d {
    pub schema_version: String,
    pub crs: Crs,
    pub coordinate_unit: CoordinateUnit,
    pub vertical_unit: String,
    pub sources: Vec<SceneSource>,
    pub point_source_id: String,
    pub points: Vec<[f64; 3]>,
    pub buildings: Vec<BuildingLod1>,
    #[serde(default)]
    pub pois: Vec<ScenePoi>,
    pub camera: OrbitCamera,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Scene3dError {
    #[error("3D scene requires schema version 0.1.0")]
    SchemaVersion,
    #[error("3D scene requires a known projected CRS with metre axes")]
    ProjectedMetreCrs,
    #[error("3D scene vertical unit must be metres")]
    VerticalUnit,
    #[error("3D scene source IDs must be non-empty and unique")]
    Sources,
    #[error("3D scene source reference is unresolved: {0}")]
    SourceReference(String),
    #[error("3D scene requires at least one finite COPC point")]
    Points,
    #[error("invalid LOD1 building {id}: {reason}")]
    Building { id: String, reason: String },
    #[error("invalid POI {id}: {reason}")]
    Poi { id: String, reason: String },
    #[error("invalid orbit camera")]
    Camera,
    #[error("WebGPU scene failed: {0}")]
    Gpu(String),
}

impl Scene3d {
    pub fn validate(&self) -> Result<(), Scene3dError> {
        if self.schema_version != "0.1.0" {
            return Err(Scene3dError::SchemaVersion);
        }
        if self.crs.kind() != CrsKind::Projected
            || self.crs.coordinate_unit() != CoordinateUnit::Metres
            || self.coordinate_unit != CoordinateUnit::Metres
        {
            return Err(Scene3dError::ProjectedMetreCrs);
        }
        if self.vertical_unit != "metres" {
            return Err(Scene3dError::VerticalUnit);
        }
        let mut source_ids = BTreeSet::new();
        for source in &self.sources {
            if source.id.trim().is_empty()
                || source.role.trim().is_empty()
                || source.snapshot.uri.trim().is_empty()
                || !source_ids.insert(source.id.as_str())
            {
                return Err(Scene3dError::Sources);
            }
        }
        if !source_ids.contains(self.point_source_id.as_str()) {
            return Err(Scene3dError::SourceReference(self.point_source_id.clone()));
        }
        if self.points.is_empty() || !self.points.iter().flatten().all(|value| value.is_finite()) {
            return Err(Scene3dError::Points);
        }
        for building in &self.buildings {
            if !source_ids.contains(building.height_source_id.as_str()) {
                return Err(Scene3dError::SourceReference(
                    building.height_source_id.clone(),
                ));
            }
            if building.id.trim().is_empty()
                || building.footprint.len() < 3
                || !building.height.is_finite()
                || building.height <= 0.0
                || !building.base_z.is_finite()
                || !building.footprint.iter().flatten().all(|v| v.is_finite())
            {
                return Err(Scene3dError::Building {
                    id: building.id.clone(),
                    reason: "footprint, base elevation, and positive measured height are required"
                        .into(),
                });
            }
        }
        for poi in &self.pois {
            if poi.id.trim().is_empty()
                || poi.category.trim().is_empty()
                || !poi.position.iter().all(|value| value.is_finite())
            {
                return Err(Scene3dError::Poi {
                    id: poi.id.clone(),
                    reason: "finite position and non-empty category are required".into(),
                });
            }
            if !source_ids.contains(poi.source_id.as_str()) {
                return Err(Scene3dError::SourceReference(poi.source_id.clone()));
            }
        }
        if !self.camera.target.iter().all(|v| v.is_finite())
            || !self.camera.radius.is_finite()
            || self.camera.radius <= 0.0
            || !self.camera.pitch_degrees.is_finite()
            || !self.camera.yaw_degrees.is_finite()
        {
            return Err(Scene3dError::Camera);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene3dBenchmark {
    pub adapter: String,
    pub backend: String,
    pub upload_bytes: u64,
    pub upload_ns: u64,
    pub point_count: usize,
    pub building_count: usize,
    pub first_frame_ns: u64,
    pub measured_frames: u32,
    pub steady_state_fps: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

struct SceneMesh {
    points: Vec<Vertex>,
    buildings: Vec<Vertex>,
    indices: Vec<u32>,
    center: [f32; 3],
    scale: f32,
}

impl SceneMesh {
    fn build(scene: &Scene3d) -> Result<Self, Scene3dError> {
        scene.validate()?;
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for point in &scene.points {
            include_bounds(
                &mut min,
                &mut max,
                [point[0] as f32, point[1] as f32, point[2] as f32],
            );
        }
        for building in &scene.buildings {
            for point in &building.footprint {
                include_bounds(
                    &mut min,
                    &mut max,
                    [point[0] as f32, point[1] as f32, building.base_z as f32],
                );
                include_bounds(
                    &mut min,
                    &mut max,
                    [
                        point[0] as f32,
                        point[1] as f32,
                        (building.base_z + building.height) as f32,
                    ],
                );
            }
        }
        let center = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let scale = (max[0] - min[0])
            .max(max[1] - min[1])
            .max(max[2] - min[2])
            .max(1.0)
            * 0.5;
        let normalize = |p: [f32; 3]| {
            [
                (p[0] - center[0]) / scale,
                (p[1] - center[1]) / scale,
                (p[2] - center[2]) / scale,
            ]
        };
        let points = scene
            .points
            .iter()
            .map(|p| Vertex {
                position: normalize([p[0] as f32, p[1] as f32, p[2] as f32]),
                color: [0.25, 0.72, 0.92, 1.0],
            })
            .collect();
        let mut buildings = Vec::new();
        let mut indices = Vec::new();
        for building in &scene.buildings {
            let base = buildings.len() as u32;
            let count = building.footprint.len();
            for point in &building.footprint {
                buildings.push(Vertex {
                    position: normalize([point[0] as f32, point[1] as f32, building.base_z as f32]),
                    color: [0.48, 0.55, 0.62, 1.0],
                });
            }
            for point in &building.footprint {
                buildings.push(Vertex {
                    position: normalize([
                        point[0] as f32,
                        point[1] as f32,
                        (building.base_z + building.height) as f32,
                    ]),
                    color: [0.95, 0.58, 0.26, 1.0],
                });
            }
            for index in 1..count - 1 {
                indices.extend_from_slice(&[
                    base + count as u32,
                    base + count as u32 + index as u32,
                    base + count as u32 + index as u32 + 1,
                ]);
            }
            for index in 0..count {
                let next = (index + 1) % count;
                indices.extend_from_slice(&[
                    base + index as u32,
                    base + next as u32,
                    base + count as u32 + next as u32,
                    base + index as u32,
                    base + count as u32 + next as u32,
                    base + count as u32 + index as u32,
                ]);
            }
        }
        Ok(Self {
            points,
            buildings,
            indices,
            center,
            scale,
        })
    }

    fn upload_bytes(&self) -> u64 {
        (self.points.len() * std::mem::size_of::<Vertex>()
            + self.buildings.len() * std::mem::size_of::<Vertex>()
            + self.indices.len() * std::mem::size_of::<u32>()) as u64
    }
}

struct SceneGpu {
    point_buffer: wgpu::Buffer,
    point_count: u32,
    building_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    point_pipeline: wgpu::RenderPipeline,
    building_pipeline: wgpu::RenderPipeline,
}

impl SceneGpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, mesh: &SceneMesh) -> Self {
        let point_buffer = buffer(
            device,
            "scene-points",
            &mesh.points,
            wgpu::BufferUsages::VERTEX,
        );
        let building_buffer = buffer(
            device,
            "scene-buildings",
            &mesh.buildings,
            wgpu::BufferUsages::VERTEX,
        );
        let index_buffer = buffer(
            device,
            "scene-building-indices",
            &mesh.indices,
            wgpu::BufferUsages::INDEX,
        );
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene-camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene-camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene-camera-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene3d.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene3d-pipeline-layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let point_pipeline = pipeline(
            device,
            format,
            &shader,
            &pipeline_layout,
            wgpu::PrimitiveTopology::PointList,
        );
        let building_pipeline = pipeline(
            device,
            format,
            &shader,
            &pipeline_layout,
            wgpu::PrimitiveTopology::TriangleList,
        );
        Self {
            point_buffer,
            point_count: mesh.points.len() as u32,
            building_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            point_pipeline,
            building_pipeline,
        }
    }

    fn update_camera(&self, queue: &wgpu::Queue, matrix: [[f32; 4]; 4]) {
        let uniform = CameraUniform { view_proj: matrix };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_pipeline(&self.point_pipeline);
        pass.set_vertex_buffer(0, self.point_buffer.slice(..));
        pass.draw(0..self.point_count, 0..1);
        if self.index_count > 0 {
            pass.set_pipeline(&self.building_pipeline);
            pass.set_vertex_buffer(0, self.building_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}

fn buffer<T: Pod>(
    device: &wgpu::Device,
    label: &str,
    values: &[T],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    if values.is_empty() {
        return device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 4,
            usage,
            mapped_at_creation: false,
        });
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(values),
        usage,
    })
}

fn pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene3d-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: if topology == wgpu::PrimitiveTopology::TriangleList {
                Some(wgpu::Face::Back)
            } else {
                None
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

fn include_bounds(min: &mut [f32; 3], max: &mut [f32; 3], point: [f32; 3]) {
    for axis in 0..3 {
        min[axis] = min[axis].min(point[axis]);
        max[axis] = max[axis].max(point[axis]);
    }
}

fn camera_matrix(scene: &Scene3d, mesh: &SceneMesh, aspect: f32) -> [[f32; 4]; 4] {
    let yaw = scene.camera.yaw_degrees.to_radians();
    let pitch = scene.camera.pitch_degrees.clamp(-85.0, 85.0).to_radians();
    let target = [
        (scene.camera.target[0] as f32 - mesh.center[0]) / mesh.scale,
        (scene.camera.target[1] as f32 - mesh.center[1]) / mesh.scale,
        (scene.camera.target[2] as f32 - mesh.center[2]) / mesh.scale,
    ];
    let radius = (scene.camera.radius as f32 / mesh.scale).max(0.1);
    let eye = [
        target[0] + radius * pitch.cos() * yaw.sin(),
        target[1] + radius * pitch.cos() * yaw.cos(),
        target[2] + radius * pitch.sin(),
    ];
    multiply_matrix(
        perspective(45_f32.to_radians(), aspect.max(0.1), 0.01, 100.0),
        look_at(eye, target),
    )
}

fn look_at(eye: [f32; 3], target: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize3(sub3(target, eye));
    let side = normalize3(cross3(forward, [0.0, 0.0, 1.0]));
    let up = cross3(side, forward);
    [
        [side[0], up[0], -forward[0], 0.0],
        [side[1], up[1], -forward[1], 0.0],
        [side[2], up[2], -forward[2], 0.0],
        [-dot3(side, eye), -dot3(up, eye), dot3(forward, eye), 1.0],
    ]
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fovy * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

fn multiply_matrix(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    result
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(value: [f32; 3]) -> [f32; 3] {
    let length = dot3(value, value).sqrt().max(f32::EPSILON);
    [value[0] / length, value[1] / length, value[2] / length]
}

fn render_headless(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    gpu: &SceneGpu,
) -> Result<(), Scene3dError> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene3d-headless-frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene3d-headless-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        gpu.draw(&mut pass);
    }
    queue.submit(Some(encoder.finish()));
    wait_for_queue(device, queue)
}

fn wait_for_queue(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), Scene3dError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    queue.on_submitted_work_done(move || {
        let _ = sender.send(());
    });
    let deadline = Instant::now() + GPU_COMPLETION_TIMEOUT;
    loop {
        let _ = device.poll(wgpu::Maintain::Poll);
        match receiver.try_recv() {
            Ok(()) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(Scene3dError::Gpu(
                    "GPU completion callback disconnected".into(),
                ));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            return Err(Scene3dError::Gpu(format!(
                "GPU queue did not complete within {} seconds",
                GPU_COMPLETION_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub fn benchmark_scene3d_headless(
    scene: &Scene3d,
    width: u32,
    height: u32,
    measured_frames: u32,
) -> Result<Scene3dBenchmark, Scene3dError> {
    if width == 0 || height == 0 || measured_frames == 0 {
        return Err(Scene3dError::Gpu(
            "nonzero extent and frame count required".into(),
        ));
    }
    let started = Instant::now();
    let mesh = SceneMesh::build(scene)?;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or_else(|| Scene3dError::Gpu("no compatible WebGPU adapter".into()))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("genegis-scene3d-benchmark"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .map_err(|error| Scene3dError::Gpu(error.to_string()))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = attachment(&device, "scene3d-color", width, height, format);
    let depth = attachment(
        &device,
        "scene3d-depth",
        width,
        height,
        wgpu::TextureFormat::Depth32Float,
    );
    let view = texture.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let upload_started = Instant::now();
    let gpu = SceneGpu::new(&device, format, &mesh);
    let upload_ns = upload_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    gpu.update_camera(
        &queue,
        camera_matrix(scene, &mesh, width as f32 / height as f32),
    );
    render_headless(&device, &queue, &view, &depth_view, &gpu)?;
    let first_frame_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    let frames_started = Instant::now();
    for _ in 0..measured_frames {
        render_headless(&device, &queue, &view, &depth_view, &gpu)?;
    }
    let elapsed = frames_started.elapsed().as_secs_f64();
    Ok(Scene3dBenchmark {
        adapter: info.name,
        backend: format!("{:?}", info.backend).to_ascii_lowercase(),
        upload_bytes: mesh.upload_bytes(),
        upload_ns,
        point_count: scene.points.len(),
        building_count: scene.buildings.len(),
        first_frame_ns,
        measured_frames,
        steady_state_fps: measured_frames as f64 / elapsed.max(f64::EPSILON),
    })
}

fn attachment(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

struct SceneApp {
    scene: Scene3d,
    mesh: SceneMesh,
    canvas: Option<RenderCanvas>,
    gpu: Option<SceneGpu>,
    dragging: bool,
    last_cursor: Option<(f64, f64)>,
}

impl SceneApp {
    fn update_camera(&self) {
        let (Some(canvas), Some(gpu)) = (&self.canvas, &self.gpu) else {
            return;
        };
        let (width, height) = canvas.size();
        gpu.update_camera(
            canvas.queue(),
            camera_matrix(&self.scene, &self.mesh, width as f32 / height.max(1) as f32),
        );
    }
}

impl ApplicationHandler for SceneApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.canvas.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("GeneGIS 3D Scene — COPC + LOD1")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("window"),
        );
        let canvas = pollster::block_on(RenderCanvas::new(window));
        self.gpu = Some(SceneGpu::new(canvas.device(), canvas.format(), &self.mesh));
        self.canvas = Some(canvas);
        self.update_camera();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(canvas) = &mut self.canvas {
                    canvas.resize(size.width, size.height);
                }
                self.update_camera();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if !self.dragging {
                    self.last_cursor = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.dragging => {
                if let Some((x, y)) = self.last_cursor {
                    self.scene.camera.yaw_degrees += ((position.x - x) * 0.25) as f32;
                    self.scene.camera.pitch_degrees = (self.scene.camera.pitch_degrees
                        + ((position.y - y) * 0.2) as f32)
                        .clamp(-85.0, 85.0);
                    self.update_camera();
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(position) => position.y / 120.0,
                };
                self.scene.camera.radius = (self.scene.camera.radius * (1.0 - scroll * 0.08))
                    .clamp(self.mesh.scale as f64 * 0.2, self.mesh.scale as f64 * 20.0);
                self.update_camera();
            }
            WindowEvent::RedrawRequested => {
                if let (Some(canvas), Some(gpu)) = (&self.canvas, &self.gpu) {
                    let _ = canvas.render_with_depth(CLEAR, |pass| gpu.draw(pass));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(canvas) = &self.canvas {
            canvas.request_redraw();
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}

pub fn run_scene3d_window(scene: Scene3d) -> Result<(), Scene3dError> {
    let mesh = SceneMesh::build(&scene)?;
    let event_loop = EventLoop::new().map_err(|error| Scene3dError::Gpu(error.to_string()))?;
    let mut app = SceneApp {
        scene,
        mesh,
        canvas: None,
        gpu: None,
        dragging: false,
        last_cursor: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|error| Scene3dError::Gpu(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Scene3d {
        Scene3d {
            schema_version: "0.1.0".into(),
            crs: Crs::nagoya_projected(),
            coordinate_unit: CoordinateUnit::Metres,
            vertical_unit: "metres".into(),
            sources: vec![
                SceneSource {
                    id: "copc".into(),
                    role: "point_cloud".into(),
                    snapshot: SourceSnapshot::new("file:///district.copc.laz"),
                },
                SceneSource {
                    id: "lod1".into(),
                    role: "building_height".into(),
                    snapshot: SourceSnapshot::new("file:///buildings.json"),
                },
            ],
            point_source_id: "copc".into(),
            points: vec![[0.0, 0.0, 0.0], [20.0, 20.0, 4.0]],
            buildings: vec![BuildingLod1 {
                id: "b-1".into(),
                footprint: vec![[2.0, 2.0], [8.0, 2.0], [8.0, 7.0], [2.0, 7.0]],
                base_z: 0.0,
                height: 12.5,
                height_source_id: "lod1".into(),
            }],
            pois: vec![ScenePoi {
                id: "poi-1".into(),
                position: [4.0, 4.0, 0.0],
                category: "school".into(),
                source_id: "lod1".into(),
            }],
            camera: OrbitCamera {
                target: [10.0, 10.0, 2.0],
                yaw_degrees: 30.0,
                pitch_degrees: 35.0,
                radius: 40.0,
            },
        }
    }

    #[test]
    fn validates_provenance_and_builds_point_and_extrusion_meshes() {
        let scene = fixture();
        scene.validate().expect("scene contract");
        let mesh = SceneMesh::build(&scene).expect("mesh");
        assert_eq!(mesh.points.len(), 2);
        assert_eq!(mesh.buildings.len(), 8);
        assert_eq!(mesh.indices.len(), 30);
        assert!(mesh.upload_bytes() > 0);
    }

    #[test]
    fn rejects_unknown_height_source_and_non_positive_height() {
        let mut scene = fixture();
        scene.buildings[0].height_source_id = "missing".into();
        assert!(matches!(
            scene.validate(),
            Err(Scene3dError::SourceReference(_))
        ));
        scene.buildings[0].height_source_id = "lod1".into();
        scene.buildings[0].height = 0.0;
        assert!(matches!(
            scene.validate(),
            Err(Scene3dError::Building { .. })
        ));
    }
}
