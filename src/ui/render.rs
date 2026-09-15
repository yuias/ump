//! Main rendering: layout computation and delegation to sub-components.

use unicode_width::UnicodeWidthChar;

use crate::app::{App, AppScreen};
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::border::draw_panel;
use crate::ui::header::{
    fit_header_items, format_thousands, truncate_sf2_name, HeaderItem, HeaderItemKind,
};
use crate::ui::help::render_help;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::{dot_size, px, Layout};
use crate::ui::piano_roll::render_piano_roll;
use crate::ui::status_bar::render_status_bar;
use crate::ui::text::{text_cells, text_width};
use crate::ui::theme;
use crate::ui::track_list::render_track_list;
use crate::ui::transport::render_transport;

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
    app.update_channel_levels();

    let (w, h) = renderer.window_size();
    let (cw, ch) = renderer.cell_size();

    if (w as f32) < 20.0 * cw || (h as f32) < 10.0 * ch {
        return;
    }

    let mut hits = std::mem::take(&mut app.hit_map);
    hits.clear();

    let scale = renderer.scale_factor();
    let layout = Layout::compute(w as f32, h as f32, cw, ch, scale);

    // Header
    render_header_native(renderer, layout.header, app, &mut hits);

    // Left panel: TRACK
    let left_title = match app.track_view_mode {
        crate::app::TrackViewMode::Default => "TRACK".to_string(),
        crate::app::TrackViewMode::Detail => "TRACK [Detail]".to_string(),
    };
    draw_panel(renderer, layout.left_panel, &left_title, layout.title_h);
    if app.port_count > 1 {
        render_port_tabs(renderer, layout.left_panel, layout.title_h, app, &mut hits);
    }
    render_track_list(renderer, layout.left_content, app, &mut hits);

    // Right panel: Piano Roll
    let right_title = if app.piano_roll_vertical {
        "PIANO ROLL [V]".to_string()
    } else {
        "PIANO ROLL".to_string()
    };
    draw_panel(renderer, layout.right_panel, &right_title, layout.title_h);

    render_piano_roll(
        renderer,
        layout.right_content,
        app,
        &app.note_rects,
        app.piano_roll_vertical,
    );

    // Transport and function-key hint bar
    render_transport(renderer, layout.transport, app);
    render_status_bar(renderer, layout.fkey_bar);

    // Help overlay: rendered in a separate layer so its background
    // correctly covers the base layer text (wgpu z-order fix).
    if app.show_help {
        renderer.begin_overlay();
        render_help(renderer);
    }

    app.hit_map = hits;
}

/// Minimum filename width reserved when trimming metadata to fit.
const HEADER_MIN_NAME_CELLS: f32 = 12.0;

/// Header row: PC-98-style bar with an accent logo block, filename (scrolling
/// if long), and right-aligned metadata that drops items in priority order
/// when the window is too narrow to show everything.
fn render_header_native(renderer: &mut dyn Renderer, area: Rect, app: &App, hits: &mut HitMap) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let pad = px(6.0, scale);
    let y = area.y + (area.height - ch) / 2.0;

    renderer.fill_rect(area, theme::BAR_BG);

    // Logo block: accent-filled, "ump" centered inside.
    let logo_text_w = text_width("ump", cw);
    let logo_w = logo_text_w + 2.0 * pad;
    renderer.fill_rect(Rect::new(area.x, area.y, logo_w, area.height), theme::ACCENT);
    let logo_x = area.x + (logo_w - logo_text_w) / 2.0;
    renderer.draw_text(logo_x, y, "ump", theme::GROUND, ch);
    let logo_right = area.x + logo_w;

    // Metadata items, in left-to-right display order.
    let (ts_num, ts_den) = app.time_signature();
    let mut items = vec![
        HeaderItem {
            kind: HeaderItemKind::Smf,
            text: format!("SMF {}", app.format),
            color: theme::BAR_FG,
        },
        HeaderItem {
            kind: HeaderItemKind::Tpqn,
            text: format!("TPQN {}", app.ticks_per_quarter),
            color: theme::BAR_FG,
        },
        HeaderItem {
            kind: HeaderItemKind::Notes,
            text: format!("NOTES {}", format_thousands(app.total_notes)),
            color: theme::BAR_FG,
        },
        HeaderItem {
            kind: HeaderItemKind::Tracks,
            text: format!("TRACKS {}", app.track_count),
            color: theme::BAR_FG,
        },
    ];
    if app.port_count > 1 {
        items.push(HeaderItem {
            kind: HeaderItemKind::Ports,
            text: format!("PORTS {}", app.port_count),
            color: theme::BAR_FG,
        });
    }
    items.push(HeaderItem {
        kind: HeaderItemKind::TimeSig,
        text: format!("{}/{}", ts_num, 1 << ts_den),
        color: theme::BAR_FG,
    });
    items.push(HeaderItem {
        kind: HeaderItemKind::Bpm,
        text: format!("BPM {:.1}", app.current_bpm()),
        color: theme::BAR_FG,
    });
    items.push(HeaderItem {
        kind: HeaderItemKind::Mode,
        text: app.midi_mode.clone(),
        color: theme::BAR_FG,
    });
    if !app.sf2_name.is_empty() {
        items.push(HeaderItem {
            kind: HeaderItemKind::Sf2,
            text: format!("SF2 {}", truncate_sf2_name(&app.sf2_name)),
            color: theme::ACCENT,
        });
    }

    let kinds: Vec<HeaderItemKind> = items.iter().map(|it| it.kind).collect();
    let widths: Vec<f32> = items.iter().map(|it| text_width(&it.text, cw)).collect();
    let dot = dot_size(scale);
    let sep_width = dot + 2.0 * pad;

    let min_name_w = HEADER_MIN_NAME_CELLS * cw;
    let available = (area.right() - pad - (logo_right + pad + min_name_w)).max(0.0);
    let kept = fit_header_items(&kinds, &widths, sep_width, available);

    let metadata_w = if kept.is_empty() {
        0.0
    } else {
        kept.iter().map(|&i| widths[i]).sum::<f32>() + sep_width * (kept.len() - 1) as f32
    };
    // With no metadata drawn, the filename can extend to the bar's edge
    // (only one `pad` gap, not one on each side of an empty block).
    let metadata_start = if kept.is_empty() {
        area.right()
    } else {
        area.right() - pad - metadata_w
    };

    let mut x = metadata_start;
    for (n, &i) in kept.iter().enumerate() {
        if n > 0 {
            let sep_h = ch * 0.7;
            let sep_y = area.y + (area.height - sep_h) / 2.0;
            renderer.fill_rect(Rect::new(x + pad, sep_y, dot, sep_h), theme::FRAME);
            x += sep_width;
        }
        renderer.draw_text(x, y, &items[i].text, items[i].color, ch);
        x += widths[i];
    }

    // Filename, scrolling if it doesn't fit the remaining space.
    let name_x = logo_right + pad;
    let name_w = (metadata_start - pad - name_x).max(0.0);
    hits.push(Rect::new(name_x, area.y, name_w, area.height), HitAction::OpenMidi);
    draw_header_filename(renderer, app, name_x, y, name_w, cw, ch);
}

/// Draw right-aligned port tabs ("P1 P2 ...") inside a panel's title strip.
fn render_port_tabs(renderer: &mut dyn Renderer, panel: Rect, title_h: f32, app: &App, hits: &mut HitMap) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let pad = px(4.0, scale);
    let dot = dot_size(scale);
    let tab_h = title_h - 2.0 * px(2.0, scale);
    let tab_w = text_width("P9", cw) + 2.0 * pad;

    let tab_y = panel.y + (title_h - tab_h) / 2.0;
    let mut x = panel.right() - pad;
    for p in (0..app.port_count).rev() {
        x -= tab_w;
        let rect = Rect::new(x, tab_y, tab_w, tab_h);
        let is_current = p == app.current_port;
        let label = format!("P{}", p + 1);
        if is_current {
            renderer.fill_rect(rect, theme::GROUND);
            let tx = rect.x + (rect.width - text_width(&label, cw)) / 2.0;
            let ty = rect.y + (rect.height - ch) / 2.0;
            renderer.draw_text(tx, ty, &label, theme::TITLE_BG, ch);
        } else {
            renderer.fill_rect(Rect::new(rect.x, rect.y, rect.width, dot), theme::TITLE_FG);
            renderer.fill_rect(Rect::new(rect.x, rect.bottom() - dot, rect.width, dot), theme::TITLE_FG);
            renderer.fill_rect(Rect::new(rect.x, rect.y, dot, rect.height), theme::TITLE_FG);
            renderer.fill_rect(Rect::new(rect.right() - dot, rect.y, dot, rect.height), theme::TITLE_FG);
            let tx = rect.x + (rect.width - text_width(&label, cw)) / 2.0;
            let ty = rect.y + (rect.height - ch) / 2.0;
            renderer.draw_text(tx, ty, &label, theme::TITLE_FG, ch);
        }
        hits.push(rect, HitAction::PortTab(p));
        x -= pad;
    }
}

/// Draws the header filename, marquee-scrolling it if it doesn't fit `name_w`.
fn draw_header_filename(
    renderer: &mut dyn Renderer,
    app: &App,
    x: f32,
    y: f32,
    name_w: f32,
    cw: f32,
    ch: f32,
) {
    if app.file_name.is_empty() {
        renderer.draw_text(x, y, "NO FILE", theme::DIM, ch);
        return;
    }

    let name_width = text_cells(&app.file_name);
    let visible_cells = (name_w / cw).floor() as usize;

    if name_width <= visible_cells {
        renderer.draw_text(x, y, &app.file_name, theme::BAR_FG, ch);
        return;
    }

    // Marquee scroll: pause → scroll → loop.
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
    let visible = visible_slice(&looping, offset, visible_cells);
    renderer.draw_text(x, y, &visible, theme::BAR_FG, ch);
}

/// Extract visible substring at a given cell offset with max visible width.
/// Characters that don't fully fit within the window are skipped.
fn visible_slice(text: &str, offset_cells: usize, max_cells: usize) -> String {
    let mut result = String::new();
    let mut pos = 0usize;
    let end = offset_cells + max_cells;

    for ch in text.chars() {
        let w = UnicodeWidthChar::width_cjk(ch).unwrap_or(1);
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
    // Only the player screen registers hit regions; keep the map empty here
    // so stale player-screen regions can't be hit-tested while browsing.
    app.hit_map.clear();
    if let Some(ref mut browser) = app.file_browser {
        browser.render(renderer);
    }
}
