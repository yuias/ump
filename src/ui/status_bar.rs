//! Bottom status bar: quick action hints.

use crate::renderer::types::{Color, Rect};
use crate::renderer::Renderer;
use crate::ui::layout::SECTION_PADDING_X;

pub fn render_status_bar(renderer: &mut dyn Renderer, area: Rect) {
    let (cw, ch) = renderer.cell_size();
    let key_fg = Color::rgb(20, 20, 30);
    let key_bg = Color::rgb(160, 160, 180);
    let desc_fg = Color::rgb(140, 140, 160);

    let mut x = area.x + SECTION_PADDING_X as f32 * cw;
    let y = area.y;

    let badges: &[(&str, &str)] = &[
        (" Space ", "Play "),
        (" O ", "Open "),
        (" F ", "SF2 "),
        (" M ", "Mute "),
        (" +/- ", "Vol "),
        (" E ", "Ext "),
        (" S ", "Stop "),
        (" ? ", "Help "),
        (" Q ", "Quit "),
    ];

    for (key, desc) in badges {
        // Key badge: background rect + bold text
        let key_w = key.len() as f32 * cw;
        renderer.fill_rect(Rect::new(x, y, key_w, ch), key_bg);
        renderer.draw_text_bold(x, y, key, key_fg, ch);
        x += key_w;

        // Description text
        renderer.draw_text(x, y, desc, desc_fg, ch);
        x += desc.len() as f32 * cw;

        // Gap
        x += cw;
    }
}
