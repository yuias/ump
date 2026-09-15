//! PC-98-style color palette: theme tokens and per-channel MIDI colors.

use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;

/// Base ground (window background).
pub const GROUND: Color = Color::rgb(0x00, 0x00, 0x00);
/// Header bar background.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const BAR_BG: Color = Color::rgb(0x22, 0x33, 0xCC);
/// Header bar foreground.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const BAR_FG: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// Panel frame line color.
pub const FRAME: Color = Color::rgb(0x22, 0xCC, 0xCC);
/// Panel title strip background.
pub const TITLE_BG: Color = Color::rgb(0x22, 0xCC, 0xCC);
/// Panel title strip text color.
pub const TITLE_FG: Color = Color::rgb(0x00, 0x00, 0x00);
/// Primary body text color.
pub const TEXT: Color = Color::rgb(0xEE, 0xEE, 0xEE);
/// Dimmed/secondary text color.
pub const DIM: Color = Color::rgb(0x88, 0x88, 0x88);
/// Highlight/emphasis color.
pub const ACCENT: Color = Color::rgb(0xEE, 0xDD, 0x22);
/// Playback cursor / muted indicator color.
pub const PLAYHEAD: Color = Color::rgb(0xEE, 0x22, 0x33);
/// Selected row background.
pub const SELECTED_BG: Color = Color::rgb(0x22, 0x33, 0xCC);
/// Recessed well background (e.g. empty progress track).
pub const WELL: Color = Color::rgb(0x16, 0x16, 0x16);
/// Positive/playing status color.
pub const OK: Color = Color::rgb(0x22, 0xCC, 0x44);
/// Piano roll beat gridline color.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const BEAT: Color = Color::rgb(0x16, 0x16, 0x3A);
/// Piano roll measure gridline color.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const MEASURE: Color = Color::rgb(0x22, 0x33, 0xCC);
/// Piano roll black-key row background.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const KEY_ROW_BLACK: Color = Color::rgb(0x07, 0x07, 0x18);
/// Piano key (white key) color.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const KEY_WHITE: Color = Color::rgb(0xCC, 0xCC, 0xCC);
/// Piano key (black key) color.
#[allow(dead_code)] // not drawn until the component restyle lands
pub const KEY_BLACK: Color = Color::rgb(0x11, 0x11, 0x11);
/// Function-key bar background.
pub const FKEY_BG: Color = Color::rgb(0xEE, 0xEE, 0xEE);
/// Function-key bar text color.
pub const FKEY_FG: Color = Color::rgb(0x00, 0x00, 0x00);

/// Base colors for MIDI channels 0-7. Channels 8-15 reuse these via dithering
/// (see `channel_fill`) since 8 hues cannot distinguish 16 channels on their own.
const CHANNEL_COLORS: [Color; 8] = [
    Color::rgb(0xEE, 0x22, 0x33),
    Color::rgb(0x22, 0xCC, 0xCC),
    Color::rgb(0x22, 0xCC, 0x44),
    Color::rgb(0xEE, 0xDD, 0x22),
    Color::rgb(0xCC, 0x33, 0xCC),
    Color::rgb(0x55, 0x66, 0xFF),
    Color::rgb(0xEE, 0xEE, 0xEE),
    Color::rgb(0x99, 0x99, 0x99),
];

/// Returns a distinct color for each MIDI channel (0-15), cycling through 8 base hues.
pub fn channel_color(channel: u8) -> Color {
    CHANNEL_COLORS[(channel % 8) as usize]
}

/// Fill `rect` with the color for MIDI channel `ch`. Channels 0-7 get a solid fill;
/// channels 8-15 reuse the same 8 hues but dithered, so the checker pattern is the
/// second visual axis that distinguishes them from their low counterpart.
pub fn channel_fill(renderer: &mut dyn Renderer, rect: Rect, ch: u8, bg: Option<Color>) {
    let color = channel_color(ch);
    if ch < 8 {
        renderer.fill_rect(rect, color);
    } else {
        renderer.fill_dither(rect, color, bg);
    }
}
