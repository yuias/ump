//! wgpu renderer backend (cross-platform).
#![cfg(feature = "wgpu-backend")]

pub mod pipeline;
pub mod text;

use std::sync::Arc;

use winit::window::Window;

use crate::renderer::types::{Color, Rect};
use crate::renderer::{RenderError, RenderResult, Renderer};

use self::pipeline::{RectInstance, RectPipeline};
use self::text::GlyphonTextRenderer;

const INITIAL_INSTANCE_CAPACITY: usize = 4096;

/// wgpu-based hardware-accelerated renderer.
pub struct WgpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,

    rect_pipeline: RectPipeline,
    rect_instances: Vec<RectInstance>,
    /// Persistent GPU buffer for rect instances (reused across frames).
    instance_buffer: wgpu::Buffer,
    instance_buffer_capacity: usize,

    text: GlyphonTextRenderer,

    width: u32,
    height: u32,
    clear_color: Color,

    /// Split point for overlay layer (rect index, text index).
    /// When set, end_frame renders base then overlay in separate passes.
    overlay_split: Option<(usize, usize)>,
}

impl WgpuRenderer {
    /// Create a new WgpuRenderer from a winit window.
    pub fn new(
        window: Arc<Window>,
        width: u32,
        height: u32,
        font_path: Option<&str>,
        font_size: f32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window).map_err(|e| {
            RenderError::PlatformError(format!("Failed to create wgpu surface: {}", e))
        })?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| RenderError::PlatformError(format!("No suitable GPU adapter found: {}", e)))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("ump_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            },
        ))
        .map_err(|e| {
            RenderError::PlatformError(format!("Failed to request wgpu device: {}", e))
        })?;

        let surface_caps = surface.get_capabilities(&adapter);
        // Prefer non-sRGB format to avoid double gamma correction
        // (our Color values are already in sRGB space, matching D2D behavior)
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let rect_pipeline = RectPipeline::new(&device, surface_format, width, height);
        let text = GlyphonTextRenderer::new(&device, &queue, surface_format, font_path, font_size)?;

        // Pre-allocate persistent instance buffer
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect_instance_buffer"),
            size: (INITIAL_INSTANCE_CAPACITY * std::mem::size_of::<RectInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(WgpuRenderer {
            surface,
            device,
            queue,
            surface_config,
            rect_pipeline,
            rect_instances: Vec::with_capacity(INITIAL_INSTANCE_CAPACITY),
            instance_buffer,
            instance_buffer_capacity: INITIAL_INSTANCE_CAPACITY,
            text,
            width,
            height,
            clear_color: Color::rgb(0, 0, 0),
            overlay_split: None,
        })
    }
}

impl Renderer for WgpuRenderer {
    fn resize(&mut self, width: u32, height: u32) -> RenderResult<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.width = width;
        self.height = height;
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.rect_pipeline
            .update_uniforms(&self.queue, width, height);
        self.text.resize(width, height);
        Ok(())
    }

    fn begin_frame(&mut self) -> RenderResult<()> {
        self.rect_instances.clear();
        self.text.begin_frame();
        Ok(())
    }

    fn end_frame(&mut self) -> RenderResult<()> {
        let output = self.surface.get_current_texture().map_err(|e| match e {
            wgpu::SurfaceError::Lost => RenderError::DeviceLost,
            wgpu::SurfaceError::OutOfMemory => {
                RenderError::PlatformError("GPU out of memory".into())
            }
            other => RenderError::PlatformError(format!("Surface error: {}", other)),
        })?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Upload rect instances to persistent GPU buffer
        let rect_count = self.rect_instances.len();
        if rect_count > 0 {
            // Grow buffer if needed
            if rect_count > self.instance_buffer_capacity {
                let new_cap = (rect_count * 2).max(INITIAL_INSTANCE_CAPACITY);
                self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rect_instance_buffer"),
                    size: (new_cap * std::mem::size_of::<RectInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.instance_buffer_capacity = new_cap;
            }
            self.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.rect_instances),
            );
        }

        let (r, g, b) = self.clear_color.to_f32();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        let overlay = self.overlay_split.take();
        let (base_rect_end, base_text_end) = overlay
            .unwrap_or((rect_count, self.text.queued_count()));
        let total_text = self.text.queued_count();

        // Prepare base layer text
        self.text
            .prepare_range(&self.device, &self.queue, 0..base_text_end)
            .map_err(|e| RenderError::PlatformError(format!("Text prepare error: {}", e)))?;

        // Pass 1: base layer (clear + base rects + base text)
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("base_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64,
                            g: g as f64,
                            b: b as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if base_rect_end > 0 {
                render_pass.set_pipeline(&self.rect_pipeline.pipeline);
                render_pass.set_bind_group(0, &self.rect_pipeline.uniform_bind_group, &[]);
                render_pass.set_index_buffer(
                    self.rect_pipeline.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint16,
                );
                render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                render_pass.draw_indexed(0..6, 0, 0..base_rect_end as u32);
            }

            self.text.render(&mut render_pass).map_err(|e| {
                RenderError::PlatformError(format!("Text render error: {}", e))
            })?;
        }

        // Submit base pass before preparing overlay text.
        // glyphon's prepare() modifies the shared atlas texture via queue writes;
        // submitting first ensures the base pass executes with the original atlas.
        self.queue.submit(std::iter::once(encoder.finish()));

        // Pass 2: overlay layer (load + overlay rects + overlay text)
        if overlay.is_some() && (rect_count > base_rect_end || total_text > base_text_end) {
            self.text
                .prepare_range(&self.device, &self.queue, base_text_end..total_text)
                .map_err(|e| {
                    RenderError::PlatformError(format!("Overlay text prepare error: {}", e))
                })?;

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("overlay_encoder"),
                });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                if rect_count > base_rect_end {
                    render_pass.set_pipeline(&self.rect_pipeline.pipeline);
                    render_pass.set_bind_group(0, &self.rect_pipeline.uniform_bind_group, &[]);
                    render_pass.set_index_buffer(
                        self.rect_pipeline.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                    render_pass.draw_indexed(0..6, 0, base_rect_end as u32..rect_count as u32);
                }

                self.text.render(&mut render_pass).map_err(|e| {
                    RenderError::PlatformError(format!("Overlay text render error: {}", e))
                })?;
            }

            self.queue.submit(std::iter::once(encoder.finish()));
        }

        output.present();

        Ok(())
    }

    fn clear(&mut self, color: Color) {
        self.clear_color = color;
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let (r, g, b) = color.to_f32();
        self.rect_instances.push(RectInstance {
            rect: [rect.x, rect.y, rect.width, rect.height],
            color: [r, g, b, 1.0],
        });
    }

    fn draw_vline(&mut self, x: f32, y_top: f32, y_bottom: f32, color: Color, width: f32) {
        self.fill_rect(
            Rect::new(x - width * 0.5, y_top, width, y_bottom - y_top),
            color,
        );
    }

    fn draw_hline(&mut self, y: f32, x_left: f32, x_right: f32, color: Color, width: f32) {
        self.fill_rect(
            Rect::new(x_left, y - width * 0.5, x_right - x_left, width),
            color,
        );
    }

    fn draw_text(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32) {
        self.text.queue_text(x, y, text, color, size, false);
    }

    fn draw_text_bold(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32) {
        self.text.queue_text(x, y, text, color, size, true);
    }

    fn cell_size(&self) -> (f32, f32) {
        (self.text.cell_width, self.text.cell_height)
    }

    fn window_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn begin_overlay(&mut self) {
        self.overlay_split = Some((self.rect_instances.len(), self.text.queued_count()));
    }
}
