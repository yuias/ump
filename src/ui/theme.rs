//! Color palette for 16 MIDI channels.

use crate::renderer::types::Color;

/// Returns a distinct color for each MIDI channel (0-15).
pub fn channel_color(channel: u8) -> Color {
    match channel {
        0 => Color::rgb(255, 100, 100),   // Red
        1 => Color::rgb(100, 200, 255),   // Sky blue
        2 => Color::rgb(100, 255, 100),   // Green
        3 => Color::rgb(255, 200, 50),    // Orange
        4 => Color::rgb(200, 100, 255),   // Purple
        5 => Color::rgb(255, 150, 200),   // Pink
        6 => Color::rgb(100, 255, 200),   // Teal
        7 => Color::rgb(255, 255, 100),   // Yellow
        8 => Color::rgb(150, 150, 255),   // Periwinkle
        9 => Color::rgb(255, 180, 100),   // Peach (drums)
        10 => Color::rgb(100, 200, 150),  // Sage
        11 => Color::rgb(200, 150, 100),  // Tan
        12 => Color::rgb(150, 255, 150),  // Light green
        13 => Color::rgb(200, 200, 255),  // Lavender
        14 => Color::rgb(255, 200, 200),  // Salmon
        15 => Color::rgb(200, 255, 255),  // Cyan
        _ => Color::rgb(255, 255, 255),
    }
}

/// Border/header style colors.
pub const BORDER_COLOR: Color = Color::rgb(80, 80, 100);
pub const HEADER_FG: Color = Color::rgb(200, 200, 220);
pub const HEADER_DIM: Color = Color::rgb(120, 120, 140);
pub const PLAYHEAD_COLOR: Color = Color::rgb(255, 255, 255);
pub const PROGRESS_FILLED: Color = Color::rgb(100, 180, 255);
pub const PROGRESS_EMPTY: Color = Color::rgb(50, 50, 70);
pub const MUTED_COLOR: Color = Color::rgb(80, 80, 80);
pub const SELECTED_BG: Color = Color::rgb(40, 40, 60);
