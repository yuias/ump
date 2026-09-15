//! Mouse input for the player screen: hit-testing, click/double-click,
//! wheel scrolling, and cursor-icon hover feedback.
//!
//! Kept separate from `input.rs` (keyboard) because it needs state that
//! outlives a single event (double-click timing, wheel accumulation).

use std::time::{Duration, Instant};

use winit::event::MouseScrollDelta;
use winit::window::{CursorIcon, Window};

use crate::app::App;
use crate::ui::file_browser::BrowseTarget;
use crate::ui::hit::HitAction;
use crate::ui::input::open_browser;

/// Max gap between two presses on the same row to count as a double-click.
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
/// Max cursor movement between the two presses of a double-click, in px.
const DOUBLE_CLICK_DIST: f32 = 4.0;

/// Mouse state that persists across events: last click (for double-click
/// detection), current cursor position, hover icon, and wheel accumulation.
pub struct MouseState {
    last_click: Option<(Instant, usize, (f32, f32))>,
    cursor: (f32, f32),
    hovering: bool,
    wheel_accum: f32,
}

impl Default for MouseState {
    fn default() -> Self {
        MouseState {
            last_click: None,
            cursor: (0.0, 0.0),
            hovering: false,
            wheel_accum: 0.0,
        }
    }
}

impl MouseState {
    pub fn set_cursor_pos(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
    }

    /// Update the window cursor icon based on what's under the pointer.
    /// Only touches the window when the icon actually changes, so plain
    /// mouse movement doesn't force a redraw.
    pub fn update_hover(&mut self, app: &App, window: &Window) {
        let (x, y) = self.cursor;
        let hovering = app
            .hit_map
            .hit_test(x, y)
            .map(|a| !matches!(a, HitAction::TrackList))
            .unwrap_or(false);
        if hovering != self.hovering {
            self.hovering = hovering;
            window.set_cursor(if hovering { CursorIcon::Pointer } else { CursorIcon::Default });
        }
    }

    /// Handle a left-button press at the current cursor position.
    /// Returns `true` if app state changed (redraw needed).
    pub fn handle_left_press(&mut self, app: &mut App) -> bool {
        let (x, y) = self.cursor;
        let action = match app.hit_map.hit_test(x, y) {
            Some(a) => *a,
            None => return false,
        };

        match action {
            HitAction::TrackRow(i) => {
                let now = Instant::now();
                let is_double = matches!(
                    self.last_click,
                    Some((t, row, (lx, ly)))
                        if row == i
                            && now.duration_since(t) <= DOUBLE_CLICK_INTERVAL
                            && (lx - x).abs() <= DOUBLE_CLICK_DIST
                            && (ly - y).abs() <= DOUBLE_CLICK_DIST
                );

                app.select_track_row(i);
                if is_double {
                    app.toggle_solo_row(i);
                    // Clear so a third press starts a fresh click, not another solo toggle.
                    self.last_click = None;
                } else {
                    self.last_click = Some((now, i, (x, y)));
                }
                true
            }
            HitAction::TrackMute(i) => {
                app.toggle_mute_row(i);
                true
            }
            HitAction::PortTab(p) => {
                app.set_port(p);
                true
            }
            HitAction::OpenMidi => {
                open_browser(app, BrowseTarget::Midi);
                true
            }
            HitAction::TrackList => false,
        }
    }

    /// Handle a mouse wheel event. Only scrolls when the cursor is over a
    /// `TrackList` region, even if a more specific row/mute region sits on
    /// top of it for plain click hit-testing. `row_px` is one row's height
    /// in physical px, used to convert `PixelDelta` into row steps.
    pub fn handle_wheel(&mut self, app: &mut App, delta: MouseScrollDelta, row_px: f32) -> bool {
        let (x, y) = self.cursor;
        let over_list = app
            .hit_map
            .hit_test_where(x, y, |a| matches!(a, HitAction::TrackList))
            .is_some();
        if !over_list {
            return false;
        }

        match delta {
            // Positive y = wheel up = cursor up.
            MouseScrollDelta::LineDelta(_, dy) => {
                let n = dy.round() as i32;
                for _ in 0..n {
                    app.move_cursor_up();
                }
                for _ in 0..(-n) {
                    app.move_cursor_down();
                }
                n != 0
            }
            MouseScrollDelta::PixelDelta(pos) => {
                if row_px <= 0.0 {
                    return false;
                }
                self.wheel_accum += pos.y as f32;
                let mut moved = false;
                while self.wheel_accum >= row_px {
                    app.move_cursor_up();
                    self.wheel_accum -= row_px;
                    moved = true;
                }
                while self.wheel_accum <= -row_px {
                    app.move_cursor_down();
                    self.wheel_accum += row_px;
                    moved = true;
                }
                moved
            }
        }
    }
}
