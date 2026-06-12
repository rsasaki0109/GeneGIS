//! Minimal WebGPU canvas — Phase 0 prototype.

use std::sync::Arc;

use wgpu::SurfaceError;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

/// Holds wgpu device state for the map canvas prototype.
pub struct RenderCanvas {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl RenderCanvas {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("valid window surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("GPU adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("genegis-render"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .expect("GPU device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Self {
            window,
            surface,
            device,
            queue,
            config,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn render_clear(&self, clear: wgpu::Color) -> Result<(), SurfaceError> {
        self.render_with(clear, |_| {})
    }

    pub fn render_with(
        &self,
        clear: wgpu::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<(), SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("genegis-frame"),
                });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            draw(&mut pass);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    fn render(&self) -> Result<(), SurfaceError> {
        self.render_clear(wgpu::Color {
            r: 0.05,
            g: 0.12,
            b: 0.18,
            a: 1.0,
        })
    }
}

struct App {
    canvas: Option<RenderCanvas>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.canvas.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("GeneGIS Canvas Prototype")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("window"),
        );
        self.canvas = Some(pollster::block_on(RenderCanvas::new(window)));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(canvas) = &mut self.canvas {
                    canvas.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(canvas) = &self.canvas {
                    let _ = canvas.render();
                }
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

/// Run the Phase 0 WebGPU canvas prototype window.
pub fn run_prototype_window() {
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { canvas: None };
    event_loop.run_app(&mut app).expect("run app");
}
