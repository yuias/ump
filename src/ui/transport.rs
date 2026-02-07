//! Transport bar: play/pause indicator, current time, progress bar, total time, volume.

use crate::app::App;
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::header::format_duration;
use crate::ui::theme;

pub fn render_transport(renderer: &mut dyn Renderer, area: Rect, app: &App) {
    let (cw, ch) = renderer.cell_size();
    let fg = theme::HEADER_FG;

    let mut x = area.x;
    let y = area.y;

    // Play status icon (shows next action: ▶ when paused, ⏸ when playing)
    let icon = if app.is_playing() {
        if app.is_finished() {
            " \u{25A0} "
        } else {
            " \u{23F8} "
        }
    } else {
        " \u{25B6} "
    };
    renderer.draw_text_bold(x, y, icon, fg, ch);
    x += 3.0 * cw;

    // Current time
    let current = format!(" {} ", format_duration(app.current_time_secs()));
    renderer.draw_text(x, y, &current, fg, ch);
    x += 14.0 * cw;
    x += 1.0 * cw; // gap

    // Progress bar
    let progress = if app.total_duration_secs > 0.0 {
        (app.current_time_secs() / app.total_duration_secs).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let total_width = area.width;
    let icon_w = 3.0 * cw;
    let time_w = 14.0 * cw;
    let gap = 1.0 * cw;
    let vol_w = 17.0 * cw;
    let progress_w = (total_width - icon_w - time_w - gap - gap - time_w - vol_w).max(0.0);

    if progress_w > 0.0 {
        let filled_w = (progress as f32) * progress_w;
        let empty_w = progress_w - filled_w;

        if filled_w > 0.0 {
            renderer.fill_rect(
                Rect::new(x, y, filled_w, ch),
                theme::PROGRESS_FILLED,
            );
        }
        if empty_w > 0.0 {
            renderer.fill_rect(
                Rect::new(x + filled_w, y, empty_w, ch),
                theme::PROGRESS_EMPTY,
            );
        }
        x += progress_w;
    }
    x += 1.0 * cw; // gap

    // Total time
    let total = format!(" {} ", format_duration(app.total_duration_secs));
    renderer.draw_text(x, y, &total, fg, ch);
    x += 14.0 * cw;

    // Volume
    let vol = app.volume();
    let vol_bar_len = 8.0 * cw;
    let filled_ratio = (vol as f32) / 100.0;
    let filled_w = filled_ratio * vol_bar_len;
    let empty_w = vol_bar_len - filled_w;

    renderer.draw_text(x, y, " Vol ", fg, ch);
    x += 5.0 * cw;
    x += 1.0 * cw; // gap

    if filled_w > 0.0 {
        renderer.fill_rect(
            Rect::new(x, y, filled_w, ch),
            theme::PROGRESS_FILLED,
        );
    }
    if empty_w > 0.0 {
        renderer.fill_rect(
            Rect::new(x + filled_w, y, empty_w, ch),
            theme::PROGRESS_EMPTY,
        );
    }
}
