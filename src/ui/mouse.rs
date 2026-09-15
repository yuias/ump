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
    /// Separate accumulator for piano roll zoom/key-scroll `PixelDelta` wheel
    /// events, stepped at a different granularity (`px(40)`) than the track
    /// list's row-height-based `wheel_accum`.
    roll_wheel_accum: f32,
}

impl Default for MouseState {
    fn default() -> Self {
        MouseState {
            last_click: None,
            cursor: (0.0, 0.0),
            hovering: false,
            wheel_accum: 0.0,
            roll_wheel_accum: 0.0,
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
            .map(|a| !matches!(a, HitAction::TrackList | HitAction::PianoRoll))
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
            HitAction::Ruler { axis_origin, px_per_tick, view_start_tick, vertical } => {
                let cursor_axis = if vertical { y } else { x };
                let delta_ticks = ((cursor_axis - axis_origin) as f64 / px_per_tick).max(0.0);
                app.seek_to_tick(view_start_tick + delta_ticks as u64);
                true
            }
            HitAction::TrackList | HitAction::PianoRoll => false,
        }
    }

    /// Handle a mouse wheel event: scrolls the track list, or zooms/scrolls
    /// the piano roll's key range, depending on what's under the cursor.
    /// `row_px` is one track-list row's height in physical px (for
    /// `PixelDelta` -> row steps); `roll_step_px` is the equivalent step for
    /// piano-roll zoom/key-scroll. `shift` selects key-scroll over zoom on
    /// the piano roll (see `handle_roll_wheel`).
    pub fn handle_wheel(
        &mut self,
        app: &mut App,
        delta: MouseScrollDelta,
        row_px: f32,
        roll_step_px: f32,
        shift: bool,
    ) -> bool {
        let (x, y) = self.cursor;
        let over_list = app
            .hit_map
            .hit_test_where(x, y, |a| matches!(a, HitAction::TrackList))
            .is_some();
        if over_list {
            return self.handle_tracklist_wheel(app, delta, row_px);
        }

        let over_roll = app
            .hit_map
            .hit_test_where(x, y, |a| matches!(a, HitAction::PianoRoll | HitAction::Ruler { .. }))
            .is_some();
        if over_roll {
            return self.handle_roll_wheel(app, delta, roll_step_px, shift);
        }

        false
    }

    fn handle_tracklist_wheel(&mut self, app: &mut App, delta: MouseScrollDelta, row_px: f32) -> bool {
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

    /// Wheel over the piano roll: without Shift, zooms in/out; with Shift,
    /// scrolls the visible key range (wheel up = toward higher pitches).
    /// Winit delivers Shift+wheel as a horizontal `LineDelta`/`PixelDelta.x`
    /// on some platforms, so `x` is used in place of `y` when Shift is held
    /// and `y` carries no delta.
    fn handle_roll_wheel(&mut self, app: &mut App, delta: MouseScrollDelta, step_px: f32, shift: bool) -> bool {
        match delta {
            MouseScrollDelta::LineDelta(dx, dy) => {
                let primary = if shift && dy == 0.0 { dx } else { dy };
                if shift {
                    let lines = primary.round() as i32;
                    if lines == 0 {
                        return false;
                    }
                    app.scroll_keys(lines * 2);
                    true
                } else if primary > 0.0 {
                    app.zoom_in();
                    true
                } else if primary < 0.0 {
                    app.zoom_out();
                    true
                } else {
                    false
                }
            }
            MouseScrollDelta::PixelDelta(pos) => {
                if step_px <= 0.0 {
                    return false;
                }
                let raw = if shift && pos.y == 0.0 { pos.x } else { pos.y };
                self.roll_wheel_accum += raw as f32;
                let mut moved = false;
                while self.roll_wheel_accum >= step_px {
                    if shift {
                        app.scroll_keys(2);
                    } else {
                        app.zoom_in();
                    }
                    self.roll_wheel_accum -= step_px;
                    moved = true;
                }
                while self.roll_wheel_accum <= -step_px {
                    if shift {
                        app.scroll_keys(-2);
                    } else {
                        app.zoom_out();
                    }
                    self.roll_wheel_accum += step_px;
                    moved = true;
                }
                moved
            }
        }
    }
}
