//! Text width helpers for pixel-based text layout.
//!
//! Uses `width_cjk` (East Asian Width "Ambiguous" = wide) rather than `.len()`
//! or the default `width()`, because the fonts this app bundles/recommends
//! render Ambiguous glyphs full-width, and `.len()` counts bytes, not cells.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Number of monospace cells `s` occupies when rendered.
pub fn text_cells(s: &str) -> usize {
    UnicodeWidthStr::width_cjk(s)
}

/// Pixel width of `s` at cell width `cell_w`.
pub fn text_width(s: &str, cell_w: f32) -> f32 {
    text_cells(s) as f32 * cell_w
}

/// Truncate `s` to at most `max_cells` cells, never splitting a wide char.
pub fn truncate_cells(s: &str, max_cells: usize) -> String {
    let mut result = String::new();
    let mut used = 0usize;
    for c in s.chars() {
        let w = UnicodeWidthChar::width_cjk(c).unwrap_or(1);
        if used + w > max_cells {
            break;
        }
        result.push(c);
        used += w;
    }
    result
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

    #[test]
    fn truncate_cells_keeps_short_string_intact() {
        assert_eq!(truncate_cells("hi", 8), "hi");
    }

    #[test]
    fn truncate_cells_cuts_ascii_at_limit() {
        assert_eq!(truncate_cells("hello world", 5), "hello");
    }

    #[test]
    fn truncate_cells_never_splits_a_wide_char() {
        // "日" is 2 cells; budget of 3 can only fit one narrow char after it.
        assert_eq!(truncate_cells("a日b", 2), "a");
        assert_eq!(truncate_cells("a日b", 3), "a日");
    }
}
