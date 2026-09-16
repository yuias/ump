//! Track list panel: Default mode (compact per-channel) and Detail mode (per-track tree).

use std::sync::atomic::Ordering;

use crate::app::{App, TrackViewMode};
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::state::TrackInfoSnapshot;
use ump_playback::synth::gm::gm_instrument_name;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::{dot_size, px, ROW_HEIGHT};
use crate::ui::text::truncate_cells;
use crate::ui::theme;

/// Represents a single row in the flattened Detail view.
#[derive(Debug, Clone)]
pub enum RawRow {
    TrackHeader { track_idx: usize },
    Channel { _track_idx: usize, port: u8, channel: u8 },
}

/// Build the flat row list for the current view mode.
pub fn build_raw_rows(
    tracks: &[TrackInfoSnapshot],
    port_count: u8,
    current_port: u8,
    mode: TrackViewMode,
) -> Vec<RawRow> {
    match mode {
        TrackViewMode::Default => build_default_rows(port_count, current_port),
        TrackViewMode::Detail => build_detail_rows(tracks),
    }
}

/// Build rows for Default mode: 16 channels for the current port.
fn build_default_rows(port_count: u8, current_port: u8) -> Vec<RawRow> {
    let mut rows = Vec::new();
    let port = current_port.min(port_count.saturating_sub(1));
    for ch in 0..16u8 {
        rows.push(RawRow::Channel {
            _track_idx: 0,
            port,
            channel: ch,
        });
    }
    rows
}

/// Build rows for Detail mode: track tree with channels.
fn build_detail_rows(tracks: &[TrackInfoSnapshot]) -> Vec<RawRow> {
    let mut rows = Vec::new();
    for t in tracks {
        if t.note_count == 0 && t.name.is_empty() {
            continue;
        }
        rows.push(RawRow::TrackHeader { track_idx: t.index });
        for ch in 0..16u8 {
            if t.channel_note_counts[ch as usize] > 0 {
                rows.push(RawRow::Channel {
                    _track_idx: t.index,
                    port: t.port,
                    channel: ch,
                });
            }
        }
    }
    rows
}

/// Return the total number of rows in the current view mode.
pub fn raw_row_count(
    tracks: &[TrackInfoSnapshot],
    port_count: u8,
    current_port: u8,
    mode: TrackViewMode,
) -> usize {
    build_raw_rows(tracks, port_count, current_port, mode).len()
}

/// Which optional Default-mode columns are visible. Always tracked together
/// with the drop priority `EXP, PAN, VOL, PRG, meter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnFlags {
    prg: bool,
    vol: bool,
    pan: bool,
    exp: bool,
    meter: bool,
}

impl ColumnFlags {
    fn all() -> Self {
        ColumnFlags { prg: true, vol: true, pan: true, exp: true, meter: true }
    }

    /// Total px width consumed by the columns currently marked visible,
    /// each including its trailing gap.
    fn width(&self, param_w: f32, pad: f32, meter_w: f32) -> f32 {
        let mut w = 0.0;
        if self.prg { w += param_w + pad; }
        if self.vol { w += param_w + pad; }
        if self.pan { w += param_w + pad; }
        if self.exp { w += param_w + pad; }
        if self.meter { w += meter_w + pad; }
        w
    }

    /// Drop the next column in priority order. Returns `false` once nothing
    /// is left to drop.
    fn drop_next(&mut self) -> bool {
        if self.exp {
            self.exp = false;
        } else if self.pan {
            self.pan = false;
        } else if self.vol {
            self.vol = false;
        } else if self.prg {
            self.prg = false;
        } else if self.meter {
            self.meter = false;
        } else {
            return false;
        }
        true
    }
}

/// Decide which optional columns (PRG/VOL/PAN/EXP/meter) fit in `avail` px
/// while keeping at least `min_name_w` px for the instrument name column.
/// Drops columns in priority order: EXP, PAN, VOL, PRG, meter.
fn fit_default_columns(
    mut flags: ColumnFlags,
    avail: f32,
    min_name_w: f32,
    param_w: f32,
    pad: f32,
    meter_w: f32,
) -> ColumnFlags {
    while avail - flags.width(param_w, pad, meter_w) < min_name_w {
        if !flags.drop_next() {
            break;
        }
    }
    flags
}

/// Pixel x-positions of each Default/Detail row column, shared by the header
/// and the row renderers so labels and values always line up.
struct Columns {
    chnum_x: f32,
    name_x: f32,
    name_cols: usize,
    prg_x: Option<f32>,
    vol_x: Option<f32>,
    pan_x: Option<f32>,
    exp_x: Option<f32>,
    meter_x: Option<f32>,
    mute_x: f32,
}

/// Total px width of the 12-segment meter, including inter-segment gaps.
fn meter_width_px(cw: f32, dot: f32) -> f32 {
    let seg_w = (0.45 * cw).round().max(2.0 * dot);
    12.0 * seg_w + 11.0 * dot
}

/// Compute column x-positions for a row area starting at `x0` with `width` px.
/// `include_params` gates PRG/VOL/PAN/EXP (Detail mode channel rows omit them
/// entirely, so only the meter competes with the name column for space) and
/// selects the channel-number width for that mode's label.
fn compute_columns(x0: f32, width: f32, cw: f32, ch: f32, pad: f32, dot: f32, include_params: bool) -> Columns {
    let swatch_w = 0.6 * ch;
    // Default rows print "01"; Detail channel rows (the no-params case) print "CH01".
    let chnum_w = if include_params { 2.0 } else { 4.0 } * cw;
    let param_w = 3.0 * cw;
    let mute_w = ch;
    let meter_w = meter_width_px(cw, dot);

    let chnum_x = x0 + swatch_w + pad;
    let name_x = chnum_x + chnum_w + pad;
    let mute_x = x0 + width - pad - mute_w;
    let region_right = mute_x - pad;

    let avail = (region_right - name_x).max(0.0);
    let min_name_w = 8.0 * cw;

    let mut initial = ColumnFlags::all();
    if !include_params {
        initial.prg = false;
        initial.vol = false;
        initial.pan = false;
        initial.exp = false;
    }
    let flags = fit_default_columns(initial, avail, min_name_w, param_w, pad, meter_w);

    // Place present columns from the right edge inward, in on-screen order
    // (PRG, VOL, PAN, EXP, meter), so each reserves its width plus one gap.
    let mut cursor = region_right;
    let mut meter_x = None;
    let mut exp_x = None;
    let mut pan_x = None;
    let mut vol_x = None;
    let mut prg_x = None;

    if flags.meter { cursor -= meter_w; meter_x = Some(cursor); cursor -= pad; }
    if flags.exp { cursor -= param_w; exp_x = Some(cursor); cursor -= pad; }
    if flags.pan { cursor -= param_w; pan_x = Some(cursor); cursor -= pad; }
    if flags.vol { cursor -= param_w; vol_x = Some(cursor); cursor -= pad; }
    if flags.prg { cursor -= param_w; prg_x = Some(cursor); cursor -= pad; }

    let name_w = (cursor - name_x).max(0.0);
    let name_cols = (name_w / cw).floor().max(0.0) as usize;

    Columns { chnum_x, name_x, name_cols, prg_x, vol_x, pan_x, exp_x, meter_x, mute_x }
}

/// Row height for the list body: divides available content height into 16
/// nominal rows, clamped to a sane range so very short/tall panels stay readable.
fn list_row_height(content_h: f32, ch: f32) -> f32 {
    (content_h / 16.0).clamp(ch * ROW_HEIGHT, ch * 2.0)
}

pub fn render_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App, hits: &mut HitMap) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let pad = px(4.0, scale);
    let dot = dot_size(scale);
    let header_h = ch * ROW_HEIGHT;
    if area.height < header_h * 2.0 {
        return;
    }

    render_track_header(renderer, area, cw, ch, pad, dot, header_h, app);

    let content = Rect::new(area.x, area.y + header_h, area.width, area.height - header_h);
    let row_h = list_row_height(content.height, ch);

    hits.push(content, HitAction::TrackList);

    match app.track_view_mode {
        TrackViewMode::Default => render_default_track_list(renderer, content, app, cw, ch, pad, dot, row_h, hits),
        TrackViewMode::Detail => render_detail_track_list(renderer, content, app, cw, ch, pad, dot, row_h, hits),
    }
}

/// Draw the track list header row with column labels.
#[allow(clippy::too_many_arguments)]
fn render_track_header(
    renderer: &mut dyn Renderer,
    area: Rect,
    cw: f32,
    ch: f32,
    pad: f32,
    dot: f32,
    header_h: f32,
    app: &App,
) {
    let y = area.y + (header_h - ch) / 2.0;

    match app.track_view_mode {
        TrackViewMode::Default => {
            let cols = compute_columns(area.x, area.width, cw, ch, pad, dot, true);
            renderer.draw_text(cols.chnum_x, y, "CH", theme::DIM, ch);
            renderer.draw_text(cols.name_x, y, "INSTRUMENT", theme::DIM, ch);
            if let Some(x) = cols.prg_x { renderer.draw_text(x, y, "PRG", theme::DIM, ch); }
            if let Some(x) = cols.vol_x { renderer.draw_text(x, y, "VOL", theme::DIM, ch); }
            if let Some(x) = cols.pan_x { renderer.draw_text(x, y, "PAN", theme::DIM, ch); }
            if let Some(x) = cols.exp_x { renderer.draw_text(x, y, "EXP", theme::DIM, ch); }
            if let Some(x) = cols.meter_x { renderer.draw_text(x, y, "LEVEL", theme::DIM, ch); }
            renderer.draw_text(cols.mute_x, y, "M", theme::DIM, ch);
        }
        TrackViewMode::Detail => {
            renderer.draw_text(area.x, y, "TRACK / CHANNEL", theme::DIM, ch);
        }
    }
}

/// Draw a channel's mute indicator box: filled+"M" when muted, dotted frame+"M" otherwise.
fn draw_mute_box(renderer: &mut dyn Renderer, rect: Rect, muted: bool, ch: f32, cw: f32, dot: f32) {
    let text_color = if muted {
        renderer.fill_rect(rect, theme::PLAYHEAD);
        theme::GROUND
    } else {
        renderer.fill_rect(Rect::new(rect.x, rect.y, rect.width, dot), theme::DIM);
        renderer.fill_rect(Rect::new(rect.x, rect.bottom() - dot, rect.width, dot), theme::DIM);
        renderer.fill_rect(Rect::new(rect.x, rect.y, dot, rect.height), theme::DIM);
        renderer.fill_rect(Rect::new(rect.right() - dot, rect.y, dot, rect.height), theme::DIM);
        theme::DIM
    };
    let tx = rect.x + (rect.width - cw) / 2.0;
    let ty = rect.y + (rect.height - ch) / 2.0;
    renderer.draw_text(tx, ty, "M", text_color, ch);
}

/// Draw the 12-segment level meter. The top two segments (indices 10-11)
/// light up in `PLAYHEAD` instead of the channel color, as a clip warning.
#[allow(clippy::too_many_arguments)]
fn draw_meter(renderer: &mut dyn Renderer, x: f32, center_y: f32, seg_h: f32, cw: f32, dot: f32, level: f32, ch_idx: u8) {
    let seg_w = (0.45 * cw).round().max(2.0 * dot);
    let lit = (level * 12.0).round().clamp(0.0, 12.0) as i32;
    let y = center_y - seg_h / 2.0;
    for i in 0..12i32 {
        let seg_x = x + i as f32 * (seg_w + dot);
        let color = if i < lit {
            if i >= 10 { theme::PLAYHEAD } else { theme::channel_color(ch_idx) }
        } else {
            theme::WELL
        };
        renderer.fill_rect(Rect::new(seg_x, y, seg_w, seg_h), color);
    }
}

/// Format CC10 Pan as "C" (center) or "L{n}"/"R{n}".
fn pan_display(pan: u32) -> String {
    if pan == 64 {
        "C".to_string()
    } else if pan < 64 {
        format!("L{}", 64 - pan)
    } else {
        format!("R{}", pan - 64)
    }
}

/// Default mode: one row per channel of the current port.
#[allow(clippy::too_many_arguments)]
fn render_default_track_list(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    cw: f32,
    ch: f32,
    pad: f32,
    dot: f32,
    row_h: f32,
    hits: &mut HitMap,
) {
    if area.width < cw * 10.0 || row_h <= 0.0 {
        return;
    }

    let cs = &app.shared.channel_states;
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);
    let drum_mask = app.shared.drum_channels.load(Ordering::Relaxed);
    let active_channels = app.used_channels;
    let port_offset = app.current_port as u64 * 16;

    let visible_rows = (area.height / row_h) as usize;
    if visible_rows == 0 {
        return;
    }
    let scroll_offset = if app.track_cursor >= visible_rows {
        app.track_cursor - visible_rows + 1
    } else {
        0
    };

    let cols = compute_columns(area.x, area.width, cw, ch, pad, dot, true);

    for ch_idx in 0..16u8 {
        let row = ch_idx as usize;
        if row < scroll_offset || row >= scroll_offset + visible_rows {
            continue;
        }

        let screen_row = row - scroll_offset;
        let row_y = area.y + screen_row as f32 * row_h;
        let row_rect = Rect::new(area.x, row_y, area.width, row_h);
        let text_y = row_y + (row_h - ch) / 2.0;

        let flat_ch = port_offset + ch_idx as u64;
        let is_active = active_channels & (1u64 << flat_ch) != 0;
        let is_muted = muted_mask & (1u64 << flat_ch) != 0;
        let is_drum = drum_mask & (1u64 << flat_ch) != 0;
        let is_selected = app.track_cursor == row;

        if is_selected {
            renderer.fill_rect(row_rect, theme::SELECTED_BG);
        }

        let mute_rect = Rect::new(cols.mute_x, row_y + (row_h - ch) / 2.0, ch, ch);
        let track_rect = Rect::new(area.x, row_y, cols.mute_x - area.x, row_h);
        hits.push(track_rect, HitAction::TrackRow(row));
        hits.push(mute_rect, HitAction::TrackMute(row));
        draw_mute_box(renderer, mute_rect, is_muted, ch, cw, dot);

        if !is_active {
            renderer.draw_text(cols.chnum_x, text_y, &format!("{:02}", ch_idx + 1), theme::DIM, ch);
            renderer.draw_text(cols.name_x, text_y, "---", theme::DIM, ch);
            continue;
        }

        let row_bg = if is_selected { theme::SELECTED_BG } else { theme::GROUND };
        let swatch_size = 0.6 * ch;
        let swatch_rect = Rect::new(area.x, row_y + (row_h - swatch_size) / 2.0, swatch_size, swatch_size);
        if is_muted {
            renderer.fill_rect(swatch_rect, theme::DIM);
        } else {
            theme::channel_fill(renderer, swatch_rect, ch_idx, Some(row_bg));
        }

        let num_color = if is_selected { theme::ACCENT } else { theme::DIM };
        renderer.draw_text(cols.chnum_x, text_y, &format!("{:02}", ch_idx + 1), num_color, ch);

        let ci = flat_ch as usize;
        let prog = cs.program[ci].load(Ordering::Relaxed);
        let inst_name = if is_drum { "Drums" } else { gm_instrument_name(prog as u8) };
        let truncated = truncate_cells(inst_name, cols.name_cols);
        let name_color = if is_muted { theme::DIM } else { theme::TEXT };
        renderer.draw_text(cols.name_x, text_y, &truncated, name_color, ch);

        let param_color = if is_muted { theme::DIM } else { theme::TEXT };
        if let Some(x) = cols.prg_x {
            let prg_str = if is_drum { "DR ".to_string() } else { format!("{:03}", prog + 1) };
            renderer.draw_text(x, text_y, &prg_str, param_color, ch);
        }
        if let Some(x) = cols.vol_x {
            let vol = cs.volume[ci].load(Ordering::Relaxed);
            renderer.draw_text(x, text_y, &format!("{:>3}", vol), param_color, ch);
        }
        if let Some(x) = cols.pan_x {
            let pan = cs.pan[ci].load(Ordering::Relaxed);
            renderer.draw_text(x, text_y, &format!("{:<3}", pan_display(pan)), param_color, ch);
        }
        if let Some(x) = cols.exp_x {
            let exp = cs.expression[ci].load(Ordering::Relaxed);
            renderer.draw_text(x, text_y, &format!("{:>3}", exp), param_color, ch);
        }
        if let Some(x) = cols.meter_x {
            let level = if is_muted { 0.0 } else { app.channel_levels[ci] };
            draw_meter(renderer, x, row_y + row_h / 2.0, ch * 0.55, cw, dot, level, ch_idx);
        }
    }
}

/// Detail mode: per-track tree with scrolling.
#[allow(clippy::too_many_arguments)]
fn render_detail_track_list(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
    cw: f32,
    ch: f32,
    pad: f32,
    dot: f32,
    row_h: f32,
    hits: &mut HitMap,
) {
    if area.width < cw * 10.0 || row_h <= 0.0 {
        return;
    }

    let tracks = app.shared.track_info.lock().unwrap();
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);
    let drum_mask = app.shared.drum_channels.load(Ordering::Relaxed);
    let cs = &app.shared.channel_states;

    let rows = build_detail_rows(&tracks);

    let visible_rows = (area.height / row_h) as usize;
    if visible_rows == 0 {
        return;
    }
    let scroll_offset = if app.track_cursor >= visible_rows {
        app.track_cursor - visible_rows + 1
    } else {
        0
    };

    let show_port = app.port_count > 1;
    let indent = 2.0 * cw;
    // Channel-row columns (name/meter/mute), indented; PRG/VOL/PAN/EXP omitted.
    let cols = compute_columns(area.x + indent, area.width - indent, cw, ch, pad, dot, false);
    let header_name_cols = ((area.width / cw) as usize).saturating_sub(8);

    for (i, row) in rows.iter().enumerate().skip(scroll_offset).take(visible_rows) {
        let screen_row = i - scroll_offset;
        let row_y = area.y + screen_row as f32 * row_h;
        let row_rect = Rect::new(area.x, row_y, area.width, row_h);
        let is_selected = i == app.track_cursor;
        let text_y = row_y + (row_h - ch) / 2.0;

        if is_selected {
            renderer.fill_rect(row_rect, theme::SELECTED_BG);
        }

        match row {
            RawRow::TrackHeader { track_idx } => {
                let t = tracks.iter().find(|t| t.index == *track_idx).unwrap();
                let base_name = if t.name.is_empty() {
                    format!("Trk {}", t.index)
                } else {
                    t.name.clone()
                };
                let port_label = if show_port { format!("[P{}] ", t.port + 1) } else { String::new() };
                let full = format!("{}{}", port_label, base_name);
                let truncated = truncate_cells(&full, header_name_cols);
                renderer.draw_text_bold(area.x, text_y, &truncated, theme::TEXT, ch);
                hits.push(row_rect, HitAction::TrackRow(i));
            }
            RawRow::Channel { port, channel, .. } => {
                let ch_idx = *channel;
                let flat_ch = *port as u64 * 16 + ch_idx as u64;
                let is_muted = muted_mask & (1u64 << flat_ch) != 0;
                let is_drum = drum_mask & (1u64 << flat_ch) != 0;

                let mute_rect = Rect::new(cols.mute_x, row_y + (row_h - ch) / 2.0, ch, ch);
                let track_rect = Rect::new(area.x, row_y, cols.mute_x - area.x, row_h);
                hits.push(track_rect, HitAction::TrackRow(i));
                hits.push(mute_rect, HitAction::TrackMute(i));
                draw_mute_box(renderer, mute_rect, is_muted, ch, cw, dot);

                let row_bg = if is_selected { theme::SELECTED_BG } else { theme::GROUND };
                let swatch_size = 0.6 * ch;
                let swatch_rect = Rect::new(
                    area.x + indent,
                    row_y + (row_h - swatch_size) / 2.0,
                    swatch_size,
                    swatch_size,
                );
                if is_muted {
                    renderer.fill_rect(swatch_rect, theme::DIM);
                } else {
                    theme::channel_fill(renderer, swatch_rect, ch_idx, Some(row_bg));
                }

                let label_color = if is_muted { theme::DIM } else { theme::channel_color(ch_idx) };
                renderer.draw_text(cols.chnum_x, text_y, &format!("CH{:02}", ch_idx + 1), label_color, ch);

                let ci = flat_ch as usize;
                let prog = cs.program[ci].load(Ordering::Relaxed);
                let inst_name = if is_drum { "Drums" } else { gm_instrument_name(prog as u8) };
                let truncated = truncate_cells(inst_name, cols.name_cols);
                let name_color = if is_muted { theme::DIM } else { theme::TEXT };
                renderer.draw_text(cols.name_x, text_y, &truncated, name_color, ch);

                if let Some(x) = cols.meter_x {
                    let level = if is_muted { 0.0 } else { app.channel_levels[ci] };
                    draw_meter(renderer, x, row_y + row_h / 2.0, ch * 0.55, cw, dot, level, ch_idx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_default_columns_keeps_everything_when_wide() {
        let flags = fit_default_columns(ColumnFlags::all(), 1000.0, 80.0, 24.0, 4.0, 60.0);
        assert_eq!(flags, ColumnFlags::all());
    }

    #[test]
    fn fit_default_columns_drops_in_priority_order() {
        // param_w=24, pad=4, meter_w=60. Full width = 4*(24+4) + (60+4) = 176.
        // Shrinking avail should drop EXP, then PAN, then VOL, then PRG, then meter.
        let param_w = 24.0;
        let pad = 4.0;
        let meter_w = 60.0;
        let min_name_w = 80.0;

        // avail just below full (176) but above full-minus-exp(148) forces EXP to drop.
        let flags = fit_default_columns(ColumnFlags::all(), min_name_w + 150.0, min_name_w, param_w, pad, meter_w);
        assert!(!flags.exp);
        assert!(flags.pan && flags.vol && flags.prg && flags.meter);

        // Very little room: only name (and maybe nothing else) fits.
        let flags = fit_default_columns(ColumnFlags::all(), min_name_w + 5.0, min_name_w, param_w, pad, meter_w);
        assert!(!flags.exp && !flags.pan && !flags.vol && !flags.prg && !flags.meter);
    }

    #[test]
    fn fit_default_columns_never_loops_forever_when_impossible() {
        // avail smaller than min_name_w even with nothing shown: drop_next runs out and stops.
        let flags = fit_default_columns(ColumnFlags::all(), 10.0, 80.0, 24.0, 4.0, 60.0);
        assert_eq!(flags, ColumnFlags { prg: false, vol: false, pan: false, exp: false, meter: false });
    }

    #[test]
    fn pan_display_center_and_sides() {
        assert_eq!(pan_display(64), "C");
        assert_eq!(pan_display(0), "L64");
        assert_eq!(pan_display(127), "R63");
    }

    #[test]
    fn list_row_height_clamps_to_range() {
        let ch = 16.0;
        assert_eq!(list_row_height(1000.0, ch), ch * 2.0); // way more than 16 rows worth: capped
        assert_eq!(list_row_height(1.0, ch), ch * ROW_HEIGHT); // way less: floored
        let mid = list_row_height(16.0 * ch * 1.5, ch);
        assert!((mid - ch * 1.5).abs() < 0.01);
    }
}
