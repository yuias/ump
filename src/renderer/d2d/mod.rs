//! Direct2D renderer backend for Windows.
#![cfg(feature = "d2d")]

pub mod com;
pub mod text;

use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, ID2D1Factory, ID2D1HwndRenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::renderer::types::{Color, Rect};
use crate::renderer::{RenderError, RenderResult, Renderer};

use self::text::TextRenderer;

/// Direct2D hardware-accelerated renderer.
pub struct D2DRenderer {
    render_target: ID2D1HwndRenderTarget,
    text_renderer: TextRenderer,
    brush_cache: HashMap<(u8, u8, u8), ID2D1SolidColorBrush>,
    width: u32,
    height: u32,
}

impl D2DRenderer {
    /// Create a new D2DRenderer attached to the given window handle.
    pub fn new(
        hwnd: HWND,
        width: u32,
        height: u32,
        font_path: Option<&str>,
        font_family: &str,
        font_size: f32,
    ) -> Result<Self, RenderError> {
        com::init_com()?;
        let factory = com::create_d2d_factory()?;

        let render_target = Self::create_render_target(&factory, hwnd, width, height)?;
        let text_renderer = if let Some(path) = font_path {
            TextRenderer::with_font_file(path, font_size).unwrap_or_else(|e| {
                log_warn!("Custom font failed: {}, fallback to {}", e, font_family);
                TextRenderer::new(font_family, font_size).expect("Fallback font failed")
            })
        } else {
            TextRenderer::new(font_family, font_size)?
        };

        Ok(D2DRenderer {
            render_target,
            text_renderer,
            brush_cache: HashMap::new(),
            width,
            height,
        })
    }

    fn create_render_target(
        factory: &ID2D1Factory,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<ID2D1HwndRenderTarget, RenderError> {
        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode:
                    windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            ..Default::default()
        };

        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U { width, height },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };

        unsafe {
            factory
                .CreateHwndRenderTarget(&rt_props, &hwnd_props)
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateHwndRenderTarget failed: {}",
                        e
                    ))
                })
        }
    }

    /// Get or create a solid color brush.
    fn get_brush(&mut self, color: Color) -> &ID2D1SolidColorBrush {
        let key = (color.r, color.g, color.b);
        if !self.brush_cache.contains_key(&key) {
            let (r, g, b) = color.to_f32();
            let d2d_color = D2D1_COLOR_F {
                r,
                g,
                b,
                a: 1.0,
            };
            let brush = unsafe {
                self.render_target
                    .CreateSolidColorBrush(&d2d_color, None)
                    .expect("CreateSolidColorBrush failed")
            };
            self.brush_cache.insert(key, brush);
        }
        &self.brush_cache[&key]
    }

}

impl Renderer for D2DRenderer {
    fn resize(&mut self, width: u32, height: u32) -> RenderResult<()> {
        self.width = width;
        self.height = height;
        self.brush_cache.clear();
        let size = D2D_SIZE_U { width, height };
        unsafe {
            self.render_target.Resize(&size).map_err(|e| {
                RenderError::PlatformError(format!("Resize failed: {}", e))
            })?;
        }
        Ok(())
    }

    fn begin_frame(&mut self) -> RenderResult<()> {
        unsafe {
            self.render_target.BeginDraw();
        }
        Ok(())
    }

    fn end_frame(&mut self) -> RenderResult<()> {
        unsafe {
            self.render_target.EndDraw(None, None).map_err(|e| {
                // Check for device lost
                if e.code().0 as u32 == 0x8899000C {
                    // D2DERR_RECREATE_TARGET
                    RenderError::DeviceLost
                } else {
                    RenderError::PlatformError(format!("EndDraw failed: {}", e))
                }
            })?;
        }
        Ok(())
    }

    fn clear(&mut self, color: Color) {
        let (r, g, b) = color.to_f32();
        unsafe {
            self.render_target.Clear(Some(&D2D1_COLOR_F {
                r,
                g,
                b,
                a: 1.0,
            }));
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let brush = self.get_brush(color).clone();
        let d2d_rect = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: rect.x,
            top: rect.y,
            right: rect.right(),
            bottom: rect.bottom(),
        };
        unsafe {
            self.render_target.FillRectangle(&d2d_rect, &brush);
        }
    }

    fn draw_vline(&mut self, x: f32, y_top: f32, y_bottom: f32, color: Color, width: f32) {
        let brush = self.get_brush(color).clone();
        let p0 = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x, y: y_top };
        let p1 = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x, y: y_bottom };
        unsafe {
            self.render_target.DrawLine(p0, p1, &brush, width, None);
        }
    }

    fn draw_hline(&mut self, y: f32, x_left: f32, x_right: f32, color: Color, width: f32) {
        let brush = self.get_brush(color).clone();
        let p0 = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x: x_left, y };
        let p1 = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F { x: x_right, y };
        unsafe {
            self.render_target.DrawLine(p0, p1, &brush, width, None);
        }
    }

    fn draw_text(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32) {
        let brush = self.get_brush(color).clone();
        let text_wide: Vec<u16> = text.encode_utf16().collect();

        // Use default format for cell-size text, cached format otherwise
        let ch = self.text_renderer.cell_height;
        let format = if (size - ch).abs() < 0.5 {
            self.text_renderer.format_regular.clone()
        } else {
            self.text_renderer.get_format(size).clone()
        };

        let rect = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: x,
            top: y,
            right: self.width as f32,
            bottom: y + size * 1.5,
        };
        unsafe {
            self.render_target.DrawText(
                &text_wide,
                &format,
                &rect,
                &brush,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn draw_text_bold(&mut self, x: f32, y: f32, text: &str, color: Color, size: f32) {
        let brush = self.get_brush(color).clone();
        let text_wide: Vec<u16> = text.encode_utf16().collect();

        let format = self.text_renderer.format_bold.clone();

        let rect = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
            left: x,
            top: y,
            right: self.width as f32,
            bottom: y + size * 1.5,
        };
        unsafe {
            self.render_target.DrawText(
                &text_wide,
                &format,
                &rect,
                &brush,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn cell_size(&self) -> (f32, f32) {
        (self.text_renderer.cell_width, self.text_renderer.cell_height)
    }

    fn window_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
