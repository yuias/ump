//! Bottom function-key bar: quick action hints, PC-98 style.

use crate::app::App;
use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::layout::px;
use crate::ui::text::text_width;
use crate::ui::theme;

/// Action bound to a function-key bar slot. Keyboard equivalents keep working
/// from `input.rs` even when a slot is dropped for lack of space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FKeyAction {
    PlayPause,
    Stop,
    OpenMidi,
    OpenSf2,
    Mute,
    Detail,
    Vert,
    Mode,
    Help,
}

/// Number of items that fit into `area_width` split into equal slots: starts
/// at all items and drops from the right until every remaining slot is wide
/// enough for its key text, `longest_label_w` (the longest label over all
/// items, so `n` doesn't flicker as labels change, e.g. PLAY/PAUSE), and
/// `3*inset` (left key inset, key-label gap, right label inset).
fn fit_fkey_items(area_width: f32, key_widths: &[f32], longest_label_w: f32, inset: f32) -> usize {
    let mut n = key_widths.len();
    while n > 0 {
        let slot_w = area_width / n as f32;
        let widest_key = key_widths[..n].iter().cloned().fold(0.0f32, f32::max);
        if slot_w >= widest_key + longest_label_w + 3.0 * inset {
            break;
        }
        n -= 1;
    }
    n
}

/// Cycles GM -> GS -> XG -> GM2 -> GM; unknown modes reset to GM.
pub fn next_midi_mode(current: &str) -> &'static str {
    match current {
        "GM" => "GS",
        "GS" => "XG",
        "XG" => "GM2",
        "GM2" => "GM",
        _ => "GM",
    }
}

pub fn render_fkey_bar(renderer: &mut dyn Renderer, area: Rect, app: &App, hits: &mut HitMap) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let inset = px(4.0, scale);

    let play_label = if app.is_playing() && !app.is_finished() { "PAUSE" } else { "PLAY" };

    let items: [(&str, &str, FKeyAction); 9] = [
        ("SPC", play_label, FKeyAction::PlayPause),
        ("S", "STOP", FKeyAction::Stop),
        ("O", "OPEN", FKeyAction::OpenMidi),
        ("F", "SF2", FKeyAction::OpenSf2),
        ("M", "MUTE", FKeyAction::Mute),
        ("E", "DETAIL", FKeyAction::Detail),
        ("V", "VERT", FKeyAction::Vert),
        ("1-4", "MODE", FKeyAction::Mode),
        ("?", "HELP", FKeyAction::Help),
    ];

    let key_widths: Vec<f32> = items.iter().map(|(key, _, _)| text_width(key, cw)).collect();
    let longest_label_w = items
        .iter()
        .map(|(_, label, _)| text_width(label, cw))
        .fold(0.0f32, f32::max);

    let n = fit_fkey_items(area.width, &key_widths, longest_label_w, inset);
    if n == 0 {
        return;
    }

    let slot_w = area.width / n as f32;
    let label_h = area.height - 2.0 * px(2.0, scale);
    let label_y = area.y + px(2.0, scale);
    let key_y = area.y + (area.height - ch) / 2.0;

    for (i, (key, label, action)) in items.iter().take(n).enumerate() {
        let slot_x = area.x + slot_w * i as f32;
        let key_x = slot_x + inset;
        renderer.draw_text(key_x, key_y, key, theme::DIM, ch);

        let label_x0 = key_x + text_width(key, cw) + inset;
        let label_w = (slot_x + slot_w - inset - label_x0).max(0.0);
        let label_rect = Rect::new(label_x0, label_y, label_w, label_h);

        let hovered = matches!(app.hover, Some(HitAction::FKey(a)) if a == *action);
        renderer.fill_rect(label_rect, if hovered { theme::ACCENT } else { theme::FKEY_BG });

        let label_w_px = text_width(label, cw);
        let lx = label_rect.x + (label_rect.width - label_w_px) / 2.0;
        let ly = label_rect.y + (label_rect.height - ch) / 2.0;
        renderer.draw_text(lx, ly, label, theme::FKEY_FG, ch);

        hits.push(Rect::new(slot_x, area.y, slot_w, area.height), HitAction::FKey(*action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_items_fit_when_space_allows() {
        let widths = [30.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 30.0, 10.0];
        assert_eq!(fit_fkey_items(2000.0, &widths, 60.0, 4.0), 9);
    }

    #[test]
    fn narrow_width_drops_items_from_the_right() {
        let widths = [30.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 30.0, 10.0];
        // Required slot width is 30+60+12=102 (widest key among the first n is
        // always 30, from item 0). n=4: slot=100<102, drop. n=3: slot=133.3>=102, keep.
        let n = fit_fkey_items(400.0, &widths, 60.0, 4.0);
        assert_eq!(n, 3);
    }

    #[test]
    fn zero_width_drops_everything() {
        let widths = [30.0, 10.0];
        assert_eq!(fit_fkey_items(0.0, &widths, 60.0, 4.0), 0);
    }

    #[test]
    fn mode_cycle_order() {
        assert_eq!(next_midi_mode("GM"), "GS");
        assert_eq!(next_midi_mode("GS"), "XG");
        assert_eq!(next_midi_mode("XG"), "GM2");
        assert_eq!(next_midi_mode("GM2"), "GM");
        assert_eq!(next_midi_mode("unknown"), "GM");
    }
}
