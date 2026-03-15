//! Renderer trait abstraction and supporting types.

#[cfg(feature = "d2d")]
pub mod d2d;

#[cfg(feature = "wgpu-backend")]
pub mod wgpu_backend;

pub mod types;

use crate::renderer::types::Color;

/// Rendering error type.
#[derive(Debug)]
pub enum RenderError {
    /// Device lost — render target must be recreated.
    DeviceLost,
    /// Platform-specific error.
    PlatformError(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::DeviceLost => write!(f, "Device lost"),
            RenderError::PlatformError(msg) => write!(f, "Platform error: {}", msg),
        }
    }
}

impl std::error::Error for RenderError {}

pub type RenderResult<T> = Result<T, RenderError>;

/// Abstract renderer interface. Implementations: D2DRenderer (Windows), WgpuRenderer (cross-platform).
pub trait Renderer {
    /// Resize the render target.
    fn resize(&mut self, width: u32, height: u32) -> RenderResult<()>;

    /// Begin a new frame.
    fn begin_frame(&mut self) -> RenderResult<()>;

    /// End the current frame (present to screen).
    fn end_frame(&mut self) -> RenderResult<()>;

    /// Clear the entire surface with a solid color.
    fn clear(&mut self, color: Color);

    /// Fill a pixel rectangle with a solid color.
    fn fill_rect(&mut self, rect: types::Rect, color: Color);

    /// Draw a vertical line.
    fn draw_vline(&mut self, x: f32, y_top: f32, y_bottom: f32, color: Color, width: f32);

    /// Draw a horizontal line.
    fn draw_hline(&mut self, y: f32, x_left: f32, x_right: f32, color: Color, width: f32);

    /// Draw text at a pixel position (for non-grid text, e.g. labels).
    fn draw_text(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32);

    /// Draw bold text at a pixel position.
    fn draw_text_bold(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32);

    /// Get the cell size in pixels (width, height).
    fn cell_size(&self) -> (f32, f32);

    /// Get the window/surface size in pixels (width, height).
    fn window_size(&self) -> (u32, u32);

    /// Begin an overlay layer. Subsequent draw calls belong to the overlay,
    /// which is rendered on top of all previous content (including text).
    /// Default is a no-op (correct for immediate-mode backends like D2D).
    fn begin_overlay(&mut self) {}
}
