//! Panel helper: PC-98-style frame with a solid title strip (native pixel rendering).

use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::layout::px;
use crate::ui::theme;

/// Draw a panel: `GROUND` fill, a solid `TITLE_BG` title strip with left-aligned
/// `title` text, and a `FRAME`-colored border inset by `renderer.dot_size()`.
pub fn draw_panel(renderer: &mut dyn Renderer, area: Rect, title: &str, title_h: f32) {
    if area.width <= 0.0 || area.height <= 0.0 {
        return;
    }

    renderer.fill_rect(area, theme::GROUND);
    renderer.fill_rect(Rect::new(area.x, area.y, area.width, title_h), theme::TITLE_BG);

    let dot = renderer.dot_size();
    let left = area.x;
    let top = area.y;
    let right = area.right();
    let bottom = area.bottom();

    // Frame edges, drawn inside the rect so they stay within `area`.
    renderer.fill_rect(Rect::new(left, top, area.width, dot), theme::FRAME); // top
    renderer.fill_rect(Rect::new(left, bottom - dot, area.width, dot), theme::FRAME); // bottom
    renderer.fill_rect(Rect::new(left, top, dot, area.height), theme::FRAME); // left
    renderer.fill_rect(Rect::new(right - dot, top, dot, area.height), theme::FRAME); // right

    if !title.is_empty() {
        let scale = renderer.scale_factor();
        let (_, ch) = renderer.cell_size();
        let x = area.x + px(8.0, scale);
        let y = area.y + (title_h - ch) / 2.0;
        renderer.draw_text(x, y, title, theme::TITLE_FG, ch);
    }
}
