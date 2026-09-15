//! Text width helpers for pixel-based text layout.
//!
//! Uses `width_cjk` (East Asian Width "Ambiguous" = wide) rather than `.len()`
//! or the default `width()`, because the fonts this app bundles/recommends
//! render Ambiguous glyphs full-width, and `.len()` counts bytes, not cells.

use unicode_width::UnicodeWidthStr;

/// Number of monospace cells `s` occupies when rendered.
pub fn text_cells(s: &str) -> usize {
    UnicodeWidthStr::width_cjk(s)
}

/// Pixel width of `s` at cell width `cell_w`.
pub fn text_width(s: &str, cell_w: f32) -> f32 {
    text_cells(s) as f32 * cell_w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_counts_one_cell_per_char() {
        assert_eq!(text_cells("hello"), 5);
    }

    #[test]
    fn cjk_counts_two_cells_per_char() {
        assert_eq!(text_cells("日本語"), 6);
    }

    #[test]
    fn ambiguous_char_counts_as_wide() {
        assert_eq!(text_cells("\u{25CB}"), 2); // ○
    }

    #[test]
    fn text_width_multiplies_by_cell_width() {
        assert_eq!(text_width("hello", 8.0), 40.0);
    }
}
