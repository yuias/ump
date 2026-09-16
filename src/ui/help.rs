//! Help overlay: displays keybindings via native pixel rendering.

use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::border::draw_panel;
use crate::ui::layout;
use crate::ui::text::text_width;
use crate::ui::theme;

const BINDINGS: &[(&str, &str)] = &[
    ("Space", "Play / Pause"),
    ("S", "Stop"),
    ("Left / Right", "Seek -5s / +5s"),
    ("Up / Down", "Cursor Up / Down"),
    ("M", "Mute / Unmute track"),
    ("V", "Toggle piano roll orientation"),
    ("Shift+V", "Toggle vertical flow (falling / rising)"),
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
    let scale = renderer.scale_factor();
    let w_f = w as f32;
    let h_f = h as f32;

    // Centered popup: 50% width, 70% height
    let popup_w = w_f * 0.50;
    let popup_h = h_f * 0.70;
    let popup_x = (w_f - popup_w) / 2.0;
    let popup_y = (h_f - popup_h) / 2.0;

    let area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    let title_h = layout::title_height(ch, scale);
    draw_panel(renderer, area, "HELP", title_h);

    // Content
    let dot = layout::dot_size(scale);
    let inner = layout::panel_content(area, title_h, dot);

    let mut y = inner.y + ch;
    for (key, desc) in BINDINGS {
        if y + ch > inner.y + inner.height {
            break;
        }
        let key_str = format!("{:>12}  ", key);
        renderer.draw_text_bold(inner.x + cw, y, &key_str, theme::ACCENT, ch);
        let desc_x = inner.x + cw + text_width(&key_str, cw);
        renderer.draw_text(desc_x, y, desc, theme::TEXT, ch);
        y += ch;
    }
}
