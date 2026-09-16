//! Transport bar: play/pause and stop buttons, large current-time readout,
//! bar/beat + BPM position, seek bar with bar ticks, total time, and volume.

use crate::app::App;
use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::ui::header::format_duration;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::{dot_size, px};
use crate::ui::text::text_width;
use crate::ui::theme;

/// Minimum seek bar width before the position block, then the total time,
/// are dropped to make room.
const MIN_SEEK_W: f32 = 80.0;

/// Decides whether the position block and total-time readout fit alongside
/// the seek bar. `content_width` is the full width available for buttons,
/// current time, position block, seek bar, total time and volume combined;
/// `fixed_always` is the width of the items that are never dropped (both
/// buttons, current time, volume). Drops the position block first, then the
/// total time, until the seek bar reaches `min_seek_w`. Returns
/// `(show_position, show_total, seek_w)`.
fn fit_transport(
    content_width: f32,
    fixed_always: f32,
    position_w: f32,
    total_w: f32,
    gap: f32,
    min_seek_w: f32,
) -> (bool, bool, f32) {
    let seek_w = |show_position: bool, show_total: bool| {
        let n_gaps = 4.0 + if show_position { 1.0 } else { 0.0 } + if show_total { 1.0 } else { 0.0 };
        let extra_w = if show_position { position_w } else { 0.0 } + if show_total { total_w } else { 0.0 };
        content_width - fixed_always - extra_w - gap * n_gaps
    };

    let both = seek_w(true, true);
    if both >= min_seek_w {
        return (true, true, both);
    }
    let no_position = seek_w(false, true);
    if no_position >= min_seek_w {
        return (false, true, no_position);
    }
    (false, false, seek_w(false, false))
}

/// Smallest of {1, 2, 4, 8, ...} such that consecutive bar ticks are spaced
/// at least `min_spacing_px` apart, given the average pixel width per bar.
fn bar_tick_step(px_per_bar: f32, min_spacing_px: f32) -> u32 {
    if px_per_bar <= 0.0 {
        return 1;
    }
    let mut k = 1u32;
    while (k as f32) * px_per_bar < min_spacing_px {
        k *= 2;
    }
    k
}

/// Right-pointing triangle built from 1-dot-wide columns of decreasing
/// height (no glyphs available for icons).
fn draw_play_icon(renderer: &mut dyn Renderer, rect: Rect, color: Color, dot: f32) {
    let target = (rect.width.min(rect.height) * 0.5).max(dot);
    let cols = ((target / dot).round() as usize).max(2);
    let x0 = rect.x + (rect.width - cols as f32 * dot) / 2.0;
    let cy = rect.y + rect.height / 2.0;
    for i in 0..cols {
        let h = (target * (cols - i) as f32 / cols as f32).max(dot);
        renderer.fill_rect(Rect::new(x0 + i as f32 * dot, cy - h / 2.0, dot, h), color);
    }
}

fn draw_pause_icon(renderer: &mut dyn Renderer, rect: Rect, color: Color, dot: f32) {
    let h = (rect.height * 0.5).max(dot);
    let bar_w = dot;
    let gap = dot;
    let total_w = 2.0 * bar_w + gap;
    let x0 = rect.x + (rect.width - total_w) / 2.0;
    let y = rect.y + (rect.height - h) / 2.0;
    renderer.fill_rect(Rect::new(x0, y, bar_w, h), color);
    renderer.fill_rect(Rect::new(x0 + bar_w + gap, y, bar_w, h), color);
}

fn draw_stop_icon(renderer: &mut dyn Renderer, rect: Rect, color: Color, _dot: f32) {
    let s = rect.width.min(rect.height) * 0.4;
    let x = rect.x + (rect.width - s) / 2.0;
    let y = rect.y + (rect.height - s) / 2.0;
    renderer.fill_rect(Rect::new(x, y, s, s), color);
}

/// Draws a transport button box (frame or filled, depending on hover) and
/// its icon, via `draw_icon`.
fn draw_transport_button(
    renderer: &mut dyn Renderer,
    rect: Rect,
    hovered: bool,
    dot: f32,
    draw_icon: fn(&mut dyn Renderer, Rect, Color, f32),
) {
    if hovered {
        renderer.fill_rect(rect, theme::FRAME);
        draw_icon(renderer, rect, theme::GROUND, dot);
    } else {
        renderer.fill_rect(rect, theme::GROUND);
        renderer.fill_rect(Rect::new(rect.x, rect.y, rect.width, dot), theme::FRAME);
        renderer.fill_rect(Rect::new(rect.x, rect.bottom() - dot, rect.width, dot), theme::FRAME);
        renderer.fill_rect(Rect::new(rect.x, rect.y, dot, rect.height), theme::FRAME);
        renderer.fill_rect(Rect::new(rect.right() - dot, rect.y, dot, rect.height), theme::FRAME);
        draw_icon(renderer, rect, theme::ACCENT, dot);
    }
}

pub fn render_transport(renderer: &mut dyn Renderer, area: Rect, app: &App, hits: &mut HitMap) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let pad = px(6.0, scale);
    let dot = dot_size(scale);
    let cy = area.y + area.height / 2.0;

    renderer.fill_rect(area, theme::GROUND);
    renderer.fill_rect(Rect::new(area.x, area.y, area.width, dot), theme::FRAME);
    renderer.fill_rect(Rect::new(area.x, area.bottom() - dot, area.width, dot), theme::FRAME);
    renderer.fill_rect(Rect::new(area.x, area.y, dot, area.height), theme::FRAME);
    renderer.fill_rect(Rect::new(area.right() - dot, area.y, dot, area.height), theme::FRAME);

    let content_left = area.x + dot + pad;
    let content_right = area.right() - dot - pad;
    let content_width = (content_right - content_left).max(0.0);

    let btn_side = (area.height - 2.0 * pad).max(0.0);

    // Current time (large font).
    let time_fs = (renderer.font_size() * 1.6).round();
    let time_str = format_duration(app.current_time_secs());
    let time_w = text_width(&time_str, cw) * (time_fs / renderer.font_size());

    // Position block (BAR/BPM), two normal-size lines.
    let (bar, beat) = app.bar_map.bar_at(app.current_tick());
    let bar_line = format!("BAR {:03}:{}", bar, beat);
    let bpm_line = format!("BPM {:.1}", app.current_bpm());
    let position_w = text_width(&bar_line, cw).max(text_width(&bpm_line, cw));

    // Total time (normal font).
    let total_str = format_duration(app.total_duration_secs);
    let total_w = text_width(&total_str, cw);

    // Volume block.
    let vol_label_w = text_width("VOL", cw);
    let block_w = (3.0 * dot).max((0.6 * cw).round());
    let vol_blocks_w = 10.0 * block_w + 9.0 * dot;
    let vol_w = vol_label_w + pad + vol_blocks_w;

    let fixed_always = 2.0 * btn_side + time_w + vol_w;
    let (show_position, show_total, seek_w) =
        fit_transport(content_width, fixed_always, position_w, total_w, pad, px(MIN_SEEK_W, scale));

    let mut x = content_left;

    // 1. Play/pause button.
    let play_rect = Rect::new(x, cy - btn_side / 2.0, btn_side, btn_side);
    let play_hovered = matches!(app.hover, Some(HitAction::PlayPause));
    let show_pause = app.is_playing() && !app.is_finished();
    draw_transport_button(
        renderer,
        play_rect,
        play_hovered,
        dot,
        if show_pause { draw_pause_icon } else { draw_play_icon },
    );
    hits.push(play_rect, HitAction::PlayPause);
    x += btn_side + pad;

    // 2. Stop button.
    let stop_rect = Rect::new(x, cy - btn_side / 2.0, btn_side, btn_side);
    let stop_hovered = matches!(app.hover, Some(HitAction::Stop));
    draw_transport_button(renderer, stop_rect, stop_hovered, dot, draw_stop_icon);
    hits.push(stop_rect, HitAction::Stop);
    x += btn_side + pad;

    // 3. Current time.
    // Line height is 1.2x the font size, matching how cell_height is measured.
    renderer.draw_text(x, cy - time_fs * 1.2 / 2.0, &time_str, theme::ACCENT, time_fs);
    x += time_w + pad;

    // 4. Position block.
    if show_position {
        let top = cy - ch;
        renderer.draw_text(x, top, &bar_line, theme::TEXT, ch);
        renderer.draw_text(x, top + ch, &bpm_line, theme::DIM, ch);
        x += position_w + pad;
    }

    // 5. Seek bar.
    let seek_w = seek_w.max(0.0);
    if seek_w > 0.0 {
        let bar_h = (ch * 0.6).round();
        let bar_y = cy - bar_h / 2.0;
        let bar_rect = Rect::new(x, bar_y, seek_w, bar_h);
        renderer.fill_dither(bar_rect, theme::DIM, Some(theme::GROUND));

        let progress = if app.total_ticks > 0 {
            (app.current_tick() as f64 / app.total_ticks as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled_w = progress as f32 * seek_w;
        if filled_w > 0.0 {
            renderer.fill_rect(Rect::new(x, bar_y, filled_w, bar_h), theme::FRAME);
        }

        // Bar ticks below the seek bar.
        let total_bars = app.bar_map.bar_at(app.total_ticks).0.max(1);
        let px_per_bar = seek_w / total_bars as f32;
        let k = bar_tick_step(px_per_bar, px(8.0, scale));
        let tick_y = bar_rect.bottom() + dot;
        let mut b = k;
        while b <= total_bars {
            let tick = app.bar_map.tick_of_bar(b);
            let frac = if app.total_ticks > 0 { tick as f64 / app.total_ticks as f64 } else { 0.0 };
            let tx = x + frac as f32 * seek_w;
            renderer.fill_rect(Rect::new(tx - dot / 2.0, tick_y, dot, 2.0 * dot), theme::DIM);
            b += k;
        }

        // Handle.
        let handle_w = 2.0 * dot;
        let handle_h = (ch * 1.1).round();
        let handle_x = x + filled_w - handle_w / 2.0;
        renderer.fill_rect(Rect::new(handle_x, cy - handle_h / 2.0, handle_w, handle_h), theme::TEXT);

        let slop = px(4.0, scale);
        hits.push(
            Rect::new(x, bar_rect.y - slop, seek_w, bar_h + 2.0 * slop),
            HitAction::SeekBar { x0: x, width: seek_w, total_ticks: app.total_ticks },
        );
    }
    x += seek_w + pad;

    // 6. Total time.
    if show_total {
        renderer.draw_text(x, cy - ch / 2.0, &total_str, theme::TEXT, ch);
        x += total_w + pad;
    }

    // 7. Volume.
    renderer.draw_text(x, cy - ch / 2.0, "VOL", theme::DIM, ch);
    x += vol_label_w + pad;

    let vol = app.volume();
    let lit = vol.div_ceil(10).min(10);
    let block_h = (ch * 0.8).round();
    let block_y = cy - block_h / 2.0;
    let vol_x0 = x;
    for i in 0..10u32 {
        let bx = vol_x0 + i as f32 * (block_w + dot);
        let color = if i < lit {
            if i >= 8 { theme::ACCENT } else { theme::FRAME }
        } else {
            theme::WELL
        };
        renderer.fill_rect(Rect::new(bx, block_y, block_w, block_h), color);
    }
    hits.push(
        Rect::new(vol_x0, block_y, vol_blocks_w, block_h),
        HitAction::Volume { x0: vol_x0, block_w, gap: dot },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_transport_shows_everything_when_wide() {
        let (pos, tot, seek) = fit_transport(1000.0, 300.0, 100.0, 80.0, 10.0, 80.0);
        assert!(pos && tot);
        assert!(seek >= 80.0);
    }

    #[test]
    fn fit_transport_drops_position_first() {
        let (pos, tot, seek) = fit_transport(560.0, 300.0, 100.0, 80.0, 10.0, 80.0);
        assert!(!pos);
        assert!(tot);
        assert!(seek >= 80.0);
    }

    #[test]
    fn fit_transport_drops_both_when_very_narrow() {
        let (pos, tot, seek) = fit_transport(350.0, 300.0, 100.0, 80.0, 10.0, 80.0);
        assert!(!pos);
        assert!(!tot);
        assert!(seek < 80.0); // still can't fit, but this is the best available
        assert!(seek > 0.0);
    }

    #[test]
    fn bar_tick_step_picks_smallest_power_of_two_covering_spacing() {
        assert_eq!(bar_tick_step(10.0, 8.0), 1);
        assert_eq!(bar_tick_step(3.0, 8.0), 4);
        assert_eq!(bar_tick_step(1.0, 8.0), 8);
    }

    #[test]
    fn bar_tick_step_handles_zero_px_per_bar() {
        assert_eq!(bar_tick_step(0.0, 8.0), 1);
    }
}
