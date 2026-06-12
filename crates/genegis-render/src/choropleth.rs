//! WebGPU choropleth renderer — Phase 2 alpha (Nagoya wards).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use genegis_geometry::PolygonRing;
use genegis_style::ColorRgba;
use wgpu::util::DeviceExt;
use wgpu::SurfaceError;
use winit::{
    application::ApplicationHandler,
    event::{MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

use crate::canvas::RenderCanvas;
use crate::tiled_lod::{lod_for_zoom, ChoroplethTiledLodMap, TiledLodConfig};

const PAD_PX: f32 = 40.0;
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.043,
    g: 0.071,
    b: 0.102,
    a: 1.0,
};

/// One styled polygon feature for GPU choropleth rendering.
#[derive(Debug, Clone)]
pub struct ChoroplethFeature {
    pub rings: Vec<PolygonRing>,
    pub color: ColorRgba,
}

/// Input map: ward polygons + fill colors.
#[derive(Debug, Clone, Default)]
pub struct ChoroplethMap {
    pub features: Vec<ChoroplethFeature>,
}

impl ChoroplethMap {
    pub fn push_feature(&mut self, rings: Vec<PolygonRing>, color: ColorRgba) {
        self.features.push(ChoroplethFeature { rings, color });
    }

    /// Map bounding box in WGS84 lon/lat `(min_x, min_y, max_x, max_y)`.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for feature in &self.features {
            for ring in &feature.rings {
                for (x, y) in ring.exterior() {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
            }
        }

        (min_x, min_y, max_x, max_y)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// CPU-side triangulated mesh ready for GPU upload.
#[derive(Debug, Clone)]
pub struct ChoroplethMesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl ChoroplethMesh {
    pub fn build(map: &ChoroplethMap, viewport_w: f32, viewport_h: f32) -> Self {
        build_mesh_from_features(&map.features, map.bbox(), viewport_w, viewport_h)
    }

    /// Merge multiple tile meshes into one indexed mesh.
    pub fn merge<'a>(meshes: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for mesh in meshes {
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend(mesh.indices.iter().map(|idx| base + idx));
        }

        Self { vertices, indices }
    }

    /// Number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.indices.is_empty()
    }
}

pub(crate) fn build_mesh_from_features(
    features: &[ChoroplethFeature],
    map_bbox: (f64, f64, f64, f64),
    viewport_w: f32,
    viewport_h: f32,
) -> ChoroplethMesh {
    let (min_x, min_y, max_x, max_y) = map_bbox;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for feature in features {
        let color = [
            feature.color.r,
            feature.color.g,
            feature.color.b,
            feature.color.a,
        ];

        for ring in &feature.rings {
            let coords: Vec<f64> = ring
                .exterior()
                .iter()
                .flat_map(|(x, y)| [*x, *y])
                .collect();
            if coords.len() < 6 {
                continue;
            }

            let tri_indices = match earcutr::earcut(&coords, &[], 2) {
                Ok(indices) => indices,
                Err(_) => continue,
            };
            if tri_indices.is_empty() {
                continue;
            }

            let base = vertices.len() as u32;
            for chunk in coords.chunks(2) {
                let ndc = lonlat_to_ndc(
                    chunk[0],
                    chunk[1],
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                    viewport_w,
                    viewport_h,
                );
                vertices.push(Vertex {
                    position: ndc,
                    color,
                });
            }

            for idx in tri_indices {
                indices.push(base + idx as u32);
            }
        }
    }

    ChoroplethMesh { vertices, indices }
}

/// GPU resources for drawing a choropleth mesh.
pub struct ChoroplethGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    pipeline: wgpu::RenderPipeline,
}

/// GPU batches for tiled choropleth drawing (one pipeline, many tile buffers).
pub struct ChoroplethTiledGpu {
    pipeline: wgpu::RenderPipeline,
    batches: Vec<ChoroplethGpuBatch>,
}

struct ChoroplethGpuBatch {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl ChoroplethGpu {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        mesh: &ChoroplethMesh,
    ) -> Self {
        let pipeline = create_choropleth_pipeline(device, surface_format);
        let batch = create_gpu_batch(device, mesh);
        Self {
            vertex_buffer: batch.vertex_buffer,
            index_buffer: batch.index_buffer,
            index_count: batch.index_count,
            pipeline,
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

impl ChoroplethTiledGpu {
    /// Upload one GPU batch per non-empty tile mesh.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        meshes: &[&ChoroplethMesh],
    ) -> Self {
        let pipeline = create_choropleth_pipeline(device, surface_format);
        let batches = meshes
            .iter()
            .filter(|mesh| !mesh.is_empty())
            .map(|mesh| create_gpu_batch(device, mesh))
            .collect();

        Self { pipeline, batches }
    }

    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        for batch in &self.batches {
            if batch.index_count == 0 {
                continue;
            }
            pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
            pass.set_index_buffer(batch.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
        }
    }
}

fn create_gpu_batch(device: &wgpu::Device, mesh: &ChoroplethMesh) -> ChoroplethGpuBatch {
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("choropleth-vertices"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("choropleth-indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    ChoroplethGpuBatch {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
    }
}

fn create_choropleth_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("choropleth-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/choropleth.wgsl").into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("choropleth-layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("choropleth-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

struct ChoroplethApp {
    map: ChoroplethMap,
    tiled: ChoroplethTiledLodMap,
    zoom: f32,
    canvas: Option<RenderCanvas>,
    gpu: Option<ChoroplethTiledGpu>,
}

impl ChoroplethApp {
    fn rebuild_gpu(&mut self) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let (width, height) = canvas.size();
        let lod = lod_for_zoom(self.zoom, self.tiled.lod_levels());
        let tile_meshes = self.tiled.build_tile_meshes(width as f32, height as f32, lod);
        let mesh_refs: Vec<&ChoroplethMesh> = tile_meshes
            .iter()
            .map(|tile| &tile.mesh)
            .collect();
        self.gpu = Some(ChoroplethTiledGpu::new(
            canvas.device(),
            canvas.format(),
            &mesh_refs,
        ));
    }

    fn render_frame(&self) -> Result<(), SurfaceError> {
        let Some(canvas) = &self.canvas else {
            return Ok(());
        };
        let Some(gpu) = &self.gpu else {
            return canvas.render_clear(CLEAR);
        };

        canvas.render_with(CLEAR, |pass| gpu.draw(pass))
    }
}

impl ApplicationHandler for ChoroplethApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.canvas.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("GeneGIS Choropleth — Nagoya population density")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("window"),
        );

        let canvas = pollster::block_on(RenderCanvas::new(window));
        self.tiled = ChoroplethTiledLodMap::prepare(&self.map, TiledLodConfig::default());
        self.canvas = Some(canvas);
        self.rebuild_gpu();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 120.0) as f32,
                };
                self.zoom = (self.zoom - scroll * 0.1).clamp(0.25, 2.0);
                self.rebuild_gpu();
            }
            WindowEvent::Resized(size) => {
                if let Some(canvas) = &mut self.canvas {
                    canvas.resize(size.width, size.height);
                    self.rebuild_gpu();
                }
            }
            WindowEvent::RedrawRequested => {
                let _ = self.render_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(canvas) = &self.canvas {
            canvas.request_redraw();
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        }
    }
}

/// Open an interactive WebGPU choropleth window for the given map.
pub fn run_choropleth_window(map: ChoroplethMap) {
    let event_loop = EventLoop::new().expect("event loop");
    let tiled = ChoroplethTiledLodMap::prepare(&map, TiledLodConfig::default());
    let mut app = ChoroplethApp {
        map,
        tiled,
        zoom: 1.0,
        canvas: None,
        gpu: None,
    };
    event_loop.run_app(&mut app).expect("run app");
}

fn lonlat_to_ndc(
    x: f64,
    y: f64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    viewport_w: f32,
    viewport_h: f32,
) -> [f32; 2] {
    let dx = (max_x - min_x).max(1e-9);
    let dy = (max_y - min_y).max(1e-9);

    let avail_w = viewport_w - PAD_PX * 2.0;
    let avail_h = viewport_h - PAD_PX * 2.0;
    let map_aspect = (dx / dy) as f32;
    let view_aspect = avail_w / avail_h;

    let (draw_w, draw_h) = if map_aspect > view_aspect {
        (avail_w, avail_w / map_aspect)
    } else {
        (avail_h * map_aspect, avail_h)
    };

    let offset_x = PAD_PX + (avail_w - draw_w) / 2.0;
    let offset_y = PAD_PX + (avail_h - draw_h) / 2.0;

    let sx = offset_x + ((x - min_x) / dx * draw_w as f64) as f32;
    let sy = offset_y + ((max_y - y) / dy * draw_h as f64) as f32;

    let ndc_x = (sx / viewport_w) * 2.0 - 1.0;
    let ndc_y = 1.0 - (sy / viewport_h) * 2.0;
    [ndc_x, ndc_y]
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_style::ChoroplethStyle;

    fn sample_map() -> ChoroplethMap {
        let style = ChoroplethStyle::equal_interval(
            "density",
            "persons/km²",
            &[5000.0, 12000.0, 18000.0],
            3,
        );

        let mut map = ChoroplethMap::default();
        map.push_feature(
            vec![PolygonRing::new(vec![
                (136.88, 35.15),
                (136.95, 35.15),
                (136.95, 35.20),
                (136.88, 35.20),
                (136.88, 35.15),
            ])],
            style.color_for(6000.0),
        );
        map.push_feature(
            vec![PolygonRing::new(vec![
                (136.95, 35.15),
                (137.02, 35.15),
                (137.02, 35.20),
                (136.95, 35.20),
                (136.95, 35.15),
            ])],
            style.color_for(15000.0),
        );
        map
    }

    #[test]
    fn mesh_builds_triangles() {
        let mesh = ChoroplethMesh::build(&sample_map(), 1280.0, 720.0);
        assert!(!mesh.is_empty());
        assert!(mesh.triangle_count() >= 2);
    }

    #[test]
    fn nagoya_mesh_builds_sixteen_wards() {
        use genegis_analysis::{default_nagoya_data_path, run_nagoya_population_density};

        let analysis = run_nagoya_population_density(default_nagoya_data_path()).expect("analysis");
        let mut map = ChoroplethMap::default();
        for feature in &analysis.features {
            map.push_feature(feature.rings.clone(), feature.color);
        }

        let mesh = ChoroplethMesh::build(&map, 1280.0, 720.0);
        assert_eq!(map.features.len(), 16);
        assert!(!mesh.is_empty());
        assert!(mesh.triangle_count() > 100);
    }
}
