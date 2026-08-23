//! Headless WebGPU upload and frame benchmark for reproducible CI evidence.

use crate::{ChoroplethGpu, ChoroplethMap, ChoroplethMesh};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Measured headless GPU adapter, upload, and render timings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadlessGpuBenchmark {
    /// Adapter name reported by wgpu.
    pub adapter: String,
    /// Backend reported by wgpu.
    pub backend: String,
    /// Device type reported by wgpu (discrete, integrated, virtual, CPU, or other).
    pub device_type: String,
    /// Exact vertex and index bytes uploaded.
    pub upload_bytes: u64,
    /// GPU resource creation duration.
    pub upload_ns: u64,
    /// End-to-end initialization through completion of the first frame.
    pub first_frame_ns: u64,
    /// Completed steady-state frame count.
    pub measured_frames: u32,
    /// Frames per second after the first frame.
    pub steady_state_fps: f64,
}

/// Build the selected mesh, upload it, and complete headless render passes on the chosen adapter.
pub fn benchmark_headless_gpu(
    map: &ChoroplethMap,
    width: u32,
    height: u32,
    measured_frames: u32,
) -> Result<HeadlessGpuBenchmark, String> {
    if width == 0 || height == 0 || measured_frames == 0 {
        return Err("GPU benchmark requires nonzero extent and frame count".into());
    }
    let started = Instant::now();
    let mesh = ChoroplethMesh::build(map, width as f32, height as f32);
    if mesh.is_empty() {
        return Err("GPU benchmark mesh is empty".into());
    }
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or_else(|| "no compatible WebGPU adapter".to_string())?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("genegis-headless-benchmark"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .map_err(|error| format!("request GPU device: {error}"))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("genegis-headless-frame"),
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
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let upload_started = Instant::now();
    let gpu = ChoroplethGpu::new(&device, format, &mesh);
    let upload_ns = nanos(upload_started.elapsed());
    render_once(&device, &queue, &view, &gpu);
    let first_frame_ns = nanos(started.elapsed());
    let frames_started = Instant::now();
    for _ in 0..measured_frames {
        render_once(&device, &queue, &view, &gpu);
    }
    let seconds = frames_started.elapsed().as_secs_f64();
    Ok(HeadlessGpuBenchmark {
        adapter: info.name,
        backend: format!("{:?}", info.backend).to_ascii_lowercase(),
        device_type: format!("{:?}", info.device_type).to_ascii_lowercase(),
        upload_bytes: mesh.upload_bytes(),
        upload_ns,
        first_frame_ns,
        measured_frames,
        steady_state_fps: measured_frames as f64 / seconds.max(f64::EPSILON),
    })
}

fn render_once(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    gpu: &ChoroplethGpu,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("genegis-headless-frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("genegis-headless-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        gpu.draw(&mut pass);
    }
    let submission = queue.submit(Some(encoder.finish()));
    device.poll(wgpu::Maintain::wait_for(submission));
}

fn nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_geometry::PolygonRing;
    use genegis_style::ColorRgba;

    #[test]
    fn measures_real_wgpu_adapter_when_runner_has_one() {
        let mut map = ChoroplethMap::default();
        map.push_feature(
            vec![PolygonRing::new(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ])],
            ColorRgba::new(0.2, 0.4, 0.8, 1.0),
        );
        let report = benchmark_headless_gpu(&map, 64, 64, 3).expect("headless adapter");
        assert!(!report.adapter.is_empty());
        assert!(report.upload_bytes > 0);
        assert!(report.first_frame_ns > 0);
        assert!(report.steady_state_fps.is_finite());
    }
}
