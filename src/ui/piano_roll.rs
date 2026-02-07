//! Piano roll: horizontal scrolling note display with playhead.
//! Uses Renderer trait for native sub-pixel drawing.

use crate::app::App;
use crate::midi::event::NoteRect;
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::ui::layout::TITLE_BAR_HEIGHT;
use crate::ui::theme;

/// Render the piano roll area using the native renderer.
pub fn render_piano_roll(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
) {
    if area.width < 20.0 || area.height < 10.0 || note_rects.is_empty() {
        return;
    }

    let (cw, ch) = renderer.cell_size();

    // Clear piano roll background
    renderer.fill_rect(area, crate::renderer::types::BG_COLOR);

    // Title bar (2 cell heights, full width, larger font with padding)
    let title_h = ch * TITLE_BAR_HEIGHT;
    let title_bar = Rect::new(area.x, area.y, area.width, title_h);
    renderer.fill_rect(title_bar, theme::TITLE_BAR_BG);
    let font_size = ch * 1.02;
    let text_y = area.y + (title_h - font_size) / 2.0;
    renderer.draw_text(area.x + cw, text_y, "Piano Roll", theme::HEADER_FG, font_size);

    // Inner area (below title bar)
    let label_width_px = 4.0 * cw;
    let inner_x = area.x + label_width_px;
    let inner_y = area.y + title_h;
    let inner_w = area.width - label_width_px;
    let inner_h = area.height - title_h;

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

    // Precompute muted channel mask
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

        let muted = muted_mask & (1 << note.channel) != 0;
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

/// Note name parts without allocation.
fn key_name_parts(key: u8) -> (&'static str, i8) {
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
