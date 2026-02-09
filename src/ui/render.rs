//! Main rendering: layout computation and delegation to sub-components.

use unicode_width::UnicodeWidthChar;

use crate::app::{App, AppScreen, RightPanelMode};
use crate::renderer::Renderer;
use crate::ui::border::draw_border;
use crate::ui::header::format_duration;
use crate::ui::help::render_help;
use crate::ui::layout::Layout;
use crate::ui::midi_monitor::render_midi_monitor;
use crate::ui::piano_roll::render_piano_roll;
use crate::ui::status_bar::render_status_bar;
use crate::ui::theme;
use crate::ui::track_list::render_track_list;
use crate::ui::transport::render_transport;

/// Maximum display width for filename in cell units.
const MAX_NAME_CELLS: usize = 40;
/// Pause at start/end of scroll cycle (seconds).
const SCROLL_PAUSE_SECS: f64 = 3.0;
/// Scroll speed (cells per second).
const SCROLL_SPEED: f64 = 4.0;
/// Gap between end and restart of looping text (cell units).
const SCROLL_GAP: usize = 6;

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

    let layout = Layout::compute(cols, rows, cw, ch);

    // Header
    render_header_native(renderer, app);

    // Left panel: TRACK
    let left_title = match app.track_view_mode {
        crate::app::TrackViewMode::Default => {
            if app.port_count > 1 {
                format!(" TRACK [P{}] ", app.current_port + 1)
            } else {
                " TRACK ".to_string()
            }
        }
        crate::app::TrackViewMode::Detail => " TRACK [Detail] ".to_string(),
    };
    draw_border(renderer, layout.left_panel, &left_title, theme::BORDER_COLOR);
    render_track_list(renderer, layout.left_content, app);

    // Right panel: Monitor or PianoRoll
    let right_title = match app.right_panel_mode {
        RightPanelMode::Monitor => {
            if app.port_count > 1 {
                format!(" EVENT MONITOR [P{}] ", app.current_port + 1)
            } else {
                " EVENT MONITOR ".to_string()
            }
        }
        RightPanelMode::PianoRoll => {
            if app.piano_roll_vertical {
                " PIANO ROLL [V] ".to_string()
            } else {
                " PIANO ROLL ".to_string()
            }
        }
    };
    draw_border(renderer, layout.right_panel, &right_title, theme::BORDER_COLOR);

    match app.right_panel_mode {
        RightPanelMode::Monitor => {
            render_midi_monitor(renderer, layout.right_content, app);
        }
        RightPanelMode::PianoRoll => {
            render_piano_roll(
                renderer,
                layout.right_content,
                app,
                &app.note_rects,
                app.piano_roll_vertical,
            );
        }
    }

    // Transport and status bar
    render_transport(renderer, layout.transport, app);
    render_status_bar(renderer, layout.status_bar);

    // Help overlay drawn last (Z-order: on top of everything)
    if app.show_help {
        render_help(renderer);
    }
}

/// Header row: filename (scrolling if long) + metadata, single line.
fn render_header_native(renderer: &mut dyn Renderer, app: &App) {
    let (cw, ch) = renderer.cell_size();

    let mut x = cw;
    let y = (ch - (ch * 0.8)) / 1.25;

    // Filename (with marquee scroll for long names)
    if !app.file_name.is_empty() {
        let name_width = display_width(&app.file_name);

        if name_width <= MAX_NAME_CELLS {
            renderer.draw_text(x, y, &app.file_name, theme::HEADER_FG, ch);
            x += name_width as f32 * cw;
        } else {
            // Marquee scroll: pause → scroll → loop
            let elapsed = app.load_time.elapsed().as_secs_f64();
            let total_scroll = name_width + SCROLL_GAP;
            let scroll_secs = total_scroll as f64 / SCROLL_SPEED;
            let cycle = SCROLL_PAUSE_SECS + scroll_secs;
            let t = elapsed % cycle;

            let offset = if t < SCROLL_PAUSE_SECS {
                0
            } else {
                ((t - SCROLL_PAUSE_SECS) * SCROLL_SPEED) as usize
            };

            let gap = " ".repeat(SCROLL_GAP);
            let looping = format!("{}{}{}", app.file_name, gap, app.file_name);
            let visible = visible_slice(&looping, offset, MAX_NAME_CELLS - 2);
            renderer.draw_text(x, y, &visible, theme::HEADER_FG, ch);
            x += MAX_NAME_CELLS as f32 * cw;
        }
    }

    // Metadata fields (all ASCII — .len() is correct)
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

/// Display width of a string (CJK = 2 cells, ASCII = 1 cell).
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

/// Extract visible substring at a given cell offset with max visible width.
/// Characters that don't fully fit within the window are skipped.
fn visible_slice(text: &str, offset_cells: usize, max_cells: usize) -> String {
    let mut result = String::new();
    let mut pos = 0usize;
    let end = offset_cells + max_cells;

    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if pos >= offset_cells && pos + w <= end {
            result.push(ch);
        }
        pos += w;
        if pos >= end {
            break;
        }
    }

    result
}

fn render_browser(renderer: &mut dyn Renderer, app: &mut App) {
    if let Some(ref mut browser) = app.file_browser {
        browser.render(renderer);
    }
}
