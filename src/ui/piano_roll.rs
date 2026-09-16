//! Piano roll: scrolling note display with bar/beat grid, keyboard strip,
//! ruler (click-to-seek), and playhead. Supports horizontal (default) and
//! vertical (step-sequencer style) modes. Uses Renderer trait for native
//! sub-pixel drawing. In horizontal and vertical-`Down` mode the playhead
//! sits on the note area's edge adjacent to the keyboard, so notes travel
//! toward the keys and sound as they cross it; vertical-`Up` keeps the
//! playhead at 75% of the note area for its tracker-style history view.

use crate::app::{App, VerticalFlow};
use ump_playback::midi::event::NoteRect;
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::{dot_size, px};
use crate::ui::text::text_width;
use crate::ui::theme;

/// Render the piano roll area using the native renderer.
/// Dispatches to horizontal or vertical mode.
/// Returns the effective (post-clamp) key_scroll for the caller to write back
/// to `app.key_scroll`, or `None` when the area is too small to render (no
/// scroll state to update). See `visible_key_range` for what "effective" means.
pub fn render_piano_roll(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    vertical: bool,
    hits: &mut HitMap,
) -> Option<i32> {
    if area.width < 20.0 || area.height < 10.0 || note_rects.is_empty() {
        return None;
    }

    if vertical {
        render_vertical(renderer, area, app, note_rects, hits)
    } else {
        render_horizontal(renderer, area, app, note_rects, hits)
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

/// For each MIDI key, the channel of the note sounding on it at `tick`
/// (`start_tick <= tick < end_tick`, not muted), or `None` if silent. When
/// several notes on the same key overlap at `tick` (e.g. a re-triggered
/// note), the one with the latest `start_tick` wins; ties go to the lower
/// channel. `notes` must be sorted by `start_tick` (as produced by the
/// loader) so scanning can stop at the first note starting after `tick`.
fn sounding_keys(notes: &[NoteRect], scan_start: usize, tick: u64, muted_mask: u64) -> [Option<u8>; 128] {
    let mut latest_start = [0u64; 128];
    let mut result = [None; 128];

    for note in &notes[scan_start..] {
        if note.start_tick > tick {
            break;
        }
        if note.end_tick <= tick {
            continue;
        }
        let flat_ch = note.port as u64 * 16 + note.channel as u64;
        if muted_mask & (1u64 << flat_ch) != 0 {
            continue;
        }

        let key = note.key as usize;
        let wins = match result[key] {
            None => true,
            Some(existing_ch) => {
                note.start_tick > latest_start[key]
                    || (note.start_tick == latest_start[key] && note.channel < existing_ch)
            }
        };
        if wins {
            latest_start[key] = note.start_tick;
            result[key] = Some(note.channel);
        }
    }

    result
}

/// Compute the visible key range and per-key slot size for a piano-roll axis.
/// If the full song range fits at >= `min_slot` px/key, shows it all. Otherwise
/// shows as many keys as fit, centered by default and shifted by `key_scroll`
/// (positive = toward higher keys), clamped so the visible range never leaves
/// `[song_lo, song_hi]`. The last element of the tuple is the *effective*
/// key_scroll (the raw value after clamping), which the caller writes back to
/// `app.key_scroll` so reversing the wheel takes effect immediately instead of
/// first having to unwind whatever overshoot was clamped away; it's 0 when the
/// whole range fits (there's nothing to scroll).
fn visible_key_range(song_lo: u8, song_hi: u8, extent: f32, min_slot: f32, key_scroll: i32) -> (u8, u8, f32, i32) {
    let song_count = song_hi as i32 - song_lo as i32 + 1;
    let full_slot = extent / song_count as f32;
    if full_slot >= min_slot {
        return (song_lo, song_hi, full_slot, 0);
    }

    let visible_count = ((extent / min_slot).floor() as i32).clamp(1, song_count);
    let slot = extent / visible_count as f32;
    let center_offset = (song_count - visible_count) / 2;
    let offset = (center_offset + key_scroll).clamp(0, song_count - visible_count);
    let lo = song_lo + offset as u8;
    let hi = lo + visible_count as u8 - 1;
    (lo, hi, slot, offset - center_offset)
}

/// Horizontal piano roll: time on X-axis, keys on Y-axis. The playhead sits
/// at the note area's left edge (`x = note_area.x`, against the keyboard
/// column); time increases to the right, so the visible range is
/// `[current_tick, current_tick + note_area.width / px_per_tick]` and a
/// sounding note that started earlier shows only its remaining part.
fn render_horizontal(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    hits: &mut HitMap,
) -> Option<i32> {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let dot = dot_size(scale);

    let kb_w = (4.0 * cw).max(px(40.0, scale));
    let ruler_h = ch + 2.0 * px(2.0, scale);

    if area.width - kb_w < 10.0 || area.height - ruler_h < 10.0 {
        return None;
    }

    let note_area = Rect::new(area.x + kb_w, area.y + ruler_h, area.width - kb_w, area.height - ruler_h);
    let kb_area = Rect::new(area.x, area.y + ruler_h, kb_w, area.height - ruler_h);
    let ruler_area = Rect::new(area.x, area.y, area.width, ruler_h);

    // Key range
    let song_lo = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let song_hi = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let min_slot = px(6.0, scale);
    let (lo, hi, slot_h, effective_scroll) = visible_key_range(song_lo, song_hi, note_area.height, min_slot, app.key_scroll);

    // Pixels per tick (zoom) — scale by DPI, not font size, so changing the
    // font does not change zoom. Constants match the old cw-derived values
    // closely enough at a 14px font.
    let pixels_per_tick = 0.05 * app.zoom_level * (8.0 * scale) as f64;

    let current_tick = app.current_tick();
    let axis = TimeAxis::new(note_area.x, current_tick, pixels_per_tick);
    let (view_start_tick, view_end_tick) = axis.visible_range(note_area.x, note_area.right());

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
        let x = axis.pos(tick);
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
        let x = axis.pos(tick);
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

        let x0 = axis.pos(note.start_tick).max(note_area.x);
        let x1 = axis.pos(note.end_tick).min(note_area.right());

        if x0 >= note_area.right() || x1 <= note_area.x || x0 >= x1 {
            continue;
        }

        let rect = Rect::new(x0, bar_y, (x1 - x0).max(1.0), bar_h);
        let state = note_state(note.start_tick, note.end_tick, current_tick, muted);
        draw_note(renderer, rect, note.channel, state, dot);
    }

    // 5. Keyboard column
    let sounding = sounding_keys(note_rects, scan_start, current_tick, muted_mask);
    for key in lo..=hi {
        let offset = (hi - key) as f32;
        let row = Rect::new(kb_area.x, kb_area.y + offset * slot_h, kb_area.width, slot_h);

        renderer.fill_rect(row, theme::KEY_WHITE);
        if is_black_key(key) {
            renderer.fill_rect(Rect::new(row.x, row.y, row.width * 0.6, row.height), theme::KEY_BLACK);
        }
        if let Some(channel) = sounding[key as usize] {
            if is_black_key(key) {
                let black = Rect::new(row.x, row.y, row.width * 0.6, row.height);
                theme::channel_fill(renderer, black, channel, Some(theme::KEY_BLACK));
            } else {
                let white = Rect::new(row.x, row.y + dot, row.width, row.height - 2.0 * dot);
                theme::channel_fill(renderer, white, channel, Some(theme::KEY_WHITE));
            }
        }
        // Octave boundary marker under B and E (the two single-semitone gaps).
        if matches!(key % 12, 11 | 4) {
            renderer.fill_rect(Rect::new(row.x, row.bottom() - dot, row.width, dot), theme::DIM);
        }

        if key % 12 == 0 && slot_h >= ch * 0.6 {
            let font_size = renderer.font_size().min(slot_h);
            let (note_str, octave) = key_name_parts(key);
            let label = format!("{}{}", note_str, octave);
            let label_w = text_width(&label, cw) * (font_size / renderer.font_size());
            let label_x = kb_area.right() - px(2.0, scale) - label_w;
            let label_y = row.y + (slot_h - font_size) / 2.0;
            renderer.draw_text(label_x, label_y, &label, theme::GROUND, font_size);
        }
    }
    renderer.fill_rect(Rect::new(note_area.x - dot, note_area.y, dot, note_area.height), theme::FRAME);

    // 6. Playhead + ruler marker. Drawn after the FRAME separator so its
    // 2-dot band covers the 1-dot line at the same edge.
    let playhead_x = axis.origin_px;
    let pw = 2.0 * dot;
    renderer.fill_rect(Rect::new(playhead_x - pw / 2.0, area.y, pw, note_area.bottom() - area.y), theme::PLAYHEAD);

    const MARKER_WIDTHS: [f32; 4] = [7.0, 5.0, 3.0, 1.0];
    let marker_half_w = MARKER_WIDTHS[0] * dot / 2.0;
    let marker_x = playhead_x.clamp(ruler_area.x + marker_half_w, ruler_area.right() - marker_half_w);
    let marker_row_h = ruler_h / MARKER_WIDTHS.len() as f32;
    for (i, &w) in MARKER_WIDTHS.iter().enumerate() {
        let rw = w * dot;
        let ry = area.y + i as f32 * marker_row_h;
        renderer.fill_rect(Rect::new(marker_x - rw / 2.0, ry, rw, marker_row_h), theme::PLAYHEAD);
    }

    // Hit regions: broad wheel target first, then the more specific ruler.
    hits.push(
        Rect::new(area.x, note_area.y, area.width, note_area.height),
        HitAction::PianoRoll,
    );
    hits.push(
        Rect::new(note_area.x, area.y, note_area.width, ruler_h),
        HitAction::Ruler {
            axis_origin: axis.origin_px,
            origin_tick: axis.current_tick,
            px_per_tick: axis.px_per_tick,
            vertical: false,
        },
    );

    Some(effective_scroll)
}

/// Linear tick <-> pixel-position mapping shared by the horizontal and
/// vertical piano rolls. `origin_px` is the position of `current_tick` along
/// the axis (the playhead); `px_per_tick` carries both zoom and direction as
/// its sign, so callers don't need to special-case which way notes travel.
struct TimeAxis {
    origin_px: f32,
    current_tick: f64,
    px_per_tick: f64,
}

impl TimeAxis {
    fn new(origin_px: f32, current_tick: u64, px_per_tick: f64) -> Self {
        TimeAxis { origin_px, current_tick: current_tick as f64, px_per_tick }
    }

    fn pos(&self, tick: u64) -> f32 {
        self.origin_px + ((tick as f64 - self.current_tick) * self.px_per_tick) as f32
    }

    fn tick_at(&self, pos: f32) -> f64 {
        self.current_tick + (pos - self.origin_px) as f64 / self.px_per_tick
    }

    /// Ticks spanning `[a, b]` of the note area, as `(min, max)` regardless
    /// of which of `a`/`b` is later in time for the current axis direction.
    fn visible_range(&self, a: f32, b: f32) -> (u64, u64) {
        let ta = self.tick_at(a).max(0.0);
        let tb = self.tick_at(b).max(0.0);
        (ta.min(tb) as u64, ta.max(tb) as u64)
    }
}

/// Vertical piano roll: keys on X-axis (left=low, right=high), time on the
/// Y-axis. `app.piano_roll_flow` controls where the keyboard strip sits and
/// which way notes travel: `Down` (falling, default) has notes fall from
/// the top onto a keyboard at the bottom, with the playhead at the note
/// area's bottom edge (against the keyboard) so a sounding note shows only
/// its remaining part; `Up` (tracker style) has notes rise from below
/// toward a keyboard at the top, with the playhead fixed 75% of the way
/// down the note area so history remains visible above it.
fn render_vertical(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    note_rects: &[NoteRect],
    hits: &mut HitMap,
) -> Option<i32> {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let dot = dot_size(scale);
    let flow = app.piano_roll_flow;

    let kb_h = (2.0 * ch).max(px(28.0, scale));
    let ruler_w = text_width("000", cw) + 2.0 * px(3.0, scale);

    if area.width - ruler_w < 10.0 || area.height - kb_h < 10.0 {
        return None;
    }

    // Down: notes on top, keyboard at the bottom. Up: keyboard on top, notes below.
    let (note_area, kb_area) = match flow {
        VerticalFlow::Down => (
            Rect::new(area.x + ruler_w, area.y, area.width - ruler_w, area.height - kb_h),
            Rect::new(area.x + ruler_w, area.bottom() - kb_h, area.width - ruler_w, kb_h),
        ),
        VerticalFlow::Up => (
            Rect::new(area.x + ruler_w, area.y + kb_h, area.width - ruler_w, area.height - kb_h),
            Rect::new(area.x + ruler_w, area.y, area.width - ruler_w, kb_h),
        ),
    };
    let ruler_area = Rect::new(area.x, area.y, ruler_w, area.height);

    // Key range
    let song_lo = note_rects.iter().map(|n| n.key).min().unwrap_or(0);
    let song_hi = note_rects.iter().map(|n| n.key).max().unwrap_or(127);
    let min_slot = px(6.0, scale);
    let (lo, hi, slot_w, effective_scroll) = visible_key_range(song_lo, song_hi, note_area.width, min_slot, app.key_scroll);

    // Pixels per tick (zoom) — scale by DPI, not font size, so changing the
    // font does not change zoom. Constants match the old ch-derived values
    // closely enough at a 14px font.
    let pixels_per_tick = 0.05 * app.zoom_level * (16.0 * scale) as f64;

    let current_tick = app.current_tick();
    let (playhead_y, px_per_tick) = match flow {
        VerticalFlow::Down => (note_area.bottom(), -pixels_per_tick),
        VerticalFlow::Up => (note_area.y + note_area.height * 0.75, pixels_per_tick),
    };
    let axis = TimeAxis::new(playhead_y, current_tick, px_per_tick);
    let (view_start_tick, view_end_tick) = axis.visible_range(note_area.y, note_area.bottom());

    // 1. Note area background + black-key columns
    renderer.fill_rect(note_area, theme::GROUND);
    for key in lo..=hi {
        if is_black_key(key) {
            let offset = (key - lo) as f32;
            let col = Rect::new(note_area.x + offset * slot_w, note_area.y, slot_w, note_area.height);
            renderer.fill_rect(col, theme::KEY_ROW_BLACK);
        }
    }

    // 2. Bar/beat grid (horizontal lines)
    let grid_end = view_end_tick.max(view_start_tick + 1);
    for (tick, is_bar, _) in app.bar_map.lines_in(view_start_tick, grid_end) {
        let y = axis.pos(tick);
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
        let y = axis.pos(tick);
        if y < note_area.y || y > note_area.bottom() {
            continue;
        }
        let bar_px = app.bar_map.ticks_per_bar_at(tick) as f64 * pixels_per_tick;
        if bar_px < bar_num_min_spacing as f64 {
            continue;
        }
        let label = format!("{:03}", bar_no);
        // Label sits on the "later" side of its bar line, inside its own bar.
        // Text isn't clipped to the panel, so clamp into the note area to
        // avoid bleeding into the title bar (Down) or transport bar (Up)
        // when a bar line lands near the note area's edge.
        let label_y = match flow {
            VerticalFlow::Down => (y - px(3.0, scale) - ch).max(note_area.y),
            VerticalFlow::Up => (y + px(3.0, scale)).min(note_area.bottom() - ch),
        };
        renderer.draw_text(ruler_area.x, label_y, &label, theme::DIM, ch);
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

        let (a, b) = (axis.pos(note.start_tick), axis.pos(note.end_tick));
        let y0 = a.min(b).max(note_area.y);
        let y1 = a.max(b).min(note_area.bottom());
        if y0 >= y1 {
            continue;
        }

        let rect = Rect::new(bar_x, y0, bar_w, (y1 - y0).max(1.0));
        let state = note_state(note.start_tick, note.end_tick, current_tick, muted);
        draw_note(renderer, rect, note.channel, state, dot);
    }

    // 5. Keyboard strip. Black keys sit in the 60% nearest the note area;
    // C labels sit on the edge away from it.
    let sounding = sounding_keys(note_rects, scan_start, current_tick, muted_mask);
    let label_min_slot = text_width("C-1", cw) + px(2.0, scale);
    for key in lo..=hi {
        let offset = (key - lo) as f32;
        let col = Rect::new(kb_area.x + offset * slot_w, kb_area.y, slot_w, kb_area.height);

        renderer.fill_rect(col, theme::KEY_WHITE);
        if is_black_key(key) {
            let black = match flow {
                VerticalFlow::Down => Rect::new(col.x, col.y, col.width, col.height * 0.6),
                VerticalFlow::Up => Rect::new(col.x, col.y + col.height * 0.4, col.width, col.height * 0.6),
            };
            renderer.fill_rect(black, theme::KEY_BLACK);
        }
        if let Some(channel) = sounding[key as usize] {
            if is_black_key(key) {
                let black = match flow {
                    VerticalFlow::Down => Rect::new(col.x, col.y, col.width, col.height * 0.6),
                    VerticalFlow::Up => Rect::new(col.x, col.y + col.height * 0.4, col.width, col.height * 0.6),
                };
                theme::channel_fill(renderer, black, channel, Some(theme::KEY_BLACK));
            } else {
                let white = Rect::new(col.x + dot, col.y, col.width - 2.0 * dot, col.height);
                theme::channel_fill(renderer, white, channel, Some(theme::KEY_WHITE));
            }
        }
        // Octave boundary marker right of B and E (the two single-semitone gaps).
        if matches!(key % 12, 11 | 4) {
            renderer.fill_rect(Rect::new(col.right() - dot, col.y, dot, col.height), theme::DIM);
        }

        if key % 12 == 0 && slot_w >= label_min_slot {
            let font_size = renderer.font_size().min(slot_w * 1.2);
            let (note_str, octave) = key_name_parts(key);
            let label = format!("{}{}", note_str, octave);
            let label_w = text_width(&label, cw) * (font_size / renderer.font_size());
            let label_x = col.x + (slot_w - label_w) / 2.0;
            let label_y = match flow {
                VerticalFlow::Down => kb_area.bottom() - px(2.0, scale) - font_size,
                VerticalFlow::Up => kb_area.y + px(2.0, scale),
            };
            renderer.draw_text(label_x, label_y, &label, theme::GROUND, font_size);
        }
    }
    // FRAME separator on the note-area edge adjacent to the keyboard.
    let frame_y = match flow {
        VerticalFlow::Down => note_area.bottom(),
        VerticalFlow::Up => note_area.y - dot,
    };
    renderer.fill_rect(Rect::new(note_area.x, frame_y, note_area.width, dot), theme::FRAME);

    // 6. Playhead + ruler marker (points right toward the notes either way).
    // Drawn after the FRAME separator so its 2-dot band covers the 1-dot
    // line at the same edge (coincides with it in `Down`; `Up` keeps its
    // own separate 75% position).
    let ph = 2.0 * dot;
    renderer.fill_rect(Rect::new(area.x, playhead_y - ph / 2.0, note_area.right() - area.x, ph), theme::PLAYHEAD);

    const MARKER_HEIGHTS: [f32; 4] = [7.0, 5.0, 3.0, 1.0];
    let marker_half_h = MARKER_HEIGHTS[0] * dot / 2.0;
    let marker_y = playhead_y.clamp(ruler_area.y + marker_half_h, ruler_area.bottom() - marker_half_h);
    let marker_col_w = ruler_w / MARKER_HEIGHTS.len() as f32;
    for (i, &h) in MARKER_HEIGHTS.iter().enumerate() {
        let rh = h * dot;
        let rx = ruler_area.x + i as f32 * marker_col_w;
        renderer.fill_rect(Rect::new(rx, marker_y - rh / 2.0, marker_col_w, rh), theme::PLAYHEAD);
    }

    // Hit regions: broad wheel target first, then the more specific ruler.
    hits.push(
        Rect::new(note_area.x, area.y, note_area.width, area.height),
        HitAction::PianoRoll,
    );
    hits.push(
        Rect::new(area.x, note_area.y, ruler_w, note_area.height),
        HitAction::Ruler {
            axis_origin: axis.origin_px,
            origin_tick: axis.current_tick,
            px_per_tick: axis.px_per_tick,
            vertical: true,
        },
    );

    Some(effective_scroll)
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
    fn time_axis_current_tick_is_always_at_origin() {
        let up = TimeAxis::new(300.0, 10_000, 0.5);
        let down = TimeAxis::new(300.0, 10_000, -0.5);
        assert_eq!(up.pos(10_000), 300.0);
        assert_eq!(down.pos(10_000), 300.0);
    }

    #[test]
    fn time_axis_later_tick_is_below_origin_for_positive_ppt() {
        let axis = TimeAxis::new(300.0, 10_000, 0.5);
        assert!(axis.pos(10_100) > axis.pos(10_000));
    }

    #[test]
    fn time_axis_later_tick_is_above_origin_for_negative_ppt() {
        let axis = TimeAxis::new(300.0, 10_000, -0.5);
        assert!(axis.pos(10_100) < axis.pos(10_000));
    }

    #[test]
    fn time_axis_visible_range_endpoints_map_to_note_area_edges() {
        // ppt = 0.5 => top (y=0) is 600 ticks before current, bottom (y=400) is 200 after.
        let up = TimeAxis::new(300.0, 10_000, 0.5);
        let (lo, hi) = up.visible_range(0.0, 400.0);
        assert_eq!((lo, hi), (9_400, 10_200));

        // Negative ppt inverts which edge is the earlier tick.
        let down = TimeAxis::new(300.0, 10_000, -0.5);
        let (lo, hi) = down.visible_range(0.0, 400.0);
        assert_eq!((lo, hi), (9_800, 10_600));
    }

    #[test]
    fn time_axis_visible_range_matches_horizontal_playhead_at_origin() {
        // Horizontal mode: origin sits at the near edge, so the visible
        // range starts exactly at current_tick.
        let axis = TimeAxis::new(100.0, 5_000, 0.5);
        let (lo, hi) = axis.visible_range(100.0, 100.0 + 400.0);
        assert_eq!((lo, hi), (5_000, 5_000 + (400.0 / 0.5) as u64));
    }

    #[test]
    fn visible_key_range_shows_full_range_when_it_fits() {
        let (lo, hi, slot, effective_scroll) = visible_key_range(40, 60, 200.0, 6.0, 0);
        assert_eq!((lo, hi), (40, 60));
        assert!((slot - 200.0 / 21.0).abs() < 0.001);
        assert_eq!(effective_scroll, 0);
    }

    #[test]
    fn visible_key_range_centers_when_it_does_not_fit() {
        // 100 keys at min_slot=6 needs 600px; only 60px available -> 10 keys fit, centered.
        let (lo, hi, slot, effective_scroll) = visible_key_range(0, 99, 60.0, 6.0, 0);
        assert_eq!(hi - lo, 9);
        assert_eq!(lo, 45); // (100-10)/2
        assert!((slot - 6.0).abs() < 0.001);
        assert_eq!(effective_scroll, 0);
    }

    #[test]
    fn visible_key_range_scrolls_and_clamps_at_high_end() {
        let (lo, hi, _, effective_scroll) = visible_key_range(0, 99, 60.0, 6.0, 1000);
        assert_eq!(hi, 99); // clamped to song_hi
        assert_eq!(lo, 90);
        // Overshoot is clamped away rather than carried invisibly: applying it
        // again should reproduce the same (already-clamped) range.
        let (lo2, hi2, _, _) = visible_key_range(0, 99, 60.0, 6.0, effective_scroll);
        assert_eq!((lo2, hi2), (lo, hi));
    }

    #[test]
    fn visible_key_range_scrolls_and_clamps_at_low_end() {
        let (lo, hi, _, effective_scroll) = visible_key_range(0, 99, 60.0, 6.0, -1000);
        assert_eq!(lo, 0); // clamped to song_lo
        assert_eq!(hi, 9);
        let (lo2, hi2, _, _) = visible_key_range(0, 99, 60.0, 6.0, effective_scroll);
        assert_eq!((lo2, hi2), (lo, hi));
    }

    #[test]
    fn visible_key_range_reversing_after_overshoot_moves_immediately() {
        // Scroll far past the top, then apply one step in the other direction:
        // the range must actually move, not still be absorbing the overshoot.
        let (_, hi_clamped, _, effective_scroll) = visible_key_range(0, 99, 60.0, 6.0, 1000);
        let (_, hi_after_reverse, _, _) = visible_key_range(0, 99, 60.0, 6.0, effective_scroll - 1);
        assert!(hi_after_reverse < hi_clamped);
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

    fn nr(key: u8, channel: u8, port: u8, start_tick: u64, end_tick: u64) -> NoteRect {
        NoteRect { key, channel, port, start_tick, end_tick, velocity: 100, track: 0 }
    }

    #[test]
    fn sounding_keys_none_sounding() {
        let notes = [nr(60, 0, 0, 0, 100)];
        assert_eq!(sounding_keys(&notes, 0, 200, 0), [None; 128]);
    }

    #[test]
    fn sounding_keys_latest_start_wins_tie_breaks_to_lower_channel() {
        // Two overlapping notes on the same key at tick 50: the one that
        // started later (channel 2) wins over the earlier one (channel 1).
        let notes = [nr(60, 1, 0, 0, 100), nr(60, 2, 0, 30, 100)];
        let result = sounding_keys(&notes, 0, 50, 0);
        assert_eq!(result[60], Some(2));

        // Same start tick on both: the lower channel wins.
        let notes = [nr(60, 3, 0, 0, 100), nr(60, 1, 0, 0, 100)];
        let result = sounding_keys(&notes, 0, 50, 0);
        assert_eq!(result[60], Some(1));
    }

    #[test]
    fn sounding_keys_ignores_muted_note() {
        let notes = [nr(60, 2, 0, 0, 100)];
        let flat_ch = 1u64 << 2; // port 0, channel 2
        assert_eq!(sounding_keys(&notes, 0, 50, flat_ch)[60], None);
    }

    #[test]
    fn sounding_keys_note_ending_exactly_at_tick_is_not_sounding() {
        let notes = [nr(60, 0, 0, 0, 100)];
        assert_eq!(sounding_keys(&notes, 0, 100, 0)[60], None);
    }
}
