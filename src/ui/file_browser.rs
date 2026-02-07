//! File browser widget for selecting MIDI and SF2 files (native pixel rendering).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::renderer::types::{BG_COLOR, Color, Rect};
use crate::renderer::Renderer;
use crate::ui::border::{draw_border, inner_rect};

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
}

pub struct FileBrowser {
    current_dir: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    scroll_offset: usize,
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

            if meta.is_dir() {
                dirs.push(Entry {
                    name,
                    path: entry.path(),
                    is_dir: true,
                });
            } else if self.matches_filter(&entry.path()) {
                files.push(Entry {
                    name,
                    path: entry.path(),
                    is_dir: false,
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

    /// Render the file browser via native pixel rendering.
    pub fn render(&mut self, renderer: &mut dyn Renderer) {
        let (w, h) = renderer.window_size();
        let (cw, ch) = renderer.cell_size();
        let w_f = w as f32;
        let h_f = h as f32;

        // Centered popup: 70% width, 80% height
        let popup_w = w_f * 0.70;
        let popup_h = h_f * 0.80;
        let popup_x = (w_f - popup_w) / 2.0;
        let popup_y = (h_f - popup_h) / 2.0;

        let area = Rect::new(popup_x, popup_y, popup_w, popup_h);

        // Clear region
        renderer.fill_rect(area, BG_COLOR);

        let title = match self.target {
            BrowseTarget::Midi => " Select MIDI File ",
            BrowseTarget::Sf2 => " Select SoundFont ",
        };
        let border_color = Color::rgb(100, 180, 255);
        draw_border(renderer, area, title, border_color);

        let inner = inner_rect(area, cw, ch);
        if inner.width < cw * 10.0 || inner.height < ch * 3.0 {
            return;
        }

        // Path bar
        let header_text = if self.show_drives {
            "Select Drive".to_string()
        } else {
            self.current_dir.to_string_lossy().to_string()
        };
        let path_fg = Color::rgb(180, 180, 200);
        renderer.draw_text(inner.x + cw, inner.y + ch, &header_text, path_fg, ch);

        // File list
        let list_start_y = inner.y + ch * 3.0;
        let list_height = ((inner.height - ch * 5.0) / ch) as usize;

        // Adjust scroll_offset for visible_rows
        if self.cursor >= self.scroll_offset + list_height {
            self.scroll_offset = self.cursor - list_height + 1;
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }

        for (i, entry) in self.entries.iter().enumerate().skip(self.scroll_offset).take(list_height) {
            let screen_y = list_start_y + (i - self.scroll_offset) as f32 * ch;
            let is_selected = i == self.cursor;

            // Selection background
            if is_selected {
                let highlight_bg = Color::rgb(50, 50, 80);
                renderer.fill_rect(
                    Rect::new(inner.x, screen_y, inner.width, ch),
                    highlight_bg,
                );
            }

            let (icon, color) = if self.show_drives {
                ("\u{1F4BD}  ", Color::rgb(255, 200, 100))
            } else if entry.is_dir {
                ("\u{1F4C1}  ", Color::rgb(100, 200, 255))
            } else {
                ("\u{1F3B5}  ", Color::rgb(200, 200, 220))
            };

            let prefix = if is_selected { "> " } else { "  " };
            let text = format!("{}{}{}", prefix, icon, entry.name);
            renderer.draw_text(inner.x, screen_y, &text, color, ch);
        }

        // Help line
        let help_y = inner.y + inner.height - ch * 2.0;
        let help_text = if self.show_drives {
            "\u{2191}\u{2193}:Move  Enter:Select  Esc:Cancel"
        } else {
            "\u{2191}\u{2193}:Move  Enter:Select  BS:Parent  Home:Home  /:Drives  Esc:Cancel"
        };
        let help_fg = Color::rgb(120, 120, 140);
        renderer.draw_text(inner.x + cw, help_y, help_text, help_fg, ch);
    }
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
    extern "system" {
        fn GetLogicalDrives() -> u32;
    }
    unsafe { GetLogicalDrives() }
}
