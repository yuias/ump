//! Track list panel: Default mode (per-channel grid) and Detail mode (per-track tree).

use std::sync::atomic::Ordering;

use crate::app::{App, TrackViewMode};
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::state::TrackInfoSnapshot;
use crate::synth::gm::gm_instrument_name;
use crate::ui::theme;

/// Minimum content rows for the Default (extended) view:
/// Ch header(1) + param rows(8) + separator(1) + activity bars(3) = 13
pub const EXTENDED_MIN_CONTENT_ROWS: u16 = 13;

/// Represents a single row in the flattened Detail view.
#[derive(Debug, Clone)]
pub enum RawRow {
    TrackHeader { track_idx: usize },
    Channel { track_idx: usize, channel: u8 },
}

/// Build the flat row list for the Detail tree view.
pub fn build_raw_rows(tracks: &[TrackInfoSnapshot]) -> Vec<RawRow> {
    let mut rows = Vec::new();
    for t in tracks {
        if t.note_count == 0 && t.name.is_empty() {
            continue;
        }
        rows.push(RawRow::TrackHeader { track_idx: t.index });
        for ch in 0..16u8 {
            if t.channel_note_counts[ch as usize] > 0 {
                rows.push(RawRow::Channel {
                    track_idx: t.index,
                    channel: ch,
                });
            }
        }
    }
    rows
}

/// Return the total number of rows in the Detail tree view.
pub fn raw_row_count(tracks: &[TrackInfoSnapshot]) -> usize {
    build_raw_rows(tracks).len()
}

pub fn render_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    match app.track_view_mode {
        TrackViewMode::Default => render_extended_track_list(renderer, area, app),
        TrackViewMode::Detail => render_raw_track_list(renderer, area, app),
    }
}

fn render_raw_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    if area.width < cw * 10.0 || area.height < ch {
        return;
    }

    let tracks = app.shared.track_info.lock().unwrap();
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);

    let rows = build_raw_rows(&tracks);

    // Compute scroll offset
    let visible_rows = (area.height / ch) as usize;
    let scroll_offset = if app.track_cursor >= visible_rows {
        app.track_cursor - visible_rows + 1
    } else {
        0
    };

    for (i, row) in rows.iter().enumerate().skip(scroll_offset).take(visible_rows) {
        let screen_y = area.y + (i - scroll_offset) as f32 * ch;
        let is_selected = i == app.track_cursor;

        // Selected row background
        if is_selected {
            renderer.fill_rect(Rect::new(area.x, screen_y, area.width, ch), theme::SELECTED_BG);
        }

        let prefix = if is_selected { "> " } else { "  " };

        match row {
            RawRow::TrackHeader { track_idx } => {
                let t = tracks.iter().find(|t| t.index == *track_idx).unwrap();
                let name = if t.name.is_empty() {
                    format!("Track {}", t.index)
                } else {
                    t.name.clone()
                };
                let text = format!("{}{:<24}({} notes)", prefix, name, t.note_count);
                renderer.draw_text_bold(area.x, screen_y, &text, theme::HEADER_FG, ch);
            }
            RawRow::Channel { track_idx, channel } => {
                let ch_idx = *channel;
                let is_muted = muted_mask & (1 << ch_idx) != 0;
                let color = theme::channel_color(ch_idx);

                let t = tracks.iter().find(|t| t.index == *track_idx).unwrap();
                let note_count = t.channel_note_counts[ch_idx as usize];
                let instrument = if ch_idx == 9 {
                    "Drums".to_string()
                } else {
                    t.channel_programs[ch_idx as usize]
                        .map(|p| gm_instrument_name(p).to_string())
                        .unwrap_or_default()
                };

                let fg = if is_muted { theme::MUTED_COLOR } else { color };
                let dim = if is_muted { theme::MUTED_COLOR } else { theme::HEADER_FG };
                let dot = if is_muted { "\u{25CB}" } else { "\u{25CF}" };
                let mute_indicator = if is_muted { "M" } else { " " };

                let mut x = area.x;
                let dot_text = format!("{}  {} ", prefix, dot);
                renderer.draw_text(x, screen_y, &dot_text, fg, ch);
                x += dot_text.len() as f32 * cw;

                let ch_text = format!("Ch {:>2}  ", ch_idx + 1);
                renderer.draw_text(x, screen_y, &ch_text, fg, ch);
                x += ch_text.len() as f32 * cw;

                let inst_text = format!("{:<24}", instrument);
                renderer.draw_text(x, screen_y, &inst_text, dim, ch);
                x += inst_text.len() as f32 * cw;

                let count_text = format!("{:>5} ", note_count);
                renderer.draw_text(x, screen_y, &count_text, dim, ch);
                x += count_text.len() as f32 * cw;

                let mute_fg = if is_muted { Color::rgb(255, 80, 80) } else { theme::BORDER_COLOR };
                renderer.draw_text(x, screen_y, mute_indicator, mute_fg, ch);
            }
        }
    }
}

fn render_extended_track_list(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    if area.width < cw * 20.0 || area.height < ch * 3.0 {
        return;
    }

    let cs = &app.shared.channel_states;
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);
    let active_channels = app.used_channels as u32;

    let label_width = 4.0 * cw;
    let col_width_px = (area.width - label_width) / 16.0;
    if col_width_px < cw * 3.0 {
        return;
    }

    let label_style_fg = theme::HEADER_FG;
    let dim_fg = theme::BORDER_COLOR;
    let inactive_fg = Color::rgb(40, 40, 50);

    // Row 0: Channel number header
    let header_y = area.y;
    renderer.draw_text(area.x, header_y, " Ch", label_style_fg, ch);
    for ch_idx in 0..16u8 {
        let x = area.x + label_width + ch_idx as f32 * col_width_px;
        let is_active = active_channels & (1 << ch_idx) != 0;
        let is_muted = muted_mask & (1 << ch_idx) != 0;
        let fg = if is_muted {
            theme::MUTED_COLOR
        } else if is_active {
            theme::channel_color(ch_idx)
        } else {
            inactive_fg
        };
        let val = format!("{:>width$}", ch_idx + 1, width = ((col_width_px / cw) as usize).min(4));
        if is_active && !is_muted {
            renderer.draw_text_bold(x, header_y, &val, fg, ch);
        } else {
            renderer.draw_text(x, header_y, &val, fg, ch);
        }
    }

    let drum_mask = app.shared.drum_channels.load(Ordering::Relaxed);

    let param_labels = ["Prg", "Vol", "Pan", "Exp", "Mod", "Bnd", "Ped", "AT"];
    let param_fns: &[&dyn Fn(u8) -> String] = &[
        &|ch_idx: u8| {
            let p = cs.program[ch_idx as usize].load(Ordering::Relaxed);
            if drum_mask & (1 << ch_idx) != 0 {
                "Dr".to_string()
            } else {
                format!("{:>3}", p)
            }
        },
        &|ch_idx: u8| {
            format!("{:>3}", cs.volume[ch_idx as usize].load(Ordering::Relaxed))
        },
        &|ch_idx: u8| {
            let v = cs.pan[ch_idx as usize].load(Ordering::Relaxed);
            if v == 64 {
                " C ".to_string()
            } else if v < 64 {
                format!("L{:>2}", 64 - v)
            } else {
                format!("R{:>2}", v - 64)
            }
        },
        &|ch_idx: u8| {
            format!("{:>3}", cs.expression[ch_idx as usize].load(Ordering::Relaxed))
        },
        &|ch_idx: u8| {
            format!("{:>3}", cs.modulation[ch_idx as usize].load(Ordering::Relaxed))
        },
        &|ch_idx: u8| {
            let v = cs.pitch_bend[ch_idx as usize].load(Ordering::Relaxed) as i32 as i16 as i32;
            if v == 0 {
                "  0".to_string()
            } else {
                format!("{:>+3}", (v * 100 / 8192).clamp(-99, 99))
            }
        },
        &|ch_idx: u8| {
            let v = cs.pedal[ch_idx as usize].load(Ordering::Relaxed);
            if v >= 64 { " On".to_string() } else { "Off".to_string() }
        },
        &|ch_idx: u8| {
            let v = cs.aftertouch[ch_idx as usize].load(Ordering::Relaxed);
            format!("{:>3}", v)
        },
    ];

    for (row_idx, (label, value_fn)) in param_labels.iter().zip(param_fns.iter()).enumerate() {
        let y = area.y + (1 + row_idx) as f32 * ch;
        if y + ch > area.y + area.height {
            break;
        }

        renderer.draw_text(area.x, y, label, dim_fg, ch);

        for ch_idx in 0..16u8 {
            let x = area.x + label_width + ch_idx as f32 * col_width_px;
            let is_active = active_channels & (1 << ch_idx) != 0;
            let is_muted = muted_mask & (1 << ch_idx) != 0;

            if !is_active {
                continue;
            }

            let val = value_fn(ch_idx);
            let fg = if is_muted { theme::MUTED_COLOR } else { theme::HEADER_FG };
            let display_w = (col_width_px / cw) as usize;
            let s = format!("{:>width$}", val, width = display_w.min(4));
            renderer.draw_text(x, y, &s, fg, ch);
        }
    }

    // Separator line
    let sep_y = area.y + (1 + param_labels.len()) as f32 * ch;
    if sep_y < area.y + area.height {
        let line_right = (area.x + label_width + 16.0 * col_width_px).min(area.right());
        renderer.draw_hline(sep_y + ch * 0.5, area.x, line_right, theme::BORDER_COLOR, 1.0);
    }

    // Activity bars
    let bar_y_start = sep_y + ch;
    if bar_y_start < area.y + area.height {
        let bar_height_px = area.y + area.height - bar_y_start;
        let bottom = area.y + area.height;

        for ch_idx in 0..16u8 {
            let x = area.x + label_width + ch_idx as f32 * col_width_px;
            let is_active = active_channels & (1 << ch_idx) != 0;
            let is_muted = muted_mask & (1 << ch_idx) != 0;

            if !is_active {
                continue;
            }

            let vel = cs.velocity[ch_idx as usize].load(Ordering::Relaxed);
            if vel == 0 {
                continue;
            }

            let filled_ratio = (vel as f32 / 127.0).clamp(0.0, 1.0);
            let filled_h = filled_ratio * bar_height_px;

            let color = if is_muted {
                theme::MUTED_COLOR
            } else {
                theme::channel_color(ch_idx)
            };

            let bar_w = (col_width_px * 0.6).min(cw * 3.0);
            renderer.fill_rect(
                Rect::new(x, bottom - filled_h, bar_w, filled_h),
                color,
            );
        }
    }
}
