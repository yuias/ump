//! Glyphon-based text rendering for wgpu backend.
#![cfg(feature = "wgpu-backend")]

use std::collections::HashMap;

use glyphon::cosmic_text::{CacheKeyFlags, Hinting};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, ColorMode, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
    Weight,
};

use crate::renderer::types::{Color, TextRenderOptions};
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
    options: TextRenderOptions,
    /// `options.bold`, cleared when the custom font file has no bold face:
    /// for a missing weight cosmic-text can pick a different system font
    /// instead of reusing the regular face (it reuses it only for monospace
    /// fonts). A variable font reports its default weight here, so its `wght`
    /// axis is not counted as a bold face.
    bold_enabled: bool,
    pub(crate) default_font_size: f32,
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
        options: TextRenderOptions,
    ) -> Result<Self, RenderError> {
        let mut font_system = FontSystem::new();

        // Load custom font and detect its family name
        let mut custom_family: Option<String> = None;
        let mut bold_enabled = options.bold;
        if let Some(path) = font_path {
            let count_before = font_system.db().faces().count();
            match font_system.db_mut().load_font_file(path) {
                Ok(()) => {
                    let faces = || font_system.db().faces().skip(count_before);
                    custom_family =
                        faces().find_map(|face| face.families.first().map(|(name, _)| name.clone()));
                    let has_bold = faces().any(|face| face.weight >= Weight::SEMIBOLD);
                    bold_enabled &= has_bold;
                    if let Some(ref family) = custom_family {
                        log_info!("Custom font loaded: family={}, bold face={}", family, has_bold);
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
            Self::measure_cell(&mut font_system, font_size, custom_family.as_deref(), options);

        log_info!(
            "wgpu text: font_size={}, cell={}x{}, {:?}, bold_enabled={}",
            font_size,
            cell_width,
            cell_height,
            options,
            bold_enabled
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
            options,
            bold_enabled,
            default_font_size: font_size,
            cell_width,
            cell_height,
            width: 0,
            height: 0,
        })
    }

    fn make_attrs<'a>(
        bold: bool,
        family: Option<&'a str>,
        options: TextRenderOptions,
    ) -> Attrs<'a> {
        let mut attrs = Attrs::new();
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if let Some(f) = family {
            attrs = attrs.family(Family::Name(f));
        }
        let mut flags = CacheKeyFlags::empty();
        if !options.hinting {
            flags |= CacheKeyFlags::DISABLE_HINTING;
        }
        if options.pixel_font {
            flags |= CacheKeyFlags::PIXEL_FONT;
        }
        attrs.cache_key_flags(flags)
    }

    fn new_buffer(font_system: &mut FontSystem, size: f32, options: TextRenderOptions) -> Buffer {
        let mut buffer = Buffer::new(font_system, Metrics::new(size, size * 1.2));
        buffer.set_hinting(if options.hinting { Hinting::Enabled } else { Hinting::Disabled });
        buffer
    }

    fn measure_cell(
        font_system: &mut FontSystem,
        font_size: f32,
        family: Option<&str>,
        options: TextRenderOptions,
    ) -> (f32, f32) {
        let mut buffer = Self::new_buffer(font_system, font_size, options);
        buffer.set_size(Some(200.0), Some(200.0));
        let attrs = Self::make_attrs(false, family, options);
        buffer.set_text("M", &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        let mut width = font_size * 0.6;
        let mut height = font_size * 1.2;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                width = glyph.w;
            }
            height = run.line_height;
        }

        // Snap to whole physical pixels: rect edges and dot fonts are pixel-snapped
        // elsewhere, and fractional cells would make columns drift.
        (width.round(), height.round())
    }

    /// Re-measure glyph metrics at a new physical px size and drop cached glyph
    /// buffers shaped at the old size (they become dead weight, not incorrect —
    /// buffers are keyed by their own size, so stale entries are just never reused).
    pub fn set_font_size(&mut self, px: f32) {
        let (cell_width, cell_height) =
            Self::measure_cell(&mut self.font_system, px, self.custom_family.as_deref(), self.options);
        self.default_font_size = px;
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        self.buffer_cache.clear();
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
        // Snap to default_font_size when size ≈ cell_height, so standard text
        // is always shaped at the font's original size rather than a scaled one.
        let effective_size = if (size - self.cell_height).abs() < 1.0 {
            self.default_font_size
        } else {
            size
        };
        // Rects are pixel-snapped; a fractional origin here would shift every
        // glyph by a sub-pixel amount and blur it (e.g. columns placed after a
        // `0.6 * ch` swatch).
        self.queued.push(QueuedText {
            x: x.round(),
            y: y.round(),
            text: text.to_string(),
            color,
            size: effective_size,
            bold: bold && self.bold_enabled,
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
        let options = self.options;
        let width = self.width;
        let height = self.height;

        let subset = &self.queued[range];

        // Ensure all needed buffers exist in cache (shape only on miss)
        for q in subset {
            let size_key = (q.size * 10.0) as u32;
            let key = (q.text.clone(), size_key, q.bold);

            if !self.buffer_cache.contains_key(&key) {
                let mut buffer = Self::new_buffer(&mut self.font_system, q.size, options);
                buffer.set_size(Some(10000.0), Some(q.size * 2.0));
                let attrs = Self::make_attrs(q.bold, custom_family, options);
                buffer.set_text(&q.text, &attrs, Shaping::Advanced, None);
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
