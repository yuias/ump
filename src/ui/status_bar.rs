//! Bottom status bar: quick action hints.

use crate::renderer::types::Rect;
use crate::renderer::Renderer;
use crate::ui::layout::px;
use crate::ui::text::text_width;
use crate::ui::theme;

pub fn render_status_bar(renderer: &mut dyn Renderer, area: Rect) {
    let (cw, ch) = renderer.cell_size();
    let scale = renderer.scale_factor();
    let key_fg = theme::FKEY_FG;
    let key_bg = theme::FKEY_BG;
    let desc_fg = theme::DIM;

    let mut x = area.x + px(6.0, scale);
    let y = area.y;

    let badges: &[(&str, &str)] = &[
        (" Space ", "Play "),
        (" O ", "Open "),
        (" F ", "SF2 "),
        (" P ", "Panel "),
        (" M ", "Mute "),
        (" +/- ", "Vol "),
        (" E ", "View "),
        (" S ", "Stop "),
        (" ? ", "Help "),
    ];

    for (key, desc) in badges {
        // Key badge: background rect + bold text
        let key_w = text_width(key, cw);
        renderer.fill_rect(Rect::new(x, y, key_w, ch), key_bg);
        renderer.draw_text_bold(x, y, key, key_fg, ch);
        x += key_w;

        // Description text
        renderer.draw_text(x, y, desc, desc_fg, ch);
        x += text_width(desc, cw);

        // Gap
        x += cw;
    }
}
