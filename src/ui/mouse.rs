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
use crate::ui::fkey_bar::{next_midi_mode, FKeyAction};
use crate::ui::hit::HitAction;
use crate::ui::input::open_browser;

/// Max gap between two presses on the same row to count as a double-click.
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
/// Max cursor movement between the two presses of a double-click, in px.
const DOUBLE_CLICK_DIST: f32 = 4.0;

/// An in-progress seek-bar drag: the bar's pixel span, captured at press time
/// so the drag keeps working even once the cursor leaves the bar vertically.
struct SeekDrag {
    x0: f32,
    width: f32,
    total_ticks: u64,
    last_tick: u64,
}

/// Mouse state that persists across events: last click (for double-click
/// detection), current cursor position, hover icon, wheel accumulation, and
/// an in-progress seek-bar drag.
pub struct MouseState {
    last_click: Option<(Instant, usize, (f32, f32))>,
    cursor: (f32, f32),
    hovering: bool,
    wheel_accum: f32,
    /// Separate accumulator for piano roll zoom/key-scroll `PixelDelta` wheel
    /// events, stepped at a different granularity (`px(40)`) than the track
    /// list's row-height-based `wheel_accum`.
    roll_wheel_accum: f32,
    /// Separate accumulator for volume `PixelDelta` wheel events, stepped at
    /// the same `px(40)` granularity as `roll_wheel_accum`.
    vol_wheel_accum: f32,
    drag: Option<SeekDrag>,
}

impl Default for MouseState {
    fn default() -> Self {
        MouseState {
            last_click: None,
            cursor: (0.0, 0.0),
            hovering: false,
            wheel_accum: 0.0,
            roll_wheel_accum: 0.0,
            vol_wheel_accum: 0.0,
            drag: None,
        }
    }
}

/// Converts a cursor x position to a tick along a seek bar spanning
/// `[x0, x0 + width)`, clamped to `[0, total_ticks]`.
fn seek_bar_tick(x: f32, x0: f32, width: f32, total_ticks: u64) -> u64 {
    if width <= 0.0 {
        return 0;
    }
    let frac = ((x - x0) / width).clamp(0.0, 1.0);
    (frac as f64 * total_ticks as f64).round() as u64
}

/// Index (0-9) of the volume block at `x`, given the first block's left edge
/// and the per-block stride (`block_w + gap`).
fn volume_block_index(x: f32, x0: f32, block_w: f32, gap: f32) -> usize {
    let stride = block_w + gap;
    if stride <= 0.0 {
        return 0;
    }
    (((x - x0) / stride).floor() as i32).clamp(0, 9) as usize
}

/// Convert a cursor position on a ruler's time axis to a tick, given the
/// linear mapping recorded when the ruler was rendered, clamped to the
/// song's length. `px_per_tick` may be negative (a vertical piano roll in
/// falling/`Down` flow has tick decrease as the axis coordinate increases).
fn ruler_tick(cursor_axis: f32, axis_origin: f32, origin_tick: f64, px_per_tick: f64, total_ticks: u64) -> u64 {
    let tick = origin_tick + (cursor_axis - axis_origin) as f64 / px_per_tick;
    tick.clamp(0.0, total_ticks as f64) as u64
}

impl MouseState {
    pub fn set_cursor_pos(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
    }

    /// Update the window cursor icon based on what's under the pointer, and
    /// `app.hover` (read by `transport.rs`/`fkey_bar.rs` for hover styling).
    /// Returns `true` when the hovered action changed, so the caller only
    /// redraws when the hover styling would actually differ.
    pub fn update_hover(&mut self, app: &mut App, window: &Window) -> bool {
        let (x, y) = self.cursor;
        let action = app.hit_map.hit_test(x, y).copied();
        let hovering = action
            .map(|a| !matches!(a, HitAction::TrackList | HitAction::PianoRoll))
            .unwrap_or(false);
        if hovering != self.hovering {
            self.hovering = hovering;
            window.set_cursor(if hovering { CursorIcon::Pointer } else { CursorIcon::Default });
        }
        let changed = app.hover != action;
        app.hover = action;
        changed
    }

    /// Continues an in-progress seek-bar drag from a `CursorMoved` event.
    /// Only re-seeks when the target tick actually changed (throttle), and
    /// keeps working even if the cursor has left the bar vertically, since
    /// the drag's `x0`/`width` were captured at press time.
    pub fn handle_drag(&mut self, app: &mut App) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        let (x, _) = self.cursor;
        let tick = seek_bar_tick(x, drag.x0, drag.width, drag.total_ticks);
        if tick == drag.last_tick {
            return false;
        }
        app.seek_to_tick(tick);
        self.drag.as_mut().unwrap().last_tick = tick;
        true
    }

    /// Ends an in-progress seek-bar drag (left button released).
    pub fn end_drag(&mut self) {
        self.drag = None;
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
            HitAction::Ruler { axis_origin, origin_tick, px_per_tick, vertical } => {
                let cursor_axis = if vertical { y } else { x };
                app.seek_to_tick(ruler_tick(cursor_axis, axis_origin, origin_tick, px_per_tick, app.total_ticks));
                true
            }
            HitAction::PlayPause => {
                app.toggle_play();
                true
            }
            HitAction::Stop => {
                app.stop();
                true
            }
            HitAction::SeekBar { x0, width, total_ticks } => {
                let tick = seek_bar_tick(x, x0, width, total_ticks);
                app.seek_to_tick(tick);
                self.drag = Some(SeekDrag { x0, width, total_ticks, last_tick: tick });
                true
            }
            HitAction::Volume { x0, block_w, gap } => {
                let idx = volume_block_index(x, x0, block_w, gap);
                app.set_volume((idx as u32 + 1) * 10);
                true
            }
            HitAction::FKey(action) => {
                match action {
                    FKeyAction::PlayPause => app.toggle_play(),
                    FKeyAction::Stop => app.stop(),
                    FKeyAction::OpenMidi => open_browser(app, BrowseTarget::Midi),
                    FKeyAction::OpenSf2 => open_browser(app, BrowseTarget::Sf2),
                    FKeyAction::Mute => app.toggle_mute_selected(),
                    FKeyAction::Detail => app.toggle_track_view_mode(),
                    FKeyAction::Vert => app.toggle_piano_roll_orientation(),
                    FKeyAction::Mode => {
                        let next = next_midi_mode(&app.midi_mode);
                        app.set_midi_mode(next);
                    }
                    FKeyAction::Help => app.toggle_help(),
                }
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
        let over_volume = app
            .hit_map
            .hit_test_where(x, y, |a| matches!(a, HitAction::Volume { .. }))
            .is_some();
        if over_volume {
            // Same px(40) step as the piano roll's PixelDelta accumulator.
            return self.handle_volume_wheel(app, delta, roll_step_px);
        }

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

    /// Wheel over the volume blocks: wheel up = louder, one `volume_up`/`volume_down`
    /// step per line, or per `step_px` of `PixelDelta`.
    fn handle_volume_wheel(&mut self, app: &mut App, delta: MouseScrollDelta, step_px: f32) -> bool {
        match delta {
            MouseScrollDelta::LineDelta(_, dy) => {
                let n = dy.round() as i32;
                for _ in 0..n {
                    app.volume_up();
                }
                for _ in 0..(-n) {
                    app.volume_down();
                }
                n != 0
            }
            MouseScrollDelta::PixelDelta(pos) => {
                if step_px <= 0.0 {
                    return false;
                }
                self.vol_wheel_accum += pos.y as f32;
                let mut moved = false;
                while self.vol_wheel_accum >= step_px {
                    app.volume_up();
                    self.vol_wheel_accum -= step_px;
                    moved = true;
                }
                while self.vol_wheel_accum <= -step_px {
                    app.volume_down();
                    self.vol_wheel_accum += step_px;
                    moved = true;
                }
                moved
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_tick_positive_px_per_tick_moves_forward_with_cursor() {
        // origin at axis=100 => tick=1000; +100px at 0.5 px/tick => +200 ticks.
        assert_eq!(ruler_tick(200.0, 100.0, 1000.0, 0.5, 100_000), 1200);
    }

    #[test]
    fn ruler_tick_negative_px_per_tick_moves_backward_with_cursor() {
        // Falling vertical flow: moving down (increasing axis) decreases the tick.
        assert_eq!(ruler_tick(200.0, 100.0, 1000.0, -0.5, 100_000), 800);
    }

    #[test]
    fn ruler_tick_clamps_to_zero() {
        assert_eq!(ruler_tick(0.0, 1000.0, 0.0, 1.0, 100_000), 0);
    }

    #[test]
    fn ruler_tick_clamps_to_total_ticks() {
        assert_eq!(ruler_tick(200_000.0, 0.0, 0.0, 1.0, 100_000), 100_000);
    }

    #[test]
    fn seek_bar_tick_maps_midpoint_to_half_ticks() {
        assert_eq!(seek_bar_tick(150.0, 100.0, 100.0, 200_000), 100_000);
    }

    #[test]
    fn seek_bar_tick_clamps_before_bar_start() {
        assert_eq!(seek_bar_tick(50.0, 100.0, 100.0, 200_000), 0);
    }

    #[test]
    fn seek_bar_tick_clamps_past_bar_end() {
        assert_eq!(seek_bar_tick(500.0, 100.0, 100.0, 200_000), 200_000);
    }

    #[test]
    fn volume_block_index_picks_first_block() {
        assert_eq!(volume_block_index(5.0, 0.0, 10.0, 2.0), 0);
    }

    #[test]
    fn volume_block_index_picks_later_block() {
        // stride = 12; x=25 -> index 2.
        assert_eq!(volume_block_index(25.0, 0.0, 10.0, 2.0), 2);
    }

    #[test]
    fn volume_block_index_clamps_to_last_block() {
        assert_eq!(volume_block_index(1000.0, 0.0, 10.0, 2.0), 9);
    }

    #[test]
    fn volume_block_index_clamps_before_first_block() {
        assert_eq!(volume_block_index(-50.0, 0.0, 10.0, 2.0), 0);
    }
}
