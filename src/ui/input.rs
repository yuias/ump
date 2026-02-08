//! Keyboard input handling (winit KeyEvent).

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{App, AppScreen};
use crate::ui::file_browser::{BrowseResult, BrowseTarget, FileBrowser};

/// Input result.
pub enum InputResult {
    /// No event occurred.
    None,
    /// An event was handled (UI may have changed).
    Handled,
    /// Application should quit.
    Quit,
}

/// Process a single winit KeyEvent with current modifier state. Returns the result action.
pub fn handle_winit_input(
    app: &mut App,
    event: &KeyEvent,
    modifiers: ModifiersState,
) -> InputResult {
    if event.state != ElementState::Pressed {
        return InputResult::None;
    }

    let quit = match app.screen {
        AppScreen::Player => process_player_key(app, event, modifiers),
        AppScreen::FileBrowser => process_browser_key(app, event, modifiers),
    };

    if quit {
        InputResult::Quit
    } else {
        InputResult::Handled
    }
}

/// Extract effective character from a KeyEvent.
/// Prefers logical_key, falls back to event.text for robustness
/// against winit/Windows keyboard mapping inconsistencies.
fn effective_char(event: &KeyEvent) -> Option<&str> {
    match &event.logical_key {
        Key::Character(c) => Some(c.as_str()),
        _ => event.text.as_ref().map(|t| t.as_str()),
    }
}

/// Process a key in player screen. Returns true if app should quit.
fn process_player_key(app: &mut App, event: &KeyEvent, modifiers: ModifiersState) -> bool {
    let is_ctrl = modifiers.control_key();
    let ch = effective_char(event);

    // Help overlay intercepts most keys
    if app.show_help {
        match ch {
            Some("?") => app.toggle_help(),
            Some("c") if is_ctrl => return true,
            _ => {}
        }
        if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
            app.toggle_help();
        }
        return false;
    }

    // Character-based commands
    if let Some(s) = ch {
        if s == "c" && is_ctrl {
            return true;
        }
        match s {
            "q" | "Q" => return true,
            "s" | "S" => app.stop(),
            "m" | "M" => app.toggle_mute_selected(),
            "p" | "P" => app.toggle_piano_roll(),
            "v" | "V" => app.toggle_piano_roll_orientation(),
            "e" | "E" => app.toggle_track_view_mode(),
            "+" | "=" => app.volume_up(),
            "-" => app.volume_down(),
            "[" => app.zoom_out(),
            "]" => app.zoom_in(),
            "1" => app.set_midi_mode("GM"),
            "2" => app.set_midi_mode("GS"),
            "3" => app.set_midi_mode("XG"),
            "4" => app.set_midi_mode("GM2"),
            "o" | "O" => open_browser(app, BrowseTarget::Midi),
            "f" | "F" => open_browser(app, BrowseTarget::Sf2),
            "d" | "D" => {
                let _ = app.reset_sf2_to_default();
            }
            "?" => app.toggle_help(),
            _ => {}
        }
    }

    // Named key commands
    if let Key::Named(named) = &event.logical_key {
        match named {
            NamedKey::Escape => return true,
            NamedKey::Space => app.toggle_play(),
            NamedKey::ArrowLeft => app.seek_backward(),
            NamedKey::ArrowRight => app.seek_forward(),
            NamedKey::ArrowUp => app.move_cursor_up(),
            NamedKey::ArrowDown => app.move_cursor_down(),
            NamedKey::Tab => app.cycle_focus(),
            _ => {}
        }
    }

    false
}

/// Process a key in file browser screen. Returns true if app should quit.
fn process_browser_key(app: &mut App, event: &KeyEvent, modifiers: ModifiersState) -> bool {
    let is_ctrl = modifiers.control_key();
    let ch = effective_char(event);

    if let Some(s) = ch {
        if s == "c" && is_ctrl {
            return true;
        }
        match s {
            "/" | "\\" => {
                if let Some(ref mut b) = app.file_browser {
                    b.go_drives();
                }
            }
            _ => {}
        }
    }

    if let Key::Named(named) = &event.logical_key {
        match named {
            NamedKey::Escape => {
                if app.has_midi() && app.has_sf2() {
                    app.screen = AppScreen::Player;
                    app.file_browser = None;
                } else {
                    return true;
                }
            }
            NamedKey::ArrowUp => {
                if let Some(ref mut b) = app.file_browser {
                    b.cursor_up();
                }
            }
            NamedKey::ArrowDown => {
                if let Some(ref mut b) = app.file_browser {
                    b.cursor_down();
                }
            }
            NamedKey::Backspace => {
                if let Some(ref mut b) = app.file_browser {
                    b.go_parent();
                }
            }
            NamedKey::Home => {
                if let Some(ref mut b) = app.file_browser {
                    b.go_home();
                }
            }
            NamedKey::Enter => {
                handle_browser_enter(app);
            }
            _ => {}
        }
    }
    false
}

fn handle_browser_enter(app: &mut App) {
    let result = app
        .file_browser
        .as_mut()
        .map(|b| b.enter())
        .unwrap_or(BrowseResult::Continue);

    match result {
        BrowseResult::Selected(path) => {
            let target = app
                .file_browser
                .as_ref()
                .map(|b| b.target)
                .unwrap_or(BrowseTarget::Midi);

            // Remember parent directory for next browser open
            if let Some(parent) = path.parent() {
                app.last_browser_dir = Some(parent.to_path_buf());
            }

            let path_str = path.to_string_lossy().to_string();

            match target {
                BrowseTarget::Sf2 => {
                    if let Err(e) = app.reload_sf2(&path_str) {
                        log_warn!("Failed to load SF2: {}", e);
                    } else if !app.has_midi() {
                        open_browser(app, BrowseTarget::Midi);
                    } else {
                        app.file_browser = None;
                        app.screen = AppScreen::Player;
                        let _ = app.ensure_audio();
                        app.start_playback();
                    }
                }
                BrowseTarget::Midi => {
                    if let Err(e) = app.reload_midi(&path_str) {
                        log_warn!("Failed to load MIDI: {}", e);
                    } else if !app.has_sf2() {
                        open_browser(app, BrowseTarget::Sf2);
                    } else {
                        app.file_browser = None;
                        app.screen = AppScreen::Player;
                        let _ = app.ensure_audio();
                        app.start_playback();
                    }
                }
            }
        }
        BrowseResult::Cancel => {
            if app.has_midi() && app.has_sf2() {
                app.screen = AppScreen::Player;
                app.file_browser = None;
            }
        }
        BrowseResult::Continue => {}
    }
}

fn open_browser(app: &mut App, target: BrowseTarget) {
    let browser = FileBrowser::new(target, app.last_browser_dir.as_deref());
    app.file_browser = Some(browser);
    app.screen = AppScreen::FileBrowser;
}
