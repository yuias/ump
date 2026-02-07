//! Layout computation: converts window pixel dimensions to pixel regions.
//! Implements a CSS-like box model with uniform section margins.

use crate::renderer::types::Rect;

/// Height of native title bars (piano roll, channels) in cell-height units.
pub const TITLE_BAR_HEIGHT: f32 = 1.6;

/// Cell rows allocated for each title bar region.
pub const TITLE_BAR_CELL_ROWS: u16 = 2;

/// Margin between sibling sections (cell rows). BG_COLOR is visible in this gap.
pub const SECTION_MARGIN: u16 = 1;

/// Horizontal padding inside sections (cell columns).
pub const SECTION_PADDING_X: u16 = 1;

/// Computed layout regions for the player screen (all pixel-based).
pub struct Layout {
    /// Pixel-precise region for native piano roll rendering (including its title bar).
    pub piano_roll_px: Option<Rect>,
    /// Pixel region for native track list title bar (Channels).
    pub track_title_px: Rect,
    /// Channels content area (pixel).
    pub track_list: Rect,
    /// Playback control bar (pixel).
    pub transport: Rect,
    /// Quick action hints bar (pixel).
    pub status_bar: Rect,
}

impl Layout {
    /// Compute layout from total grid dimensions and display options.
    ///
    /// `channels_needed_rows` is the content rows the Channels section needs
    /// (excluding title bar). When piano roll is shown, Channels gets only what
    /// it needs and the remainder goes to Piano Roll.
    ///
    /// Layout order (top to bottom):
    ///   Header (1 row) → margin → [Piano Roll → margin →]
    ///   Channels title (TITLE_BAR_CELL_ROWS) + content → margin →
    ///   Transport (1 row) → margin → Status bar (1 row)
    pub fn compute(
        cols: u16,
        rows: u16,
        show_piano_roll: bool,
        channels_needed_rows: u16,
        cell_w: f32,
        cell_h: f32,
    ) -> Self {
        // Fixed row consumption:
        //   header(1) + transport(1) + status_bar(1) = 3 rows
        //   margins: header-content(1) + channels-transport(1) + transport-status(1) = 3
        //   + piano_roll-channels margin(1) if piano_roll shown = +1
        let header_rows: u16 = 1;
        let footer_rows: u16 = 1 + SECTION_MARGIN + 1; // transport + margin + status_bar
        let margin_count = if show_piano_roll { 3 } else { 2 };
        // margins between: header↔content, [pr↔channels,] channels↔transport
        // (transport↔status margin is inside footer_rows)
        let fixed = header_rows + footer_rows + margin_count * SECTION_MARGIN;
        let available = rows.saturating_sub(fixed).max(1);

        let mut y = header_rows + SECTION_MARGIN;

        // Channels total = title bar + content (capped at needed rows)
        let channels_content = if show_piano_roll {
            // Give channels only what it needs, rest to piano roll
            channels_needed_rows.min(available.saturating_sub(TITLE_BAR_CELL_ROWS))
        } else {
            // No piano roll: channels gets all available (minus title bar)
            available.saturating_sub(TITLE_BAR_CELL_ROWS)
        };
        let channels_total = TITLE_BAR_CELL_ROWS + channels_content;

        // Piano roll (optional): gets remaining space after channels
        let (piano_roll_px, _pr_rows) = if show_piano_roll {
            let pr = available.saturating_sub(channels_total).max(1);
            let px = Rect::new(
                0.0,
                y as f32 * cell_h,
                cols as f32 * cell_w,
                pr as f32 * cell_h,
            );
            y += pr + SECTION_MARGIN;
            (Some(px), pr)
        } else {
            (None, 0)
        };

        // Title bar pixel rect (vertically centered within TITLE_BAR_CELL_ROWS)
        let tt_y = y as f32 * cell_h
            + (TITLE_BAR_CELL_ROWS as f32 - TITLE_BAR_HEIGHT) * cell_h / 2.0;
        let track_title_px = Rect::new(
            0.0,
            tt_y,
            cols as f32 * cell_w,
            TITLE_BAR_HEIGHT * cell_h,
        );

        let track_list = Rect::new(
            0.0,
            (y + TITLE_BAR_CELL_ROWS) as f32 * cell_h,
            cols as f32 * cell_w,
            channels_content as f32 * cell_h,
        );

        y += channels_total + SECTION_MARGIN;

        // Transport
        let transport = Rect::new(
            0.0,
            y as f32 * cell_h,
            cols as f32 * cell_w,
            1.0 * cell_h,
        );
        y += 1 + SECTION_MARGIN;

        // Status bar
        let status_bar = Rect::new(
            0.0,
            y as f32 * cell_h,
            cols as f32 * cell_w,
            1.0 * cell_h,
        );

        Layout {
            piano_roll_px,
            track_title_px,
            track_list,
            transport,
            status_bar,
        }
    }
}
