//! File browser widget for selecting MIDI and SF2 files (native pixel rendering).
//! Renders as a modal dialog (see `render.rs::render_browser`), DOS `dir`-style.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use unicode_width::UnicodeWidthChar;

use crate::debug::days_to_ymd;
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::border::draw_panel;
use crate::ui::fkey_bar::draw_fkey_items;
use crate::ui::header::format_thousands;
use crate::ui::hit::{BrowserButton, HitAction, HitMap};
use crate::ui::layout;
use crate::ui::text::{text_cells, text_width, truncate_cells};
use crate::ui::theme;

/// Width of the right-aligned size/tag column, in cells. 13 fits sizes up to
/// 9,999,999,999 bytes; SF2 banks routinely exceed the 11 cells of 100 MB.
const SIZE_COL_CELLS: usize = 13;
/// Width of the date column (`YYYY-MM-DD HH:MM`), in cells.
const DATE_COL_CELLS: usize = 16;
/// Gap between adjacent list columns, in cells.
const COL_GAP_CELLS: usize = 2;
/// Name column width below which the date, then the size column, are dropped.
const MIN_NAME_CELLS: usize = 16;
/// Scrollbar thumb minimum used when paging from a click outside a render
/// pass (mouse handling has no renderer/scale access). Close enough to
/// `render`'s `px(12.0, scale)` that it doesn't change which side of the
/// thumb a click resolves to, except within a couple of px of the boundary.
const SCROLL_MIN_THUMB_PX: f32 = 12.0;

/// What kind of file the browser is selecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseTarget {
    Midi,
    Sf2,
}

/// Result of a browser interaction step.
#[allow(dead_code)]
pub enum BrowseResult {
    Continue,
    Selected(PathBuf),
    Cancel,
}

/// Directory entry for display.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

pub struct FileBrowser {
    current_dir: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    scroll_offset: usize,
    /// Number of rows visible in the last render, used by mouse wheel/page
    /// handling to move the cursor by a page.
    visible_rows: usize,
    pub target: BrowseTarget,
    show_drives: bool,
}

impl FileBrowser {
    pub fn new(target: BrowseTarget, start_dir: Option<&Path>) -> Self {
        let current_dir = start_dir
            .map(|p| p.to_path_buf())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut browser = FileBrowser {
            current_dir,
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            visible_rows: 0,
            target,
            show_drives: false,
        };
        browser.refresh_entries();
        browser
    }

    fn refresh_entries(&mut self) {
        self.entries.clear();
        self.show_drives = false;

        if let Some(parent) = self.current_dir.parent() {
            if parent != self.current_dir {
                self.entries.push(Entry {
                    name: "..".to_string(),
                    path: parent.to_path_buf(),
                    is_dir: true,
                    size: 0,
                    modified: None,
                });
            }
        }

        let Ok(read_dir) = fs::read_dir(&self.current_dir) else {
            self.reset_cursor();
            return;
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in read_dir.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let modified = meta.modified().ok();

            if meta.is_dir() {
                dirs.push(Entry {
                    name,
                    path: entry.path(),
                    is_dir: true,
                    size: 0,
                    modified,
                });
            } else if self.matches_filter(&entry.path()) {
                files.push(Entry {
                    name,
                    path: entry.path(),
                    is_dir: false,
                    size: meta.len(),
                    modified,
                });
            }
        }

        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        self.entries.extend(dirs);
        self.entries.extend(files);
        self.reset_cursor();
    }

    fn refresh_drives(&mut self) {
        self.entries.clear();
        self.show_drives = true;

        for root in list_roots() {
            let name = root.to_string_lossy().to_string();
            self.entries.push(Entry {
                name,
                path: root,
                is_dir: true,
                size: 0,
                modified: None,
            });
        }
        self.reset_cursor();
    }

    fn reset_cursor(&mut self) {
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    fn matches_filter(&self, path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(OsStr::to_str)
            .map(|s| s.to_lowercase());
        match self.target {
            BrowseTarget::Midi => matches!(ext.as_deref(), Some("mid" | "midi")),
            BrowseTarget::Sf2 => matches!(ext.as_deref(), Some("sf2")),
        }
    }

    pub fn cursor_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.cursor == 0 {
            self.cursor = self.entries.len() - 1;
        } else {
            self.cursor -= 1;
        }
        self.adjust_scroll();
    }

    pub fn cursor_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.cursor + 1 >= self.entries.len() {
            self.cursor = 0;
        } else {
            self.cursor += 1;
        }
        self.adjust_scroll();
    }

    fn adjust_scroll(&mut self) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    /// Move the cursor to row `i` (from a mouse click), clamped to the
    /// entry range. Only clamps scrolling upward; `render` clamps the other
    /// direction against the current visible row count.
    pub fn set_cursor(&mut self, i: usize) {
        if self.entries.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = i.min(self.entries.len() - 1);
        self.adjust_scroll();
    }

    /// Move the cursor up by one visible page (no wraparound, unlike
    /// `cursor_up`).
    pub fn page_up(&mut self) {
        let target = page_target(self.cursor, self.visible_rows.max(1), self.entries.len(), true);
        self.set_cursor(target);
    }

    /// Move the cursor down by one visible page (no wraparound, unlike
    /// `cursor_down`).
    pub fn page_down(&mut self) {
        let target = page_target(self.cursor, self.visible_rows.max(1), self.entries.len(), false);
        self.set_cursor(target);
    }

    /// Handle a press on the scrollbar track at `local_y` (relative to the
    /// track's top), `track_h` px tall: pages toward the thumb.
    pub fn scroll_click(&mut self, local_y: f32, track_h: f32) {
        if self.entries.is_empty() || self.visible_rows == 0 {
            return;
        }
        let (thumb_y, thumb_h) = thumb_geometry(
            self.entries.len(),
            self.visible_rows,
            self.scroll_offset,
            track_h,
            SCROLL_MIN_THUMB_PX,
        );
        if local_y < thumb_y {
            self.page_up();
        } else if local_y > thumb_y + thumb_h {
            self.page_down();
        }
    }

    pub fn enter(&mut self) -> BrowseResult {
        let Some(entry) = self.entries.get(self.cursor).cloned() else {
            return BrowseResult::Continue;
        };

        if entry.is_dir {
            self.current_dir = entry.path;
            self.refresh_entries();
            BrowseResult::Continue
        } else {
            BrowseResult::Selected(entry.path)
        }
    }

    pub fn go_parent(&mut self) {
        if self.show_drives {
            return;
        }
        match self.current_dir.parent() {
            Some(parent) if parent != self.current_dir => {
                self.current_dir = parent.to_path_buf();
                self.refresh_entries();
            }
            _ => {
                self.refresh_drives();
            }
        }
    }

    pub fn go_home(&mut self) {
        if let Some(home) = dirs::home_dir() {
            self.current_dir = home;
            self.refresh_entries();
        }
    }

    pub fn go_drives(&mut self) {
        self.refresh_drives();
    }

    /// Render the file browser as a modal dialog. `can_cancel` gates the
    /// CANCEL button (mirrors `input.rs`: Escape quits the app outright when
    /// no MIDI/SF2 is loaded, so a clickable Cancel would be misleading
    /// there). `hover` is `app.hover`, used to highlight the hovered button.
    pub fn render(
        &mut self,
        renderer: &mut dyn Renderer,
        can_cancel: bool,
        hover: Option<&HitAction>,
        hits: &mut HitMap,
    ) {
        let (w, h) = renderer.window_size();
        let (cw, ch) = renderer.cell_size();
        let scale = renderer.scale_factor();
        let w_f = w as f32;
        let h_f = h as f32;

        let target_w = (w_f * 0.70).min(100.0 * cw);
        let min_w = (w_f - 2.0 * layout::px(6.0, scale)).min(60.0 * cw);
        let popup_w = target_w.max(min_w).max(0.0);
        let popup_h = h_f * 0.80;
        let popup_x = (w_f - popup_w) / 2.0;
        let popup_y = (h_f - popup_h) / 2.0;
        let area = Rect::new(popup_x, popup_y, popup_w, popup_h);

        let title = match self.target {
            BrowseTarget::Midi => "OPEN MIDI FILE",
            BrowseTarget::Sf2 => "OPEN SOUNDFONT",
        };
        let title_h = layout::title_height(ch, scale);
        draw_panel(renderer, area, title, title_h);

        let dot = layout::dot_size(scale);
        let inner = layout::panel_content(area, title_h, dot);
        if inner.width < cw * 10.0 || inner.height < ch * 5.0 {
            return;
        }

        let pad = layout::px(6.0, scale);

        // Path bar: full width, flush with the panel content, header-bar style.
        let path_h = ch + 2.0 * layout::px(2.0, scale);
        let path_rect = Rect::new(inner.x, inner.y, inner.width, path_h);
        renderer.fill_rect(path_rect, theme::BAR_BG);
        let path_text = if self.show_drives {
            "DRIVES".to_string()
        } else {
            let full = self.current_dir.to_string_lossy().to_string();
            let avail_cells = ((path_rect.width - 2.0 * pad) / cw).floor().max(0.0) as usize;
            path_tail(&full, avail_cells)
        };
        renderer.draw_text(
            path_rect.x + pad,
            path_rect.y + layout::px(2.0, scale),
            &path_text,
            theme::BAR_FG,
            ch,
        );

        // Button row: full width, flush with the bottom, fkey-bar styled.
        let button_h = ch + 2.0 * layout::px(3.0, scale);
        let button_row = Rect::new(inner.x, inner.bottom() - button_h, inner.width, button_h);

        // List area: between the path bar and the button row, inset by `pad`.
        let list_top = path_rect.bottom() + pad;
        let list_bottom = button_row.y - pad;
        let list_area = Rect::new(
            inner.x + pad,
            list_top,
            (inner.width - 2.0 * pad).max(0.0),
            (list_bottom - list_top).max(0.0),
        );

        let row_h = ch * layout::ROW_HEIGHT;
        let visible_rows = if row_h > 0.0 { (list_area.height / row_h) as usize } else { 0 };

        if visible_rows > 0 {
            if self.cursor >= self.scroll_offset + visible_rows {
                self.scroll_offset = self.cursor - visible_rows + 1;
            }
            if self.cursor < self.scroll_offset {
                self.scroll_offset = self.cursor;
            }
        }
        self.visible_rows = visible_rows;

        hits.push(list_area, HitAction::BrowserList);

        let show_scrollbar = visible_rows > 0 && self.entries.len() > visible_rows;
        let scrollbar_w = layout::px(8.0, scale).max(2.0 * dot);
        let content_w = if show_scrollbar { (list_area.width - scrollbar_w).max(0.0) } else { list_area.width };

        if self.entries.is_empty() {
            let msg = "NO FILES";
            let msg_w = text_width(msg, cw);
            let mx = list_area.x + (content_w - msg_w) / 2.0;
            let my = list_area.y + (list_area.height - ch) / 2.0;
            renderer.draw_text(mx, my, msg, theme::DIM, ch);
        } else {
            let avail_cells = (content_w / cw).floor().max(0.0) as usize;
            let (show_size, show_date) =
                fit_columns(avail_cells, SIZE_COL_CELLS, DATE_COL_CELLS, COL_GAP_CELLS);

            let date_x = list_area.x + content_w - DATE_COL_CELLS as f32 * cw;
            let size_right = if show_date { date_x - COL_GAP_CELLS as f32 * cw } else { list_area.x + content_w };
            let size_x = size_right - SIZE_COL_CELLS as f32 * cw;
            let name_right = if show_size { size_x - COL_GAP_CELLS as f32 * cw } else { size_right };
            let name_cells = ((name_right - list_area.x) / cw).floor().max(0.0) as usize;

            for (i, entry) in self.entries.iter().enumerate().skip(self.scroll_offset).take(visible_rows) {
                let row_i = i - self.scroll_offset;
                let row_y = list_area.y + row_i as f32 * row_h;
                let row_rect = Rect::new(list_area.x, row_y, content_w, row_h);
                let text_y = row_y + (row_h - ch) / 2.0;
                let is_selected = i == self.cursor;

                if is_selected {
                    renderer.fill_rect(row_rect, theme::SELECTED_BG);
                }
                hits.push(row_rect, HitAction::BrowserRow(i));

                let (name_color, tag) = if self.show_drives {
                    (theme::ACCENT, Some("<DRV>"))
                } else if entry.name == ".." || entry.is_dir {
                    (theme::FRAME, Some("<DIR>"))
                } else {
                    (theme::TEXT, None)
                };
                let no_date = self.show_drives || entry.name == "..";

                let name = truncate_cells(&entry.name, name_cells);
                renderer.draw_text(list_area.x, text_y, &name, name_color, ch);

                if show_size {
                    let size_text = match tag {
                        Some(t) => format!("{:>w$}", t, w = SIZE_COL_CELLS),
                        None => format!("{:>w$}", format_thousands(entry.size as usize), w = SIZE_COL_CELLS),
                    };
                    renderer.draw_text(size_x, text_y, &size_text, theme::DIM, ch);
                }
                if show_date
                    && !no_date
                    && let Some(m) = entry.modified
                {
                    let date_text = format_timestamp(m);
                    renderer.draw_text(date_x, text_y, &date_text, theme::DIM, ch);
                }
            }

            if show_scrollbar {
                let track = Rect::new(list_area.x + content_w, list_area.y, scrollbar_w, list_area.height);
                renderer.fill_dither(track, theme::DIM, Some(theme::GROUND));
                let min_thumb = layout::px(12.0, scale);
                let (thumb_y, thumb_h) =
                    thumb_geometry(self.entries.len(), visible_rows, self.scroll_offset, list_area.height, min_thumb);
                renderer.fill_rect(Rect::new(track.x, track.y + thumb_y, scrollbar_w, thumb_h), theme::FRAME);
                hits.push(track, HitAction::BrowserScroll { track_y: track.y, track_h: track.height });
            }
        }

        let mut buttons: Vec<(&str, &str, HitAction)> = vec![
            ("ENT", "OPEN", HitAction::BrowserButton(BrowserButton::Open)),
            ("BS", "UP", HitAction::BrowserButton(BrowserButton::Up)),
            ("HOME", "HOME", HitAction::BrowserButton(BrowserButton::Home)),
            ("/", "DRIVES", HitAction::BrowserButton(BrowserButton::Drives)),
        ];
        if can_cancel {
            buttons.push(("ESC", "CANCEL", HitAction::BrowserButton(BrowserButton::Cancel)));
        }
        draw_fkey_items(renderer, button_row, &buttons, hover.copied(), hits);
    }
}

/// Target cursor after paging by `visible` rows: clamped to `[0, total-1]`
/// with no wraparound (unlike `cursor_up`/`cursor_down`).
fn page_target(cursor: usize, visible: usize, total: usize, up: bool) -> usize {
    if total == 0 {
        return 0;
    }
    if up {
        cursor.saturating_sub(visible)
    } else {
        (cursor + visible).min(total - 1)
    }
}

/// Scrollbar thumb geometry within a `track_h`-tall track: `total` entries,
/// `visible` shown, scrolled to `offset`. Thumb height is proportional to
/// `visible/total`, floored at `min_thumb`. Returns `(thumb_y, thumb_h)`,
/// relative to the track's top.
fn thumb_geometry(total: usize, visible: usize, offset: usize, track_h: f32, min_thumb: f32) -> (f32, f32) {
    if total == 0 || track_h <= 0.0 {
        return (0.0, track_h.max(0.0));
    }
    let raw_h = track_h * (visible.min(total) as f32 / total as f32);
    let thumb_h = raw_h.max(min_thumb).min(track_h);
    let max_offset = total.saturating_sub(visible).max(1);
    let thumb_y = (track_h - thumb_h) * (offset as f32 / max_offset as f32);
    (thumb_y, thumb_h)
}

/// Which optional list columns fit `avail_cells`: drops the date column,
/// then the size column, once the remaining name column would go under
/// `MIN_NAME_CELLS`. Returns `(show_size, show_date)`.
fn fit_columns(avail_cells: usize, size_cells: usize, date_cells: usize, gap_cells: usize) -> (bool, bool) {
    let both = size_cells + gap_cells + date_cells + gap_cells;
    let size_only = size_cells + gap_cells;
    if avail_cells >= MIN_NAME_CELLS + both {
        (true, true)
    } else if avail_cells >= MIN_NAME_CELLS + size_only {
        (true, false)
    } else {
        (false, false)
    }
}

/// Tail-truncates `path` to at most `max_cells` display cells, prefixing
/// `...` when it doesn't fit; never splits a wide character.
fn path_tail(path: &str, max_cells: usize) -> String {
    if text_cells(path) <= max_cells {
        return path.to_string();
    }
    const ELLIPSIS: &str = "...";
    let ellipsis_cells = text_cells(ELLIPSIS);
    if max_cells <= ellipsis_cells {
        return truncate_cells(ELLIPSIS, max_cells);
    }
    let budget = max_cells - ellipsis_cells;

    let mut tail_chars: Vec<char> = Vec::new();
    let mut used = 0usize;
    for c in path.chars().rev() {
        let w = UnicodeWidthChar::width_cjk(c).unwrap_or(1);
        if used + w > budget {
            break;
        }
        tail_chars.push(c);
        used += w;
    }
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    format!("{}{}", ELLIPSIS, tail)
}

/// Formats a `SystemTime` as `YYYY-MM-DD HH:MM` in UTC (no local-time crate
/// dependency; mirrors `debug::chrono_now`).
fn format_timestamp(time: SystemTime) -> String {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (y, mo, d) = days_to_ymd(days);
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, mo, d, h, mi)
}

/// List available filesystem roots.
fn list_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mask = unsafe { windows_drives_mask() };
        let mut roots = Vec::new();
        for i in 0..26u8 {
            if mask & (1 << i) != 0 {
                let letter = (b'A' + i) as char;
                roots.push(PathBuf::from(format!("{}:\\", letter)));
            }
        }
        roots
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

#[cfg(windows)]
unsafe fn windows_drives_mask() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalDrives() -> u32;
    }
    unsafe { GetLogicalDrives() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn path_tail_keeps_short_path_intact() {
        assert_eq!(path_tail("C:\\short", 20), "C:\\short");
    }

    #[test]
    fn path_tail_prefixes_ellipsis_and_keeps_tail() {
        let result = path_tail("C:\\a\\very\\long\\path\\here", 12);
        assert!(result.starts_with("..."));
        assert!(text_cells(&result) <= 12);
        assert!(result.ends_with("here"));
    }

    #[test]
    fn path_tail_never_splits_a_wide_char() {
        let path = format!("C:\\{}", "\u{65E5}".repeat(10)); // 10 CJK chars = 20 cells
        let result = path_tail(&path, 10);
        assert!(text_cells(&result) <= 10);
        let body = result.trim_start_matches("...");
        assert_eq!(text_cells(body) % 2, 0);
    }

    #[test]
    fn format_timestamp_epoch() {
        assert_eq!(format_timestamp(SystemTime::UNIX_EPOCH), "1970-01-01 00:00");
    }

    #[test]
    fn format_timestamp_leap_day() {
        // 2020-02-29 00:00 UTC (2020 is a leap year); cross-checked against
        // `days_to_ymd` directly rather than a recalled epoch value.
        let (y, mo, d) = days_to_ymd(18321);
        assert_eq!((y, mo, d), (2020, 2, 29));
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(18321 * 86400);
        assert_eq!(format_timestamp(t), "2020-02-29 00:00");
    }

    #[test]
    fn format_timestamp_end_of_year() {
        // 2023-12-31 23:59 UTC.
        let (y, mo, d) = days_to_ymd(19722);
        assert_eq!((y, mo, d), (2023, 12, 31));
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(19722 * 86400 + 23 * 3600 + 59 * 60);
        assert_eq!(format_timestamp(t), "2023-12-31 23:59");
    }

    #[test]
    fn fit_columns_shows_everything_when_wide() {
        assert_eq!(fit_columns(60, 10, 16, 2), (true, true));
    }

    #[test]
    fn fit_columns_drops_date_first() {
        // 16 (min name) + 10 (size) + 2 (gap) = 28 fits without date;
        // adding the date's 16+2 pushes it over.
        assert_eq!(fit_columns(28, 10, 16, 2), (true, false));
        assert_eq!(fit_columns(46, 10, 16, 2), (true, true));
    }

    #[test]
    fn fit_columns_drops_size_when_too_narrow() {
        assert_eq!(fit_columns(20, 10, 16, 2), (false, false));
    }

    #[test]
    fn thumb_geometry_full_height_when_all_visible() {
        let (y, h) = thumb_geometry(10, 10, 0, 100.0, 12.0);
        assert_eq!(y, 0.0);
        assert_eq!(h, 100.0);
    }

    #[test]
    fn thumb_geometry_proportional_and_positioned() {
        let (y, h) = thumb_geometry(100, 10, 0, 100.0, 5.0);
        assert!((h - 10.0).abs() < 0.01);
        assert!((y - 0.0).abs() < 0.01);

        let (y, h) = thumb_geometry(100, 10, 90, 100.0, 5.0);
        assert!((h - 10.0).abs() < 0.01);
        assert!((y - 90.0).abs() < 0.01); // max_offset = 90, offset=90 -> bottom
    }

    #[test]
    fn thumb_geometry_floors_at_min_thumb() {
        let (_, h) = thumb_geometry(1000, 5, 0, 100.0, 12.0);
        assert_eq!(h, 12.0);
    }

    #[test]
    fn page_target_moves_up_without_wrap() {
        assert_eq!(page_target(3, 5, 20, true), 0);
        assert_eq!(page_target(0, 5, 20, true), 0);
    }

    #[test]
    fn page_target_moves_down_clamped_to_last() {
        assert_eq!(page_target(3, 5, 20, false), 8);
        assert_eq!(page_target(18, 5, 20, false), 19);
    }
}
