//! Box-drawing border utility for native pixel rendering.

use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;

/// Draw a single-line box border with optional title (native pixel rendering).
pub fn draw_border(renderer: &mut dyn Renderer, area: Rect, title: &str, color: Color) {
    let (cw, ch) = renderer.cell_size();

    if area.width < cw * 2.0 || area.height < ch * 2.0 {
        return;
    }

    let left = area.x;
    let right = area.right();
    let top = area.y;
    let bottom = area.bottom();
    let line_w = 1.0;

    // Top edge
    renderer.draw_hline(top, left, right, color, line_w);
    // Bottom edge
    renderer.draw_hline(bottom, left, right, color, line_w);
    // Left edge
    renderer.draw_vline(left, top, bottom, color, line_w);
    // Right edge
    renderer.draw_vline(right, top, bottom, color, line_w);

    // Title centered on top edge
    if !title.is_empty() {
        let title_w = title.len() as f32 * cw;
        let title_x = left + (area.width - title_w) / 2.0;
        // Clear background behind title
        let bg = crate::renderer::types::BG_COLOR;
        renderer.fill_rect(Rect::new(title_x, top - ch * 0.3, title_w, ch), bg);
        renderer.draw_text(title_x, top - ch * 0.3, title, color, ch);
    }
}

/// Return the inner area of a bordered region (1 cell inset).
pub fn inner_rect(area: Rect, cell_w: f32, cell_h: f32) -> Rect {
    if area.width < cell_w * 2.0 || area.height < cell_h * 2.0 {
        return Rect::new(area.x, area.y, 0.0, 0.0);
    }
    Rect::new(
        area.x + cell_w,
        area.y + cell_h,
        area.width - cell_w * 2.0,
        area.height - cell_h * 2.0,
    )
}
