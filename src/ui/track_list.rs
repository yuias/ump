//! Track list panel: Default mode (compact per-channel) and Detail mode (per-track tree).

use std::sync::atomic::Ordering;

use crate::app::{App, TrackViewMode};
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::state::TrackInfoSnapshot;
use crate::synth::gm::gm_instrument_name;
use crate::ui::layout::ROW_HEIGHT;
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

pub fn render_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    let row_h = ch * ROW_HEIGHT;
    if area.height < row_h * 2.0 {
        return;
    }

    // Header row
    render_track_header(renderer, area, cw, ch, row_h, app);

    // Content area below header
    let content = Rect::new(area.x, area.y + row_h, area.width, area.height - row_h);
    match app.track_view_mode {
        TrackViewMode::Default => render_default_track_list(renderer, content, app),
        TrackViewMode::Detail => render_detail_track_list(renderer, content, app),
    }
}

/// Draw the track list header row with column labels.
fn render_track_header(
    renderer: &mut dyn Renderer,
    area: Rect,
    cw: f32,
    ch: f32,
    row_h: f32,
    app: &App,
) {
    let dim = theme::HEADER_DIM;
    let y = area.y;

    // // Separator line below header
    // renderer.draw_hline(area.x, area.x + area.width, y + row_h, theme::BORDER_COLOR, 1.0);

    match app.track_view_mode {
        TrackViewMode::Default => {
            // Match column layout of render_default_track_list
            let params_len = 24;
            let status_len = 4;
            let right_cols = params_len + status_len;
            let name_cols = ((area.width / cw) as usize).saturating_sub(3 + right_cols);

            let label = format!("Ch {:<width$}", "Instrument", width = name_cols);
            renderer.draw_text(area.x, y, &label, dim, ch);

            let x = area.x + (3 + name_cols) as f32 * cw;
            let params = " Prg Vol    Pan  Exp  ";
            renderer.draw_text(x, y, params, dim, ch);
        }
        TrackViewMode::Detail => {
            renderer.draw_text(area.x, y, "  Track / Channel", dim, ch);
        }
    }
}

/// Default mode: 1-line per channel display for left panel.
///
/// `01:Piano1         P:00 V:100 C64 E127 PLAY`
fn render_default_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    let row_h = ch * ROW_HEIGHT;
    if area.width < cw * 10.0 || area.height < row_h {
        return;
    }

    let cs = &app.shared.channel_states;
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);
    let drum_mask = app.shared.drum_channels.load(Ordering::Relaxed);
    let active_channels = app.used_channels;
    let current_port = app.current_port;
    let port_offset = current_port as u64 * 16;

    let visible_rows = (area.height / row_h) as usize;

    // Scroll offset (1 line per channel)
    let scroll_offset = if app.track_cursor >= visible_rows {
        app.track_cursor - visible_rows + 1
    } else {
        0
    };

    // Params column starts after name area; status is right-aligned
    // Layout: "NN:Name        P:xxx V:xxx Cxx E:xxx STAT"
    let params_len = 25; // " P:xxx V:xxx Cxx E:xxx "
    let status_len = 4;  // "PLAY" / "MUTE"
    let right_cols = params_len + status_len;
    let name_cols = ((area.width / cw) as usize).saturating_sub(3 + right_cols); // 3 = "NN:"

    for ch_idx in 0..16u8 {
        let row = ch_idx as usize;
        if row < scroll_offset || row >= scroll_offset + visible_rows {
            continue;
        }

        let screen_row = row - scroll_offset;
        let y = area.y + screen_row as f32 * row_h;
        let flat_ch = port_offset + ch_idx as u64;
        let is_active = active_channels & (1u64 << flat_ch) != 0;
        let is_muted = muted_mask & (1u64 << flat_ch) != 0;
        let is_drum = drum_mask & (1u64 << flat_ch) != 0;
        let is_selected = app.track_cursor == ch_idx as usize;

        let ch_color = if is_muted {
            theme::MUTED_COLOR
        } else {
            theme::channel_color(ch_idx)
        };
        let inactive_color = Color::rgb(50, 50, 60);

        if is_selected {
            renderer.fill_rect(Rect::new(area.x, y, area.width, row_h), theme::SELECTED_BG);
        }

        if !is_active {
            let text = format!("{:>2}:---", ch_idx + 1);
            renderer.draw_text(area.x, y, &text, inactive_color, ch);
            continue;
        }

        let ci = flat_ch as usize;
        let mut x = area.x;

        // Channel number + instrument name
        let prog = cs.program[ci].load(Ordering::Relaxed);
        let inst_name = if is_drum {
            "Drums"
        } else {
            gm_instrument_name(prog as u8)
        };
        let truncated: String = inst_name.chars().take(name_cols).collect();
        let label = format!("{:>2}:{:<width$}", ch_idx + 1, truncated, width = name_cols);
        renderer.draw_text(x, y, &label, ch_color, ch);
        x += label.len() as f32 * cw;

        // Parameters: P, V, Pan, E
        let dim = if is_muted { theme::MUTED_COLOR } else { theme::HEADER_FG };
        let vol = cs.volume[ci].load(Ordering::Relaxed);
        let pan = cs.pan[ci].load(Ordering::Relaxed);
        let exp = cs.expression[ci].load(Ordering::Relaxed);

        let pan_str = if pan == 64 {
            "C64".to_string()
        } else if pan < 64 {
            format!("L{:>2}", 64 - pan)
        } else {
            format!("R{:>2}", pan - 64)
        };

        let prg_val = if is_drum {
            " Dr".to_string()
        } else {
            format!("{:>3}", prog)
        };

        let params = format!(" P:{} V:{:>3} {} E:{:>3} ", prg_val, vol, pan_str, exp);
        renderer.draw_text(x, y, &params, dim, ch);

        // Status indicator (right-aligned)
        let vel = cs.velocity[ci].load(Ordering::Relaxed);
        let status = if is_muted {
            "MUTE"
        } else if vel > 0 {
            "PLAY"
        } else {
            "    "
        };
        let status_color = if is_muted {
            Color::rgb(255, 80, 80)
        } else {
            Color::rgb(80, 200, 80)
        };
        let status_x = area.x + area.width - status_len as f32 * cw;
        if status_x > x {
            renderer.draw_text(status_x, y, status, status_color, ch);
        }
    }
}

/// Detail mode: per-track tree with scrolling.
fn render_detail_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    let row_h = ch * ROW_HEIGHT;
    if area.width < cw * 10.0 || area.height < row_h {
        return;
    }

    let tracks = app.shared.track_info.lock().unwrap();
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);

    let rows = build_detail_rows(&tracks);

    let visible_rows = (area.height / row_h) as usize;
    let scroll_offset = if app.track_cursor >= visible_rows {
        app.track_cursor - visible_rows + 1
    } else {
        0
    };

    let show_port = app.port_count > 1;
    let max_name_len = ((area.width / cw) as usize).saturating_sub(8);

    for (i, row) in rows.iter().enumerate().skip(scroll_offset).take(visible_rows) {
        let screen_y = area.y + (i - scroll_offset) as f32 * row_h;
        let is_selected = i == app.track_cursor;

        if is_selected {
            renderer.fill_rect(Rect::new(area.x, screen_y, area.width, row_h), theme::SELECTED_BG);
        }

        let prefix = if is_selected { "> " } else { "  " };

        match row {
            RawRow::TrackHeader { track_idx } => {
                let t = tracks.iter().find(|t| t.index == *track_idx).unwrap();
                let name = if t.name.is_empty() {
                    format!("Trk {}", t.index)
                } else {
                    t.name.chars().take(max_name_len).collect()
                };
                let port_label = if show_port {
                    format!("[P{}]", t.port + 1)
                } else {
                    String::new()
                };
                let text = format!("{}{}{}", prefix, port_label, name);
                renderer.draw_text_bold(area.x, screen_y, &text, theme::HEADER_FG, ch);
            }
            RawRow::Channel { port, channel, .. } => {
                let ch_idx = *channel;
                let flat_ch = *port as u64 * 16 + ch_idx as u64;
                let is_muted = muted_mask & (1u64 << flat_ch) != 0;
                let color = theme::channel_color(ch_idx);

                let fg = if is_muted { theme::MUTED_COLOR } else { color };
                let dot = if is_muted { "\u{25CB}" } else { "\u{25CF}" };

                let text = format!("{}  {} Ch{:>2}", prefix, dot, ch_idx + 1);
                renderer.draw_text(area.x, screen_y, &text, fg, ch);
            }
        }
    }
}
