//! Glyphon-based text rendering for wgpu backend.
#![cfg(feature = "wgpu-backend")]

use std::collections::HashMap;

use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, ColorMode, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
    Weight,
};

use crate::renderer::types::Color;
use crate::renderer::RenderError;

/// Queued text entry for a single frame.
struct QueuedText {
    x: f32,
    y: f32,
    text: String,
    color: Color,
    size: f32,
    bold: bool,
}

/// Cache key: (text, size_key, bold).
type TextCacheKey = (String, u32, bool);

/// Glyphon text renderer wrapper.
pub struct GlyphonTextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    renderer: TextRenderer,
    #[allow(dead_code)]
    cache: Cache,
    viewport: Viewport,

    queued: Vec<QueuedText>,
    /// Content-based buffer cache: avoids re-shaping unchanged text.
    buffer_cache: HashMap<TextCacheKey, Buffer>,

    custom_family: Option<String>,
    default_font_size: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    width: u32,
    height: u32,
}

impl GlyphonTextRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        font_path: Option<&str>,
        font_size: f32,
    ) -> Result<Self, RenderError> {
        let mut font_system = FontSystem::new();

        // Load custom font and detect its family name
        let mut custom_family: Option<String> = None;
        if let Some(path) = font_path {
            let count_before = font_system.db().faces().count();
            match font_system.db_mut().load_font_file(path) {
                Ok(()) => {
                    let mut idx = 0;
                    for face in font_system.db().faces() {
                        if idx >= count_before {
                            if let Some((name, _)) = face.families.first() {
                                custom_family = Some(name.clone());
                                break;
                            }
                        }
                        idx += 1;
                    }
                    if let Some(ref family) = custom_family {
                        log_info!("Custom font loaded: family={}", family);
                    }
                }
                Err(e) => {
                    log_warn!("Failed to load custom font '{}': {}", path, e);
                }
            }
        }

        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        // Use Web color mode: our surface is non-sRGB (linear texture storing sRGB values),
        // so we must skip glyphon's srgb_to_linear conversion in the shader.
        let mut atlas =
            TextAtlas::with_color_mode(device, queue, &cache, surface_format, ColorMode::Web);
        let renderer = TextRenderer::new(&mut atlas, device, Default::default(), None);
        let viewport = Viewport::new(device, &cache);

        let (cell_width, cell_height) =
            Self::measure_cell(&mut font_system, font_size, custom_family.as_deref());

        log_info!(
            "wgpu text: font_size={}, cell={}x{}",
            font_size,
            cell_width,
            cell_height
        );

        Ok(GlyphonTextRenderer {
            font_system,
            swash_cache,
            atlas,
            renderer,
            cache,
            viewport,
            queued: Vec::with_capacity(512),
            buffer_cache: HashMap::with_capacity(256),
            custom_family,
            default_font_size: font_size,
            cell_width,
            cell_height,
            width: 0,
            height: 0,
        })
    }

    fn make_attrs<'a>(bold: bool, family: Option<&'a str>) -> Attrs<'a> {
        match (bold, family) {
            (true, Some(f)) => Attrs::new().weight(Weight::BOLD).family(Family::Name(f)),
            (true, None) => Attrs::new().weight(Weight::BOLD),
            (false, Some(f)) => Attrs::new().family(Family::Name(f)),
            (false, None) => Attrs::new(),
        }
    }

    fn measure_cell(
        font_system: &mut FontSystem,
        font_size: f32,
        family: Option<&str>,
    ) -> (f32, f32) {
        let metrics = Metrics::new(font_size, font_size * 1.2);
        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_size(font_system, Some(200.0), Some(200.0));
        let attrs = Self::make_attrs(false, family);
        buffer.set_text(font_system, "M", &attrs, Shaping::Advanced);
        buffer.shape_until_scroll(font_system, false);

        let mut width = font_size * 0.6;
        let mut height = font_size * 1.2;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                width = glyph.w;
            }
            height = run.line_height;
        }

        (width, height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn begin_frame(&mut self) {
        self.queued.clear();
        // buffer_cache persists across frames
    }

    pub fn queue_text(
        &mut self,
        x: f32,
        y: f32,
        text: &str,
        color: Color,
        size: f32,
        bold: bool,
    ) {
        if text.is_empty() {
            return;
        }
        // Snap to default_font_size when size ≈ cell_height (matches D2D behavior
        // where format_regular at original font_size is used for standard text).
        let effective_size = if (size - self.cell_height).abs() < 1.0 {
            self.default_font_size
        } else {
            size
        };
        self.queued.push(QueuedText {
            x,
            y,
            text: text.to_string(),
            color,
            size: effective_size,
            bold,
        });
    }

    /// Return the number of queued text entries.
    pub fn queued_count(&self) -> usize {
        self.queued.len()
    }

    #[allow(dead_code)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(), glyphon::PrepareError> {
        self.prepare_range(device, queue, 0..self.queued.len())
    }

    /// Prepare only a subset of queued text for rendering.
    /// Called once per layer to ensure correct z-ordering with rects.
    pub fn prepare_range(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        range: std::ops::Range<usize>,
    ) -> Result<(), glyphon::PrepareError> {
        self.viewport.update(
            queue,
            Resolution {
                width: self.width,
                height: self.height,
            },
        );

        let custom_family = self.custom_family.as_deref();
        let width = self.width;
        let height = self.height;

        let subset = &self.queued[range];

        // Ensure all needed buffers exist in cache (shape only on miss)
        for q in subset {
            let size_key = (q.size * 10.0) as u32;
            let key = (q.text.clone(), size_key, q.bold);

            if !self.buffer_cache.contains_key(&key) {
                let metrics = Metrics::new(q.size, q.size * 1.2);
                let mut buffer = Buffer::new(&mut self.font_system, metrics);
                buffer.set_size(&mut self.font_system, Some(10000.0), Some(q.size * 2.0));
                let attrs = Self::make_attrs(q.bold, custom_family);
                buffer.set_text(&mut self.font_system, &q.text, &attrs, Shaping::Advanced);
                buffer.shape_until_scroll(&mut self.font_system, false);
                self.buffer_cache.insert(key, buffer);
            }
        }

        // Build TextArea list referencing cached buffers
        let text_areas: Vec<TextArea> = subset
            .iter()
            .map(|q| {
                let size_key = (q.size * 10.0) as u32;
                let key = (q.text.clone(), size_key, q.bold);
                let buffer = self.buffer_cache.get(&key).unwrap();
                TextArea {
                    buffer,
                    left: q.x,
                    top: q.y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    default_color: GlyphonColor::rgba(q.color.r, q.color.g, q.color.b, 255),
                    custom_glyphs: &[],
                }
            })
            .collect();

        self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        )
    }

    pub fn render<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), glyphon::RenderError> {
        self.renderer
            .render(&self.atlas, &self.viewport, render_pass)
    }
}
