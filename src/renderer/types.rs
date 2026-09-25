//! Core rendering types: Color, Rect.

/// RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Convert to f32 tuple (0.0..1.0) for GPU color values.
    pub fn to_f32(self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }
}

/// Pixel-coordinate rectangle (f32 for sub-pixel precision).
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// Glyph rasterization settings, chosen per font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRenderOptions {
    /// Snap glyph advances to whole pixels and hint outlines. Without it,
    /// glyphs land on quarter-pixel offsets and their stems smear across
    /// two pixel columns.
    pub hinting: bool,
    /// Rasterize with no sub-pixel offset at all; meant for bitmap-style dot
    /// fonts whose outlines already sit on the pixel grid.
    pub pixel_font: bool,
    /// Allow bold text. The renderer still drops bold when the loaded font
    /// file has no bold face.
    pub bold: bool,
}

impl Default for TextRenderOptions {
    fn default() -> Self {
        TextRenderOptions { hinting: true, pixel_font: false, bold: true }
    }
}
