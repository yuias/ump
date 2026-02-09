//! Layout computation: converts window pixel dimensions to pixel regions.
//! Left-right split layout (RCP-98 style).

use crate::renderer::types::Rect;
use crate::ui::border::inner_rect;

/// Margin between sibling sections (cell rows). BG_COLOR is visible in this gap.
pub const SECTION_MARGIN: u16 = 1;

/// Horizontal padding inside sections (cell columns).
pub const SECTION_PADDING_X: u16 = 1;

/// Standard row height multiplier (relative to cell_h).
/// All list-style components should use `cell_h * ROW_HEIGHT` for line spacing.
pub const ROW_HEIGHT: f32 = 1.15;

/// Left panel width in cell columns.
const LEFT_PANEL_COLS: u16 = 60;

/// Computed layout regions for the player screen (all pixel-based).
pub struct Layout {
    /// Left panel outer area (TRACK, including border).
    pub left_panel: Rect,
    /// Left panel inner content area (border excluded).
    pub left_content: Rect,
    /// Right panel outer area (Monitor/PianoRoll, including border).
    pub right_panel: Rect,
    /// Right panel inner content area (border excluded).
    pub right_content: Rect,
    /// Playback control bar.
    pub transport: Rect,
    /// Quick action hints bar.
    pub status_bar: Rect,
}

impl Layout {
    /// Compute layout from total grid dimensions.
    ///
    /// Layout order (top to bottom):
    ///   Header (1 row) → margin → [Left | Right panels] → margin →
    ///   Transport (1 row) → margin → Status bar (1 row)
    pub fn compute(cols: u16, rows: u16, cell_w: f32, cell_h: f32) -> Self {
        // Fixed rows: header(1) + margins(3) + transport(1) + status(1)
        let header_rows: u16 = 1;
        let transport_rows: u16 = 1;
        let status_rows: u16 = 1;
        let margins: u16 = 3; // header↔content, content↔transport, transport↔status

        let fixed = header_rows + transport_rows + status_rows + margins * SECTION_MARGIN;
        let content_rows = rows.saturating_sub(fixed).max(1);

        let content_y = (header_rows + SECTION_MARGIN) as f32 * cell_h;
        let content_h = content_rows as f32 * cell_h;
        let full_w = cols as f32 * cell_w;

        // Left panel: fixed width
        let left_w = (LEFT_PANEL_COLS as f32 * cell_w).min(full_w * 0.5);
        let left_panel = Rect::new(0.0, content_y, left_w, content_h);
        let left_content = inner_rect(left_panel, cell_w, cell_h);

        // Right panel: remaining width
        let right_x = left_w;
        let right_w = (full_w - left_w).max(0.0);
        let right_panel = Rect::new(right_x, content_y, right_w, content_h);
        let right_content = inner_rect(right_panel, cell_w, cell_h);

        let transport_y = content_y + content_h + SECTION_MARGIN as f32 * cell_h;
        let transport = Rect::new(0.0, transport_y, full_w, cell_h);

        let status_y = transport_y + cell_h + SECTION_MARGIN as f32 * cell_h;
        let status_bar = Rect::new(0.0, status_y, full_w, cell_h);

        Layout {
            left_panel,
            left_content,
            right_panel,
            right_content,
            transport,
            status_bar,
        }
    }
}
