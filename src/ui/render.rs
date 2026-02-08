//! Main rendering: layout computation and delegation to sub-components.

use crate::app::{App, AppScreen, TrackViewMode};
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::header::format_duration;
use crate::ui::help::render_help;
use crate::ui::layout::Layout;
use crate::ui::midi_monitor::render_midi_monitor;
use crate::ui::piano_roll::render_piano_roll;
use crate::ui::status_bar::render_status_bar;
use crate::ui::theme;
use crate::ui::track_list::{self, render_track_list};
use crate::ui::transport::render_transport;

pub fn render(renderer: &mut dyn Renderer, app: &mut App) {
    match app.screen {
        AppScreen::Player => render_player(renderer, app),
        AppScreen::FileBrowser => render_browser(renderer, app),
    }
}

fn render_player(renderer: &mut dyn Renderer, app: &mut App) {
    let (w, h) = renderer.window_size();
    let (cw, ch) = renderer.cell_size();
    let cols = (w as f32 / cw) as u16;
    let rows = (h as f32 / ch) as u16;

    if cols < 10 || rows < 8 {
        return;
    }

    // Compute minimum content rows needed for Channels section
    let channels_needed = match app.track_view_mode {
        TrackViewMode::Default => track_list::EXTENDED_MIN_CONTENT_ROWS,
        TrackViewMode::Detail => {
            let tracks = app.shared.track_info.lock().unwrap();
            track_list::raw_row_count(&tracks, app.port_count, app.current_port, app.track_view_mode) as u16
        }
    };

    let layout = Layout::compute(cols, rows, app.show_piano_roll, channels_needed, cw, ch);

    // Header: native rendered (filename larger, metadata normal, same row)
    render_header_native(renderer, app);

    // Channels title bar (native, TITLE_BAR_BG background)
    let track_title = match app.track_view_mode {
        TrackViewMode::Default => {
            if app.port_count > 1 {
                "Channels [Port]"
            } else {
                "Channels"
            }
        }
        TrackViewMode::Detail => "Channels [Detail]",
    };
    render_native_title_bar(renderer, layout.track_title_px, track_title);

    // Piano roll or MIDI monitor
    if let Some(pr_px) = layout.piano_roll_px {
        if app.midi_monitor {
            render_midi_monitor(renderer, pr_px, app);
        } else {
            render_piano_roll(renderer, pr_px, app, &app.note_rects, app.piano_roll_vertical);
        }
    }

    // Render track list, transport, status bar directly
    render_track_list(renderer, layout.track_list, app);
    render_transport(renderer, layout.transport, app);
    render_status_bar(renderer, layout.status_bar);

    // Help overlay drawn last (Z-order: on top of everything)
    if app.show_help {
        render_help(renderer);
    }
}

/// Header row: filename in larger font + metadata in normal font, single line.
fn render_header_native(renderer: &mut dyn Renderer, app: &App) {
    let (cw, ch) = renderer.cell_size();

    let mut x = cw;
    let y = (ch - (ch * 0.8)) / 2.0;

    // Filename (larger font)
    if !app.file_name.is_empty() {
        renderer.draw_text(x, y, &app.file_name, theme::HEADER_FG, ch);
        x += app.file_name.len() as f32 * cw;
    }

    // Metadata fields (normal font)
    let bpm = app.current_bpm();
    let (ts_num, ts_den) = app.time_signature();
    let sep = " | ";

    let mut fields = vec![
        format!("BPM: {:.1}", bpm),
        format!("{}/{}", ts_num, 1 << ts_den),
        format!("Notes: {}", app.total_notes),
        format!("Tracks: {}", app.track_count),
        format!("Duration: {}", format_duration(app.total_duration_secs)),
        format!("Res: {}", app.ticks_per_quarter),
        format!("SMF {}", app.format),
        format!("Mode: {}", app.midi_mode),
    ];

    if app.port_count > 1 {
        fields.push(format!("Ports: {}", app.port_count));
    }

    for field in &fields {
        renderer.draw_text(x, y, sep, theme::BORDER_COLOR, ch);
        x += sep.len() as f32 * cw;
        renderer.draw_text(x, y, field, theme::HEADER_FG, ch);
        x += field.len() as f32 * cw;
    }

    if !app.sf2_name.is_empty() {
        renderer.draw_text(x, y, sep, theme::BORDER_COLOR, ch);
        x += sep.len() as f32 * cw;
        let sf2 = format!("SF2: {}", app.sf2_name);
        renderer.draw_text(x, y, &sf2, theme::PROGRESS_FILLED, ch);
    }
}

/// Draw a native title bar with larger font and vertical centering.
fn render_native_title_bar(renderer: &mut dyn Renderer, rect: Rect, title: &str) {
    let (cw, ch) = renderer.cell_size();
    renderer.fill_rect(rect, theme::TITLE_BAR_BG);
    let font_size = ch * 1.02;
    let text_y = rect.y + (rect.height - font_size) / 2.0;
    renderer.draw_text(rect.x + cw, text_y, title, theme::HEADER_FG, font_size);
}

fn render_browser(renderer: &mut dyn Renderer, app: &mut App) {
    if let Some(ref mut browser) = app.file_browser {
        browser.render(renderer);
    }
}
