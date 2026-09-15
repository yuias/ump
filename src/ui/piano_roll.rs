//! Piano roll: scrolling note display with bar/beat grid, keyboard strip,
//! ruler (click-to-seek), and playhead. Supports horizontal (default) and
//! vertical (step-sequencer style) modes. Uses Renderer trait for native
//! sub-pixel drawing.

use crate::app::App;
use ump_playback::midi::event::NoteRect;
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::{dot_size, px};
use crate::ui::text::text_width;
use crate::ui::theme;

/// Render the piano roll area using the native renderer.
/// Dispatches to horizontal or vertical mode.
pub fn render_piano_roll(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    vertical: bool,
    hits: &mut HitMap,
) {
    if area.width < 20.0 || area.height < 10.0 || note_rects.is_empty() {
        return;
    }

    if vertical {
        render_vertical(renderer, area, app, note_rects, hits);
    } else {
        render_horizontal(renderer, area, app, note_rects, hits);
    }
}

/// How a note should be drawn, given the playhead position and mute state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteState {
    /// Muted channel: outline only, regardless of playback position.
    Muted,
    /// Currently sounding (`start <= tick < end`).
    Sounding,
    /// Already played (`end <= tick`).
    Finished,
    /// Not yet reached.
    Upcoming,
}

fn note_state(start_tick: u64, end_tick: u64, tick: u64, muted: bool) -> NoteState {
    if muted {
        NoteState::Muted
    } else if start_tick <= tick && tick < end_tick {
        NoteState::Sounding
    } else if end_tick <= tick {
        NoteState::Finished
    } else {
        NoteState::Upcoming
    }
}

/// Draw a single note rectangle per its `NoteState`. Finished notes on
/// channels 8-15 dither in `DIM` rather than their channel color: those
/// channels already dither their channel color when *upcoming*, and
/// dithering a dithered color a second time would be indistinguishable
/// from the background.
fn draw_note(renderer: &mut dyn Renderer, rect: Rect, channel: u8, state: NoteState, dot: f32) {
    match state {
        NoteState::Muted => {
            renderer.fill_rect(Rect::new(rect.x, rect.y, rect.width, dot), theme::DIM);
            renderer.fill_rect(Rect::new(rect.x, rect.bottom() - dot, rect.width, dot), theme::DIM);
            renderer.fill_rect(Rect::new(rect.x, rect.y, dot, rect.height), theme::DIM);
            renderer.fill_rect(Rect::new(rect.right() - dot, rect.y, dot, rect.height), theme::DIM);
        }
        NoteState::Sounding => {
            renderer.fill_rect(rect, theme::channel_color(channel));
            renderer.fill_rect(Rect::new(rect.x, rect.y, rect.width, dot), theme::TEXT);
        }
        NoteState::Finished => {
            let fg = if channel < 8 { theme::channel_color(channel) } else { theme::DIM };
            renderer.fill_dither(rect, fg, None);
        }
        NoteState::Upcoming => {
            theme::channel_fill(renderer, rect, channel, None);
        }
    }
}

/// Compute the visible key range and per-key slot size for a piano-roll axis.
/// If the full song range fits at >= `min_slot` px/key, shows it all. Otherwise
/// shows as many keys as fit, centered by default and shifted by `key_scroll`
/// (positive = toward higher keys), clamped so the visible range never leaves
/// `[song_lo, song_hi]`.
fn visible_key_range(song_lo: u8, song_hi: u8, extent: f32, min_slot: f32, key_scroll: i32) -> (u8, u8, f32) {
    let song_count = song_hi as i32 - song_lo as i32 + 1;
    let full_slot = extent / song_count as f32;
    if full_slot >= min_slot {
        return (song_lo, song_hi, full_slot);
    }

    let visible_count = ((extent / min_slot).floor() as i32).clamp(1, song_count);
    let slot = extent / visible_count as f32;
    let center_offset = (song_count - visible_count) / 2;
    let offset = (center_offset + key_scroll).clamp(0, song_count - visible_count);
    let lo = song_lo + offset as u8;
    let hi = lo + visible_count as u8 - 1;
    (lo, hi, slot)
}

/// Horizontal piano roll: time on X-axis, keys on Y-axis.
fn render_horizontal(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    hits: &mut HitMap,
) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let dot = dot_size(scale);

    let kb_w = (4.0 * cw).max(px(40.0, scale));
    let ruler_h = ch + 2.0 * px(2.0, scale);

    if area.width - kb_w < 10.0 || area.height - ruler_h < 10.0 {
        return;
    }

    let note_area = Rect::new(area.x + kb_w, area.y + ruler_h, area.width - kb_w, area.height - ruler_h);
    let kb_area = Rect::new(area.x, area.y + ruler_h, kb_w, area.height - ruler_h);
    let ruler_area = Rect::new(area.x, area.y, area.width, ruler_h);

    // Key range
    let song_lo = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let song_hi = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let min_slot = px(6.0, scale);
    let (lo, hi, slot_h) = visible_key_range(song_lo, song_hi, note_area.height, min_slot, app.key_scroll);

    // Pixels per tick (zoom) — scale by DPI, not font size, so changing the
    // font does not change zoom. Constants match the old cw-derived values
    // closely enough at a 14px font.
    let pixels_per_tick = 0.05 * app.zoom_level * (8.0 * scale) as f64;

    let current_tick = app.current_tick();

    // Visible tick range: playhead at 25% from the left
    let visible_ticks = note_area.width as f64 / pixels_per_tick;
    let playhead_offset = visible_ticks * 0.25;
    let view_start_tick = if current_tick as f64 > playhead_offset {
        (current_tick as f64 - playhead_offset) as u64
    } else {
        0
    };
    let view_end_tick = view_start_tick + visible_ticks as u64;

    // 1. Note area background + black-key rows
    renderer.fill_rect(note_area, theme::GROUND);
    for key in lo..=hi {
        if is_black_key(key) {
            let offset = (hi - key) as f32;
            let row = Rect::new(note_area.x, note_area.y + offset * slot_h, note_area.width, slot_h);
            renderer.fill_rect(row, theme::KEY_ROW_BLACK);
        }
    }

    // 2. Bar/beat grid
    let grid_end = view_end_tick.max(view_start_tick + 1);
    for (tick, is_bar, _) in app.bar_map.lines_in(view_start_tick, grid_end) {
        let x = note_area.x + ((tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
        if x < note_area.x || x > note_area.right() {
            continue;
        }
        if is_bar {
            renderer.fill_rect(Rect::new(x, area.y, dot, note_area.bottom() - area.y), theme::MEASURE);
        } else {
            let beat_px = app.bar_map.ticks_per_beat_at(tick) as f64 * pixels_per_tick;
            if beat_px >= min_slot as f64 {
                renderer.fill_rect(Rect::new(x, note_area.y, dot, note_area.height), theme::BEAT);
            }
        }
    }

    // 3. Ruler: background, bottom line, bar numbers
    renderer.fill_rect(ruler_area, theme::GROUND);
    renderer.fill_rect(Rect::new(ruler_area.x, ruler_area.bottom() - dot, ruler_area.width, dot), theme::MEASURE);
    let bar_num_min_spacing = text_width("000", cw) + px(6.0, scale);
    for (tick, is_bar, bar_no) in app.bar_map.lines_in(view_start_tick, grid_end) {
        if !is_bar {
            continue;
        }
        let x = note_area.x + ((tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
        if x < note_area.x || x > note_area.right() {
            continue;
        }
        let bar_px = app.bar_map.ticks_per_bar_at(tick) as f64 * pixels_per_tick;
        if bar_px < bar_num_min_spacing as f64 {
            continue;
        }
        let label = format!("{:03}", bar_no);
        let label_y = ruler_area.y + (ruler_h - ch) / 2.0;
        renderer.draw_text(x + px(3.0, scale), label_y, &label, theme::DIM, ch);
    }

    // 4. Notes
    let bar_h = (slot_h - dot).max(dot);

    // Binary search optimization: skip notes far before view
    let max_dur = app.total_ticks / 4;
    let scan_start = if view_start_tick > max_dur {
        note_rects.partition_point(|n| n.start_tick < view_start_tick - max_dur)
    } else {
        0
    };

    let muted_mask = app
        .shared
        .muted_channels
        .load(std::sync::atomic::Ordering::Relaxed);

    for note in &note_rects[scan_start..] {
        if note.start_tick > view_end_tick {
            break;
        }
        if note.end_tick < view_start_tick {
            continue;
        }
        if note.key < lo || note.key > hi {
            continue;
        }

        let flat_ch = note.port as u64 * 16 + note.channel as u64;
        let muted = muted_mask & (1u64 << flat_ch) != 0;

        let offset = (hi - note.key) as f32;
        let slot_y = note_area.y + offset * slot_h;
        let bar_y = slot_y + (slot_h - bar_h) / 2.0;

        let start_x_f = (note.start_tick as f64 - view_start_tick as f64) * pixels_per_tick;
        let end_x_f = (note.end_tick as f64 - view_start_tick as f64) * pixels_per_tick;

        let x0 = note_area.x + start_x_f.max(0.0) as f32;
        let x1 = note_area.x + (end_x_f as f32).min(note_area.width);

        if x0 >= note_area.right() || x1 <= note_area.x || x0 >= x1 {
            continue;
        }

        let rect = Rect::new(x0, bar_y, (x1 - x0).max(1.0), bar_h);
        let state = note_state(note.start_tick, note.end_tick, current_tick, muted);
        draw_note(renderer, rect, note.channel, state, dot);
    }

    // 5. Keyboard column
    for key in lo..=hi {
        let offset = (hi - key) as f32;
        let row = Rect::new(kb_area.x, kb_area.y + offset * slot_h, kb_area.width, slot_h);

        renderer.fill_rect(row, theme::KEY_WHITE);
        if is_black_key(key) {
            renderer.fill_rect(Rect::new(row.x, row.y, row.width * 0.6, row.height), theme::KEY_BLACK);
        }
        // Octave boundary marker under B and E (the two single-semitone gaps).
        if matches!(key % 12, 11 | 4) {
            renderer.fill_rect(Rect::new(row.x, row.bottom() - dot, row.width, dot), theme::DIM);
        }

        if key % 12 == 0 && slot_h >= ch * 0.6 {
            let font_size = ch.min(slot_h * 1.2);
            let (note_str, octave) = key_name_parts(key);
            let label = format!("{}{}", note_str, octave);
            let label_w = text_width(&label, cw) * (font_size / ch);
            let label_x = kb_area.right() - px(2.0, scale) - label_w;
            let label_y = row.y + (slot_h - font_size) / 2.0;
            renderer.draw_text(label_x, label_y, &label, theme::GROUND, font_size);
        }
    }
    renderer.fill_rect(Rect::new(note_area.x - dot, note_area.y, dot, note_area.height), theme::FRAME);

    // 6. Playhead + ruler marker
    let playhead_x = note_area.x + ((current_tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
    if playhead_x >= area.x && playhead_x <= area.right() {
        let pw = 2.0 * dot;
        renderer.fill_rect(Rect::new(playhead_x - pw / 2.0, area.y, pw, note_area.bottom() - area.y), theme::PLAYHEAD);

        const MARKER_WIDTHS: [f32; 4] = [7.0, 5.0, 3.0, 1.0];
        let marker_row_h = ruler_h / MARKER_WIDTHS.len() as f32;
        for (i, &w) in MARKER_WIDTHS.iter().enumerate() {
            let rw = w * dot;
            let ry = area.y + i as f32 * marker_row_h;
            renderer.fill_rect(Rect::new(playhead_x - rw / 2.0, ry, rw, marker_row_h), theme::PLAYHEAD);
        }
    }

    // Hit regions: broad wheel target first, then the more specific ruler.
    hits.push(
        Rect::new(area.x, note_area.y, area.width, note_area.height),
        HitAction::PianoRoll,
    );
    hits.push(
        Rect::new(note_area.x, area.y, note_area.width, ruler_h),
        HitAction::Ruler {
            axis_origin: note_area.x,
            px_per_tick: pixels_per_tick,
            view_start_tick,
            vertical: false,
        },
    );
}

/// Vertical piano roll: keys on X-axis (left=low, right=high), time on Y-axis (flows downward).
/// Playhead is a horizontal line at 75% from top.
fn render_vertical(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    hits: &mut HitMap,
) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let dot = dot_size(scale);

    let kb_h = (2.0 * ch).max(px(28.0, scale));
    let ruler_w = text_width("000", cw) + 2.0 * px(3.0, scale);

    if area.width - ruler_w < 10.0 || area.height - kb_h < 10.0 {
        return;
    }

    let note_area = Rect::new(area.x + ruler_w, area.y + kb_h, area.width - ruler_w, area.height - kb_h);
    let kb_area = Rect::new(area.x + ruler_w, area.y, area.width - ruler_w, kb_h);
    let ruler_area = Rect::new(area.x, area.y, ruler_w, area.height);

    // Key range
    let song_lo = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let song_hi = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let min_slot = px(6.0, scale);
    let (lo, hi, slot_w) = visible_key_range(song_lo, song_hi, note_area.width, min_slot, app.key_scroll);

    // Pixels per tick (zoom) — scale by DPI, not font size, so changing the
    // font does not change zoom. Constants match the old ch-derived values
    // closely enough at a 14px font.
    let pixels_per_tick = 0.05 * app.zoom_level * (16.0 * scale) as f64;

    let current_tick = app.current_tick();

    // Visible tick range: playhead at 75% from top (notes flow down toward it)
    let visible_ticks = note_area.height as f64 / pixels_per_tick;
    let playhead_offset = visible_ticks * 0.75;
    let view_start_tick = if current_tick as f64 > playhead_offset {
        (current_tick as f64 - playhead_offset) as u64
    } else {
        0
    };
    let view_end_tick = view_start_tick + visible_ticks as u64;

    // 1. Note area background + black-key columns
    renderer.fill_rect(note_area, theme::GROUND);
    for key in lo..=hi {
        if is_black_key(key) {
            let offset = (key - lo) as f32;
            let col = Rect::new(note_area.x + offset * slot_w, note_area.y, slot_w, note_area.height);
            renderer.fill_rect(col, theme::KEY_ROW_BLACK);
        }
    }

    // 2. Bar/beat grid (horizontal lines, time flows down)
    let grid_end = view_end_tick.max(view_start_tick + 1);
    for (tick, is_bar, _) in app.bar_map.lines_in(view_start_tick, grid_end) {
        let y = note_area.y + ((tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
        if y < note_area.y || y > note_area.bottom() {
            continue;
        }
        if is_bar {
            renderer.fill_rect(Rect::new(area.x, y, note_area.right() - area.x, dot), theme::MEASURE);
        } else {
            let beat_px = app.bar_map.ticks_per_beat_at(tick) as f64 * pixels_per_tick;
            if beat_px >= min_slot as f64 {
                renderer.fill_rect(Rect::new(note_area.x, y, note_area.width, dot), theme::BEAT);
            }
        }
    }

    // 3. Ruler: background, right edge line, bar numbers
    renderer.fill_rect(ruler_area, theme::GROUND);
    renderer.fill_rect(Rect::new(ruler_area.right() - dot, ruler_area.y, dot, ruler_area.height), theme::MEASURE);
    let bar_num_min_spacing = ch + px(6.0, scale);
    for (tick, is_bar, bar_no) in app.bar_map.lines_in(view_start_tick, grid_end) {
        if !is_bar {
            continue;
        }
        let y = note_area.y + ((tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
        if y < note_area.y || y > note_area.bottom() {
            continue;
        }
        let bar_px = app.bar_map.ticks_per_bar_at(tick) as f64 * pixels_per_tick;
        if bar_px < bar_num_min_spacing as f64 {
            continue;
        }
        let label = format!("{:03}", bar_no);
        renderer.draw_text(ruler_area.x, y + px(3.0, scale), &label, theme::DIM, ch);
    }

    // 4. Notes
    let bar_w = (slot_w - dot).max(dot);

    let max_dur = app.total_ticks / 4;
    let scan_start = if view_start_tick > max_dur {
        note_rects.partition_point(|n| n.start_tick < view_start_tick - max_dur)
    } else {
        0
    };

    let muted_mask = app
        .shared
        .muted_channels
        .load(std::sync::atomic::Ordering::Relaxed);

    for note in &note_rects[scan_start..] {
        if note.start_tick > view_end_tick {
            break;
        }
        if note.end_tick < view_start_tick {
            continue;
        }
        if note.key < lo || note.key > hi {
            continue;
        }

        let flat_ch = note.port as u64 * 16 + note.channel as u64;
        let muted = muted_mask & (1u64 << flat_ch) != 0;

        let key_offset = (note.key - lo) as f32;
        let slot_x = note_area.x + key_offset * slot_w;
        let bar_x = slot_x + (slot_w - bar_w) / 2.0;

        let start_y_f = (note.start_tick as f64 - view_start_tick as f64) * pixels_per_tick;
        let end_y_f = (note.end_tick as f64 - view_start_tick as f64) * pixels_per_tick;

        let y0 = note_area.y + start_y_f.max(0.0) as f32;
        let y1 = note_area.y + (end_y_f as f32).min(note_area.height);

        if y0 >= note_area.bottom() || y1 <= note_area.y || y0 >= y1 {
            continue;
        }

        let rect = Rect::new(bar_x, y0, bar_w, (y1 - y0).max(1.0));
        let state = note_state(note.start_tick, note.end_tick, current_tick, muted);
        draw_note(renderer, rect, note.channel, state, dot);
    }

    // 5. Keyboard strip
    let label_min_slot = text_width("C-1", cw) + px(2.0, scale);
    for key in lo..=hi {
        let offset = (key - lo) as f32;
        let col = Rect::new(kb_area.x + offset * slot_w, kb_area.y, slot_w, kb_area.height);

        renderer.fill_rect(col, theme::KEY_WHITE);
        if is_black_key(key) {
            renderer.fill_rect(Rect::new(col.x, col.y, col.width, col.height * 0.6), theme::KEY_BLACK);
        }
        // Octave boundary marker right of B and E (the two single-semitone gaps).
        if matches!(key % 12, 11 | 4) {
            renderer.fill_rect(Rect::new(col.right() - dot, col.y, dot, col.height), theme::DIM);
        }

        if key % 12 == 0 && slot_w >= label_min_slot {
            let font_size = ch.min(slot_w * 1.2);
            let (note_str, octave) = key_name_parts(key);
            let label = format!("{}{}", note_str, octave);
            let label_w = text_width(&label, cw) * (font_size / ch);
            let label_x = col.x + (slot_w - label_w) / 2.0;
            let label_y = kb_area.y + px(2.0, scale);
            renderer.draw_text(label_x, label_y, &label, theme::GROUND, font_size);
        }
    }
    renderer.fill_rect(Rect::new(note_area.x, note_area.y - dot, note_area.width, dot), theme::FRAME);

    // 6. Playhead + ruler marker
    let playhead_y = note_area.y + ((current_tick as f64 - view_start_tick as f64) * pixels_per_tick) as f32;
    if playhead_y >= area.y && playhead_y <= area.bottom() {
        let ph = 2.0 * dot;
        renderer.fill_rect(Rect::new(area.x, playhead_y - ph / 2.0, note_area.right() - area.x, ph), theme::PLAYHEAD);

        const MARKER_HEIGHTS: [f32; 4] = [7.0, 5.0, 3.0, 1.0];
        let marker_col_w = ruler_w / MARKER_HEIGHTS.len() as f32;
        for (i, &h) in MARKER_HEIGHTS.iter().enumerate() {
            let rh = h * dot;
            let rx = ruler_area.x + i as f32 * marker_col_w;
            renderer.fill_rect(Rect::new(rx, playhead_y - rh / 2.0, marker_col_w, rh), theme::PLAYHEAD);
        }
    }

    // Hit regions: broad wheel target first, then the more specific ruler.
    hits.push(
        Rect::new(note_area.x, area.y, note_area.width, area.height),
        HitAction::PianoRoll,
    );
    hits.push(
        Rect::new(area.x, note_area.y, ruler_w, note_area.height),
        HitAction::Ruler {
            axis_origin: note_area.y,
            px_per_tick: pixels_per_tick,
            view_start_tick,
            vertical: true,
        },
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_key_range_shows_full_range_when_it_fits() {
        let (lo, hi, slot) = visible_key_range(40, 60, 200.0, 6.0, 0);
        assert_eq!((lo, hi), (40, 60));
        assert!((slot - 200.0 / 21.0).abs() < 0.001);
    }

    #[test]
    fn visible_key_range_centers_when_it_does_not_fit() {
        // 100 keys at min_slot=6 needs 600px; only 60px available -> 10 keys fit, centered.
        let (lo, hi, slot) = visible_key_range(0, 99, 60.0, 6.0, 0);
        assert_eq!(hi - lo, 9);
        assert_eq!(lo, 45); // (100-10)/2
        assert!((slot - 6.0).abs() < 0.001);
    }

    #[test]
    fn visible_key_range_scrolls_and_clamps_at_high_end() {
        let (lo, hi, _) = visible_key_range(0, 99, 60.0, 6.0, 1000);
        assert_eq!(hi, 99); // clamped to song_hi
        assert_eq!(lo, 90);
    }

    #[test]
    fn visible_key_range_scrolls_and_clamps_at_low_end() {
        let (lo, hi, _) = visible_key_range(0, 99, 60.0, 6.0, -1000);
        assert_eq!(lo, 0); // clamped to song_lo
        assert_eq!(hi, 9);
    }

    #[test]
    fn note_state_muted_overrides_playback_position() {
        assert_eq!(note_state(0, 100, 50, true), NoteState::Muted);
    }

    #[test]
    fn note_state_classifies_by_tick() {
        assert_eq!(note_state(100, 200, 50, false), NoteState::Upcoming);
        assert_eq!(note_state(100, 200, 100, false), NoteState::Sounding);
        assert_eq!(note_state(100, 200, 199, false), NoteState::Sounding);
        assert_eq!(note_state(100, 200, 200, false), NoteState::Finished);
    }
}
