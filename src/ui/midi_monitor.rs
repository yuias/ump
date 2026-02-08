//! MIDI Monitor: tracker-style real-time per-channel note data display.
//! Combines note event data (Location, Note, ST, GT, Vel) with channel
//! state parameters (Prg, Vol, Pan, Exp, Mod, Bnd, Ped, Bnk, Rev, Cho).
//! Displays channels for the current port in multi-port mode.

use std::sync::atomic::Ordering;

use crate::app::App;
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::ui::layout::TITLE_BAR_HEIGHT;
use crate::ui::piano_roll::key_name_parts;
use crate::ui::theme;

/// Column widths in cell units.
const W_CH: f32 = 3.0;
const W_LOC: f32 = 12.0;
const W_NOTE: f32 = 5.0;
const W_NUM: f32 = 5.0; // ST, GT, Vel, and channel-state numeric columns
const W_NARROW: f32 = 4.0; // compact columns (Pan, Bnd, Ped)

/// Header label color (dim red, tracker style).
const HEADER_COLOR: Color = Color::rgb(180, 80, 80);
/// Separator color between note data and channel state groups.
const GROUP_SEP_COLOR: Color = Color::rgb(60, 50, 50);
/// Dim color for inactive / no-data cells.
const DIM_COLOR: Color = Color::rgb(80, 80, 80);

/// Column definition: (header label, width in cell units).
const COLUMNS: &[(&str, f32)] = &[
    // Note event group
    ("Ch",  W_CH),
    ("Location", W_LOC),
    ("Note", W_NOTE),
    ("ST",  W_NUM),
    ("GT",  W_NUM),
    ("Vel", W_NARROW),
    // Channel state group
    ("Prg", W_NARROW),
    ("Vol", W_NARROW),
    ("Pan", W_NARROW),
    ("Exp", W_NARROW),
    ("Mod", W_NARROW),
    ("Bnd", W_NARROW),
    ("Ped", W_NARROW),
    ("Bnk", W_NARROW),
    ("Rev", W_NARROW),
    ("Cho", W_NARROW),
];

/// Index where channel-state group starts (after Vel).
const STATE_GROUP_START: usize = 6;

/// Render the MIDI monitor panel in the given area.
pub fn render_midi_monitor(
    renderer: &mut dyn Renderer,
    area: Rect,
    app: &App,
) {
    if area.width < 40.0 || area.height < 20.0 {
        return;
    }

    let (cw, ch) = renderer.cell_size();

    // Clear background
    renderer.fill_rect(area, crate::renderer::types::BG_COLOR);

    // Title bar
    let title_h = ch * TITLE_BAR_HEIGHT;
    let title_bar = Rect::new(area.x, area.y, area.width, title_h);
    renderer.fill_rect(title_bar, theme::TITLE_BAR_BG);
    let font_size = ch * 1.02;
    let text_y = area.y + (title_h - font_size) / 2.0;
    let title = if app.port_count > 1 {
        format!("MIDI Monitor [P{}]", app.current_port + 1)
    } else {
        "MIDI Monitor".to_string()
    };
    renderer.draw_text(area.x + cw, text_y, &title, theme::HEADER_FG, font_size);

    // Content area
    let content_y = area.y + title_h;
    let row_h = ch * 1.15;
    let data_font = ch * 0.85;

    // Compute column X positions
    let base_x = area.x + cw * 0.5;
    let mut col_x: Vec<f32> = Vec::with_capacity(COLUMNS.len());
    let mut x = base_x;
    for &(_, w) in COLUMNS {
        col_x.push(x);
        x += cw * w;
    }

    // Determine how many columns fit
    let visible_cols = col_x
        .iter()
        .enumerate()
        .take_while(|&(_, &cx)| cx < area.x + area.width - cw * 2.0)
        .count()
        .min(COLUMNS.len());

    // Draw column headers
    let header_y = content_y + (row_h - data_font) / 2.0;
    for i in 0..visible_cols {
        renderer.draw_text(col_x[i], header_y, COLUMNS[i].0, HEADER_COLOR, data_font);
    }

    // Group separator (vertical line between note data and channel state)
    if visible_cols > STATE_GROUP_START {
        let sep_x = col_x[STATE_GROUP_START] - cw * 0.3;
        renderer.draw_vline(sep_x, content_y, area.y + area.height, GROUP_SEP_COLOR, 1.0);
    }

    // Header separator line
    let sep_y = content_y + row_h;
    renderer.draw_hline(area.x, area.x + area.width, sep_y, theme::BORDER_COLOR, 1.0);

    // Time signature for location calculation
    let ts_num = app.shared.time_sig_num.load(Ordering::Relaxed).max(1);
    let ts_den_pow = app.shared.time_sig_den.load(Ordering::Relaxed);
    let tpq = app.ticks_per_quarter as u32;

    let current_port = app.current_port;
    let port_offset = current_port as u64 * 16;
    let used = app.used_channels;
    let muted_mask = app.shared.muted_channels.load(Ordering::Relaxed);
    let drum_mask = app.shared.drum_channels.load(Ordering::Relaxed);
    let mut row = 0u16;

    for ch_idx in 0..16u8 {
        let flat_ch = port_offset + ch_idx as u64;
        if used & (1u64 << flat_ch) == 0 {
            continue;
        }

        let y = sep_y + row as f32 * row_h;
        if y + row_h > area.y + area.height {
            break;
        }

        let text_y = y + (row_h - data_font) / 2.0;
        let ci = flat_ch as usize;

        let muted = muted_mask & (1u64 << flat_ch) != 0;
        let ch_color = if muted {
            theme::MUTED_COLOR
        } else {
            theme::channel_color(ch_idx)
        };
        let val_color = if muted { theme::MUTED_COLOR } else { theme::HEADER_FG };

        let mon = &app.shared.monitor;
        let cs = &app.shared.channel_states;
        let key_raw = mon.note_key[ci].load(Ordering::Relaxed);
        let has_data = key_raw != 0xFFFF;

        // Helper: draw a column value if it fits
        let mut draw_col = |col: usize, text: &str, color: Color| {
            if col < visible_cols {
                renderer.draw_text(col_x[col], text_y, text, color, data_font);
            }
        };

        // Ch
        draw_col(0, &format!("{:>2}", ch_idx + 1), ch_color);

        if has_data {
            let vel = mon.note_vel[ci].load(Ordering::Relaxed);
            let tick = mon.note_tick[ci].load(Ordering::Relaxed);
            let st = mon.step_time[ci].load(Ordering::Relaxed);
            let gt = mon.gate_time[ci].load(Ordering::Relaxed);

            // Location
            draw_col(1, &tick_to_location(tick, tpq, ts_num, ts_den_pow), ch_color);

            // Note
            let (note_name, octave) = key_name_parts(key_raw as u8);
            draw_col(2, &format!("{:<2}{}", note_name, octave), ch_color);

            // ST, GT, Vel
            draw_col(3, &format!("{:>4}", st), ch_color);
            draw_col(4, &format!("{:>4}", gt), ch_color);
            draw_col(5, &format!("{:>3}", vel), ch_color);
        } else {
            draw_col(1, "---", DIM_COLOR);
        }

        // Channel state columns (always shown if channel is active)
        // Prg
        let prog = cs.program[ci].load(Ordering::Relaxed);
        let prg_str = if drum_mask & (1u64 << flat_ch) != 0 {
            " Dr".to_string()
        } else {
            format!("{:>3}", prog)
        };
        draw_col(6, &prg_str, val_color);

        // Vol
        draw_col(7, &format!("{:>3}", cs.volume[ci].load(Ordering::Relaxed)), val_color);

        // Pan
        let pan = cs.pan[ci].load(Ordering::Relaxed);
        let pan_str = if pan == 64 {
            " C ".to_string()
        } else if pan < 64 {
            format!("L{:>2}", 64 - pan)
        } else {
            format!("R{:>2}", pan - 64)
        };
        draw_col(8, &pan_str, val_color);

        // Exp
        draw_col(9, &format!("{:>3}", cs.expression[ci].load(Ordering::Relaxed)), val_color);

        // Mod
        draw_col(10, &format!("{:>3}", cs.modulation[ci].load(Ordering::Relaxed)), val_color);

        // Bnd
        let bend = cs.pitch_bend[ci].load(Ordering::Relaxed) as i32 as i16 as i32;
        let bnd_str = if bend == 0 {
            "  0".to_string()
        } else {
            format!("{:>+3}", (bend * 100 / 8192).clamp(-99, 99))
        };
        draw_col(11, &bnd_str, val_color);

        // Ped
        let ped = cs.pedal[ci].load(Ordering::Relaxed);
        draw_col(12, if ped >= 64 { " On" } else { "Off" }, val_color);

        // Bnk
        draw_col(13, &format!("{:>3}", cs.bank[ci].load(Ordering::Relaxed)), val_color);

        // Rev
        draw_col(14, &format!("{:>3}", cs.reverb[ci].load(Ordering::Relaxed)), val_color);

        // Cho
        draw_col(15, &format!("{:>3}", cs.chorus[ci].load(Ordering::Relaxed)), val_color);

        // Row separator
        let row_sep_y = y + row_h;
        if row_sep_y < area.y + area.height {
            renderer.draw_hline(
                area.x,
                area.x + area.width,
                row_sep_y,
                Color::rgb(40, 40, 40),
                1.0,
            );
        }

        row += 1;
    }
}

/// Convert an absolute tick to "Measure.Beat.Tick" string.
fn tick_to_location(tick: u32, tpq: u32, ts_num: u32, ts_den_pow: u32) -> String {
    if tpq == 0 || ts_num == 0 {
        return format!("{}", tick);
    }
    let denom_val = 1u32 << ts_den_pow;
    let ticks_per_beat = tpq * 4 / denom_val;
    let ticks_per_measure = ticks_per_beat * ts_num;

    if ticks_per_measure == 0 || ticks_per_beat == 0 {
        return format!("{}", tick);
    }

    let measure = tick / ticks_per_measure + 1;
    let remaining = tick % ticks_per_measure;
    let beat = remaining / ticks_per_beat + 1;
    let sub_tick = remaining % ticks_per_beat;

    format!("{:>4}.{:>2}.{:>03}", measure, beat, sub_tick)
}
