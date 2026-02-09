//! Piano roll: scrolling note display with playhead.
//! Supports horizontal (default) and vertical (step-sequencer style) modes.
//! Uses Renderer trait for native sub-pixel drawing.

use crate::app::App;
use crate::midi::event::NoteRect;
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::ui::theme;

/// Render the piano roll area using the native renderer.
/// Dispatches to horizontal or vertical mode.
pub fn render_piano_roll(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    vertical: bool,
) {
    if area.width < 20.0 || area.height < 10.0 || note_rects.is_empty() {
        return;
    }

    if vertical {
        render_vertical(renderer, area, app, note_rects);
    } else {
        render_horizontal(renderer, area, app, note_rects);
    }
}

/// Horizontal piano roll: time on X-axis, keys on Y-axis.
fn render_horizontal(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
) {
    let (cw, ch) = renderer.cell_size();

    // Inner area
    let label_width_px = 4.0 * cw;
    let inner_x = area.x + label_width_px;
    let inner_y = area.y;
    let inner_w = area.width - label_width_px;
    let inner_h = area.height;

    if inner_w < 10.0 || inner_h < 10.0 {
        return;
    }

    // Determine key range from note data
    let min_key = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let max_key = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let key_range = (max_key - min_key + 1) as f32;

    // Pixels per tick (zoom) — scale by cell width to match old cell-based units
    let pixels_per_tick = 0.05 * app.zoom_level * cw as f64;

    // Current playback tick
    let current_tick = app.current_tick();

    // Visible tick range: center on current_tick
    let visible_ticks = inner_w as f64 / pixels_per_tick;
    let playhead_offset = visible_ticks * 0.25;
    let view_start_tick = if current_tick as f64 > playhead_offset {
        (current_tick as f64 - playhead_offset) as u64
    } else {
        0
    };
    let view_end_tick = view_start_tick + visible_ticks as u64;

    // Draw piano key labels
    let label_step = if key_range <= inner_h / ch {
        1.0
    } else {
        (key_range / (inner_h / ch)).ceil()
    };

    // Slot height per key (pixel)
    let note_h = (inner_h / key_range).max(1.0);

    // Label font size is the visual reference for bar height
    let label_font_size = ch * 0.8;
    // Bar height: 80% of label font size, capped at slot height to prevent overlap
    let bar_h = (label_font_size * 0.8).min(note_h).max(1.0);

    let mut key = min_key;
    while key <= max_key {
        let offset = (max_key - key) as f32;
        let slot_y = inner_y + offset * note_h;
        // Center label within slot (clamp offset to 0 when slot < font)
        let label_y = slot_y + (note_h - label_font_size).max(0.0) / 2.0;

        let label_color = if is_black_key(key) {
            Color::rgb(100, 100, 100)
        } else {
            theme::HEADER_FG
        };

        let (note_str, octave) = key_name_parts(key);
        let label = format!("{}{}", note_str, octave);
        renderer.draw_text(area.x + 2.0, label_y, &label, label_color, label_font_size);

        key += label_step as u8;
    }

    // Separator line
    renderer.draw_vline(
        area.x + label_width_px,
        inner_y,
        inner_y + inner_h,
        theme::BORDER_COLOR,
        1.0,
    );

    // Binary search optimization: skip notes far before view
    let max_dur = app.total_ticks / 4;
    let scan_start = if view_start_tick > max_dur {
        note_rects.partition_point(|n| n.start_tick < view_start_tick - max_dur)
    } else {
        0
    };

    // Precompute muted channel mask (u64)
    let muted_mask = app
        .shared
        .muted_channels
        .load(std::sync::atomic::Ordering::Relaxed);

    // Draw note rectangles
    for note in &note_rects[scan_start..] {
        if note.start_tick > view_end_tick {
            break;
        }
        if note.end_tick < view_start_tick {
            continue;
        }
        if note.key < min_key || note.key > max_key {
            continue;
        }

        let flat_ch = note.port as u64 * 16 + note.channel as u64;
        let muted = muted_mask & (1u64 << flat_ch) != 0;
        let color = if muted {
            theme::MUTED_COLOR
        } else {
            theme::channel_color(note.channel)
        };

        let offset = (max_key - note.key) as f32;
        let slot_y = inner_y + offset * note_h;
        // Center bar within slot, matching label-derived bar_h
        let bar_y = slot_y + (note_h - bar_h) / 2.0;

        let start_x_f =
            (note.start_tick as f64 - view_start_tick as f64) * pixels_per_tick;
        let end_x_f =
            (note.end_tick as f64 - view_start_tick as f64) * pixels_per_tick;

        let x0 = inner_x + start_x_f.max(0.0) as f32;
        let x1 = inner_x + (end_x_f as f32).min(inner_w);

        if x0 >= inner_x + inner_w || x1 <= inner_x || x0 >= x1 {
            continue;
        }

        renderer.fill_rect(
            Rect::new(x0, bar_y, (x1 - x0).max(1.0), bar_h),
            color,
        );
    }

    // Draw playhead
    let playhead_x_f = (current_tick as f64 - view_start_tick as f64) * pixels_per_tick;
    let playhead_x = inner_x + playhead_x_f as f32;
    if playhead_x >= inner_x && playhead_x <= inner_x + inner_w {
        renderer.draw_vline(playhead_x, inner_y, inner_y + inner_h, theme::PLAYHEAD_COLOR, 1.5);
    }
}

/// Vertical piano roll: keys on X-axis (left=low, right=high), time on Y-axis (flows downward).
/// Playhead is a horizontal line at 75% from top.
fn render_vertical(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
) {
    let (cw, ch) = renderer.cell_size();

    // Label strip along top (1 row of key names)
    let label_height_px = ch * 1.2;
    let inner_x = area.x;
    let inner_y = area.y + label_height_px;
    let inner_w = area.width;
    let inner_h = area.height - label_height_px;

    if inner_w < 10.0 || inner_h < 10.0 {
        return;
    }

    // Determine key range from note data
    let min_key = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let max_key = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let key_range = (max_key - min_key + 1) as f32;

    // Slot width per key (pixel)
    let note_w = (inner_w / key_range).max(1.0);

    // Draw key labels along top edge
    let label_font_size = ch * 0.7;
    let label_step = if key_range <= inner_w / (cw * 3.0) {
        1.0
    } else {
        (key_range / (inner_w / (cw * 3.0))).ceil()
    };

    let label_base_y = area.y + (label_height_px - label_font_size) / 2.0;
    let mut key = min_key;
    while key <= max_key {
        let offset = (key - min_key) as f32;
        let slot_x = inner_x + offset * note_w;

        let label_color = if is_black_key(key) {
            Color::rgb(100, 100, 100)
        } else {
            theme::HEADER_FG
        };

        let (note_str, octave) = key_name_parts(key);
        let label = format!("{}{}", note_str, octave);
        renderer.draw_text(slot_x + 1.0, label_base_y, &label, label_color, label_font_size);

        key += label_step as u8;
    }

    // Separator line below labels
    renderer.draw_hline(
        inner_x,
        inner_x + inner_w,
        inner_y,
        theme::BORDER_COLOR,
        1.0,
    );

    // Pixels per tick (zoom) — scale by cell height
    let pixels_per_tick = 0.05 * app.zoom_level * ch as f64;

    // Current playback tick
    let current_tick = app.current_tick();

    // Visible tick range: playhead at 75% from top (notes flow down toward it)
    let visible_ticks = inner_h as f64 / pixels_per_tick;
    let playhead_fraction = 0.75;
    let playhead_offset = visible_ticks * playhead_fraction;
    let view_start_tick = if current_tick as f64 > playhead_offset {
        (current_tick as f64 - playhead_offset) as u64
    } else {
        0
    };
    let view_end_tick = view_start_tick + visible_ticks as u64;

    // Bar width: 80% of slot width, capped
    let bar_w = (note_w * 0.8).max(1.0);

    // Binary search optimization: skip notes far before view
    let max_dur = app.total_ticks / 4;
    let scan_start = if view_start_tick > max_dur {
        note_rects.partition_point(|n| n.start_tick < view_start_tick - max_dur)
    } else {
        0
    };

    // Precompute muted channel mask (u64)
    let muted_mask = app
        .shared
        .muted_channels
        .load(std::sync::atomic::Ordering::Relaxed);

    // Draw note rectangles
    for note in &note_rects[scan_start..] {
        if note.start_tick > view_end_tick {
            break;
        }
        if note.end_tick < view_start_tick {
            continue;
        }
        if note.key < min_key || note.key > max_key {
            continue;
        }

        let flat_ch = note.port as u64 * 16 + note.channel as u64;
        let muted = muted_mask & (1u64 << flat_ch) != 0;
        let color = if muted {
            theme::MUTED_COLOR
        } else {
            theme::channel_color(note.channel)
        };

        let key_offset = (note.key - min_key) as f32;
        let slot_x = inner_x + key_offset * note_w;
        // Center bar within slot
        let bar_x = slot_x + (note_w - bar_w) / 2.0;

        // Y position: higher tick = further down (time flows top→bottom)
        let start_y_f =
            (note.start_tick as f64 - view_start_tick as f64) * pixels_per_tick;
        let end_y_f =
            (note.end_tick as f64 - view_start_tick as f64) * pixels_per_tick;

        let y0 = inner_y + start_y_f.max(0.0) as f32;
        let y1 = inner_y + (end_y_f as f32).min(inner_h);

        if y0 >= inner_y + inner_h || y1 <= inner_y || y0 >= y1 {
            continue;
        }

        renderer.fill_rect(
            Rect::new(bar_x, y0, bar_w, (y1 - y0).max(1.0)),
            color,
        );
    }

    // Draw playhead (horizontal line)
    let playhead_y_f = (current_tick as f64 - view_start_tick as f64) * pixels_per_tick;
    let playhead_y = inner_y + playhead_y_f as f32;
    if playhead_y >= inner_y && playhead_y <= inner_y + inner_h {
        renderer.draw_hline(inner_x, inner_x + inner_w, playhead_y, theme::PLAYHEAD_COLOR, 1.5);
    }
}

/// Note name parts without allocation.
pub(crate) fn key_name_parts(key: u8) -> (&'static str, i8) {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (key as i8 / 12) - 1;
    let note = (key % 12) as usize;
    (NAMES[note], octave)
}

/// Check if a MIDI key is a black key.
fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
