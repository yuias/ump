//! Layout computation: converts window pixel dimensions to pixel regions.
//! Left-right split layout (PC-98 style). All inputs/outputs are physical px.

use crate::renderer::types::Rect;

/// Standard row height multiplier (relative to cell_h).
/// All list-style components should use `cell_h * ROW_HEIGHT` for line spacing.
pub const ROW_HEIGHT: f32 = 1.15;

/// Outer gutter and inter-region gap, in logical px.
const GUTTER: f32 = 6.0;

/// Convert a logical px value to a physical px value at the given scale factor,
/// rounded to the nearest whole pixel.
pub fn px(logical: f32, scale: f32) -> f32 {
    (logical * scale).round()
}

/// Frame line width, matching `Renderer::dot_size()`. Layout has no renderer
/// instance to call, so the formula is duplicated here; keep both in sync.
pub fn dot_size(scale: f32) -> f32 {
    scale.round().max(1.0)
}

/// Height of a panel's title strip, given the cell height and scale factor.
pub fn title_height(cell_h: f32, scale: f32) -> f32 {
    cell_h + 2.0 * px(2.0, scale)
}

/// Inner content rect of a panel: frame excluded on all sides, title strip
/// excluded from the top (the top frame edge lies inside the title strip).
pub fn panel_content(area: Rect, title_h: f32, dot: f32) -> Rect {
    Rect::new(
        area.x + dot,
        area.y + title_h,
        (area.width - 2.0 * dot).max(0.0),
        (area.height - title_h - dot).max(0.0),
    )
}

/// Computed layout regions for the player screen (all pixel-based).
pub struct Layout {
    /// Left panel outer area (TRACK, including frame).
    pub left_panel: Rect,
    /// Left panel inner content area (frame and title strip excluded).
    pub left_content: Rect,
    /// Right panel outer area (Piano Roll, including frame).
    pub right_panel: Rect,
    /// Right panel inner content area (frame and title strip excluded).
    pub right_content: Rect,
    /// Header row.
    pub header: Rect,
    /// Playback control bar.
    pub transport: Rect,
    /// Function-key hint bar.
    pub fkey_bar: Rect,
    /// Panel title strip height (shared by left/right panels).
    pub title_h: f32,
}

impl Layout {
    /// Compute layout from window pixel dimensions.
    ///
    /// Layout order (top to bottom), gutter/gap = `px(6)`:
    ///   Header → [Left | Right panels] → Transport → Function-key bar
    /// The content row absorbs any leftover height.
    pub fn compute(window_w: f32, window_h: f32, cell_w: f32, cell_h: f32, scale: f32) -> Self {
        let gutter = px(GUTTER, scale);
        let dot = dot_size(scale);

        let full_w = (window_w - 2.0 * gutter).max(0.0);
        let content_x = gutter;

        let header_h = cell_h + 2.0 * px(4.0, scale);
        let header_y = gutter;

        let transport_h = cell_h + 2.0 * px(8.0, scale);
        let fkey_h = cell_h + 2.0 * px(3.0, scale);

        let fkey_y = window_h - gutter - fkey_h;
        let transport_y = fkey_y - gutter - transport_h;

        let content_y = header_y + header_h + gutter;
        let content_h = (transport_y - gutter - content_y).max(0.0);

        let left_w = (60.0 * cell_w).min(full_w * 0.45).round();
        let right_x = content_x + left_w + gutter;
        let right_w = (full_w - left_w - gutter).max(0.0);

        let left_panel = Rect::new(content_x, content_y, left_w, content_h);
        let right_panel = Rect::new(right_x, content_y, right_w, content_h);

        let title_h = title_height(cell_h, scale);
        let left_content = panel_content(left_panel, title_h, dot);
        let right_content = panel_content(right_panel, title_h, dot);

        let header = Rect::new(content_x, header_y, full_w, header_h);
        let transport = Rect::new(content_x, transport_y, full_w, transport_h);
        let fkey_bar = Rect::new(content_x, fkey_y, full_w, fkey_h);

        Layout {
            left_panel,
            left_content,
            right_panel,
            right_content,
            header,
            transport,
            fkey_bar,
            title_h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn check_layout(window_w: f32, window_h: f32, cell_w: f32, cell_h: f32, scale: f32) {
        let l = Layout::compute(window_w, window_h, cell_w, cell_h, scale);
        let gutter = px(GUTTER, scale);

        // Top-level regions must not overlap each other.
        let regions = [l.header, l.left_panel, l.right_panel, l.transport, l.fkey_bar];
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                assert!(
                    !overlaps(regions[i], regions[j]),
                    "regions {i} and {j} overlap at {window_w}x{window_h}@{scale}"
                );
            }
        }

        // All regions must stay within the window minus the outer gutter.
        for r in regions {
            assert!(r.x >= gutter - 0.5, "region left of gutter: {:?}", r);
            assert!(r.y >= gutter - 0.5, "region above gutter: {:?}", r);
            assert!(
                r.right() <= window_w - gutter + 0.5,
                "region right of gutter: {:?}",
                r
            );
            assert!(
                r.bottom() <= window_h - gutter + 0.5,
                "region below gutter: {:?}",
                r
            );
        }

        // fkey bar sits flush against the bottom gutter.
        assert!((l.fkey_bar.bottom() - (window_h - gutter)).abs() < 0.5);

        // Left panel must not exceed 45% of the content row's width.
        let content_w = l.left_panel.width + gutter + l.right_panel.width;
        assert!(l.left_panel.width <= content_w * 0.45 + 0.5);
    }

    #[test]
    fn layout_1280x720_scale_1_cell_8x16() {
        check_layout(1280.0, 720.0, 8.0, 16.0, 1.0);
    }

    #[test]
    fn layout_2560x1440_scale_2_cell_16x32() {
        check_layout(2560.0, 1440.0, 16.0, 32.0, 2.0);
    }
}
