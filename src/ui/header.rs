//! Header utilities.

use unicode_width::UnicodeWidthChar;

use crate::renderer::types::Color;
use crate::ui::text::text_cells;

/// Formats a duration as `MM:SS.cc` (centiseconds), for the transport bar's
/// large current-time readout and total-time display.
pub fn format_duration(secs: f64) -> String {
    let total_cs = (secs * 100.0).round() as u64;
    let cs = total_cs % 100;
    let total_sec = total_cs / 100;
    let sec = total_sec % 60;
    let min = total_sec / 60;
    format!("{:02}:{:02}.{:02}", min, sec, cs)
}

/// Identifies a header metadata item, used to pick a fixed drop order when
/// the header runs out of horizontal space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeaderItemKind {
    Smf,
    Tpqn,
    Notes,
    Tracks,
    Ports,
    TimeSig,
    Bpm,
    Mode,
    Sf2,
}

/// A single right-aligned metadata item shown in the header bar.
pub struct HeaderItem {
    /// Which item this is, used to look it up in `HEADER_DROP_ORDER`.
    pub kind: HeaderItemKind,
    /// Fully formatted display text (e.g. "TPQN 480").
    pub text: String,
    /// Text color to draw this item with.
    pub color: Color,
}

/// Order in which header items are dropped once they no longer fit.
/// `Ports` is dropped immediately after `Tracks`.
const HEADER_DROP_ORDER: [HeaderItemKind; 9] = [
    HeaderItemKind::Tracks,
    HeaderItemKind::Ports,
    HeaderItemKind::Notes,
    HeaderItemKind::Tpqn,
    HeaderItemKind::Smf,
    HeaderItemKind::Sf2,
    HeaderItemKind::TimeSig,
    HeaderItemKind::Bpm,
    HeaderItemKind::Mode,
];

/// Selects which header items fit into `available` width, dropping items in
/// `HEADER_DROP_ORDER` until what remains (items plus separators) fits.
/// Returns the kept indices in their original order.
pub fn fit_header_items(
    kinds: &[HeaderItemKind],
    widths: &[f32],
    sep_width: f32,
    available: f32,
) -> Vec<usize> {
    let mut kept: Vec<usize> = (0..kinds.len()).collect();

    let width_of = |kept: &[usize]| -> f32 {
        if kept.is_empty() {
            0.0
        } else {
            kept.iter().map(|&i| widths[i]).sum::<f32>() + sep_width * (kept.len() - 1) as f32
        }
    };

    for drop_kind in HEADER_DROP_ORDER {
        if kept.is_empty() || width_of(&kept) <= available {
            break;
        }
        kept.retain(|&i| kinds[i] != drop_kind);
    }

    kept
}

/// Formats a non-negative count with comma thousands separators (e.g. 5,812).
pub fn format_thousands(n: usize) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Truncates `name` to at most `max_cells` display cells, never splitting a
/// wide character in half.
fn truncate_cells(name: &str, max_cells: usize) -> String {
    let mut result = String::new();
    let mut used = 0usize;
    for ch in name.chars() {
        let w = UnicodeWidthChar::width_cjk(ch).unwrap_or(1);
        if used + w > max_cells {
            break;
        }
        result.push(ch);
        used += w;
    }
    result
}

/// Shortens an SF2 preset name for header display: names over 24 cells are
/// cut to 21 cells (without splitting a wide character) plus an ASCII "..."
/// suffix (U+2026 is East Asian Ambiguous and would break width accounting).
pub fn truncate_sf2_name(name: &str) -> String {
    if text_cells(name) > 24 {
        format!("{}...", truncate_cells(name, 21))
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_includes_centiseconds() {
        assert_eq!(format_duration(0.0), "00:00.00");
        assert_eq!(format_duration(65.5), "01:05.50");
        assert_eq!(format_duration(3661.005), "61:01.01");
    }

    #[test]
    fn fit_all_items_when_space_allows() {
        let kinds = [HeaderItemKind::Tracks, HeaderItemKind::Notes, HeaderItemKind::Mode];
        let widths = [10.0, 10.0, 10.0];
        let kept = fit_header_items(&kinds, &widths, 2.0, 1000.0);
        assert_eq!(kept, vec![0, 1, 2]);
    }

    #[test]
    fn narrow_width_drops_tracks_then_notes_in_order() {
        let kinds = [
            HeaderItemKind::Tracks,
            HeaderItemKind::Notes,
            HeaderItemKind::Tpqn,
            HeaderItemKind::Mode,
        ];
        let widths = [10.0, 10.0, 10.0, 10.0];
        let sep = 2.0;
        // All four items: 40 + 3*2 = 46. Dropping Tracks: 30 + 2*2 = 34.
        // Dropping Notes too: 20 + 1*2 = 22.
        let kept = fit_header_items(&kinds, &widths, sep, 34.0);
        assert_eq!(kept, vec![1, 2, 3]); // Tracks dropped, others kept

        let kept = fit_header_items(&kinds, &widths, sep, 22.0);
        assert_eq!(kept, vec![2, 3]); // Tracks then Notes dropped, in that order
    }

    #[test]
    fn ports_dropped_right_after_tracks() {
        let kinds = [
            HeaderItemKind::Tracks,
            HeaderItemKind::Ports,
            HeaderItemKind::Notes,
            HeaderItemKind::Mode,
        ];
        let widths = [10.0, 10.0, 10.0, 10.0];
        let sep = 2.0;
        // Dropping Tracks alone (30 + 2*2=34) still doesn't fit at 25;
        // Ports must go too (20 + 1*2 = 22) before Notes is touched.
        let kept = fit_header_items(&kinds, &widths, sep, 25.0);
        assert_eq!(kept, vec![2, 3]);
    }

    #[test]
    fn nothing_fits_returns_empty() {
        let kinds = [HeaderItemKind::Mode];
        let widths = [10.0];
        let kept = fit_header_items(&kinds, &widths, 2.0, 5.0);
        assert!(kept.is_empty());
    }

    #[test]
    fn format_thousands_cases() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(5812), "5,812");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }

    #[test]
    fn truncate_sf2_name_keeps_short_ascii() {
        assert_eq!(truncate_sf2_name("GM.sf2"), "GM.sf2");
    }

    #[test]
    fn truncate_sf2_name_cuts_long_ascii() {
        let name = "a".repeat(30);
        let result = truncate_sf2_name(&name);
        assert_eq!(result, format!("{}...", "a".repeat(21)));
        assert_eq!(text_cells(&result), 24);
    }

    #[test]
    fn truncate_sf2_name_never_splits_wide_char() {
        let name = "\u{65E5}".repeat(15); // 15 CJK chars = 30 cells
        let result = truncate_sf2_name(&name);
        assert!(text_cells(&result) <= 24);
        // Body before "..." must be an integer number of whole CJK chars.
        let body = result.trim_end_matches("...");
        assert_eq!(text_cells(body) % 2, 0);
    }
}
