//! Help overlay: displays keybindings via native pixel rendering.

use crate::renderer::types::{BG_COLOR, Color, Rect};
use crate::renderer::Renderer;
use crate::ui::border::{draw_border, inner_rect};

const BINDINGS: &[(&str, &str)] = &[
    ("Space", "Play / Pause"),
    ("S", "Stop"),
    ("\u{2190} / \u{2192}", "Seek -5s / +5s"),
    ("\u{2191} / \u{2193}", "Cursor Up / Down"),
    ("M", "Mute / Unmute track"),
    ("P", "Toggle right panel (Monitor/Piano Roll)"),
    ("V", "Toggle piano roll orientation"),
    ("E", "Toggle track view (Default/Detail)"),
    ("+/-", "Volume Up / Down"),
    ("[ / ]", "Zoom Out / In"),
    ("Tab", "Cycle focus panel"),
    ("O", "Open MIDI file"),
    ("F", "Open SF2 file"),
    ("D", "Reset SF2 to default"),
    ("PgUp/PgDn", "Switch port (multi-port)"),
    ("1-4", "Set mode (GM/GS/XG/GM2)"),
    ("?", "Toggle this help"),
    ("Q / Esc", "Quit"),
];

pub fn render_help(renderer: &mut dyn Renderer) {
    let (w, h) = renderer.window_size();
    let (cw, ch) = renderer.cell_size();
    let w_f = w as f32;
    let h_f = h as f32;

    // Centered popup: 50% width, 70% height
    let popup_w = w_f * 0.50;
    let popup_h = h_f * 0.70;
    let popup_x = (w_f - popup_w) / 2.0;
    let popup_y = (h_f - popup_h) / 2.0;

    let area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    // Clear popup region
    renderer.fill_rect(area, BG_COLOR);

    // Draw border
    let border_color = Color::rgb(100, 180, 255);
    draw_border(renderer, area, " Keybindings ", border_color);

    // Content
    let inner = inner_rect(area, cw, ch);
    let key_color = Color::rgb(100, 200, 255);
    let desc_color = Color::rgb(200, 200, 220);

    let mut y = inner.y + ch;
    for (key, desc) in BINDINGS {
        if y + ch > inner.y + inner.height {
            break;
        }
        let key_str = format!("{:>12}  ", key);
        renderer.draw_text_bold(inner.x + cw, y, &key_str, key_color, ch);
        let desc_x = inner.x + cw + key_str.len() as f32 * cw;
        renderer.draw_text(desc_x, y, desc, desc_color, ch);
        y += ch;
    }
}
