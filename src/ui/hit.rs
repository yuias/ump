//! Mouse hit-region map: components register clickable rects during render,
//! the input layer queries them after the frame without re-deriving layout.

use crate::renderer::types::Rect;

/// Action bound to a hit region. `usize` is the raw row index as used by
/// `app.track_cursor` / `build_raw_rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitAction {
    TrackRow(usize),
    TrackMute(usize),
    /// The whole track list area, used for wheel scrolling even when a more
    /// specific row region is stacked on top of it.
    TrackList,
    PortTab(u8),
    OpenMidi,
}

pub struct HitRegion {
    pub rect: Rect,
    pub action: HitAction,
}

/// Regions registered during the current frame's render pass.
#[derive(Default)]
pub struct HitMap {
    regions: Vec<HitRegion>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn push(&mut self, rect: Rect, action: HitAction) {
        self.regions.push(HitRegion { rect, action });
    }

    /// Last pushed wins: more specific regions pushed after broader ones
    /// (e.g. a mute box on top of its row) take priority.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<&HitAction> {
        self.hit_test_where(x, y, |_| true)
    }

    /// Like `hit_test`, but only considers regions matching `pred`. Used to
    /// find a broader region (e.g. `TrackList`) under the cursor even when a
    /// more specific region is on top of it for plain hit testing.
    pub fn hit_test_where(
        &self,
        x: f32,
        y: f32,
        pred: impl Fn(&HitAction) -> bool,
    ) -> Option<&HitAction> {
        self.regions
            .iter()
            .rev()
            .find(|r| pred(&r.action) && contains(r.rect, x, y))
            .map(|r| &r.action)
    }
}

/// Left/top inclusive, right/bottom exclusive.
fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_pushed_wins() {
        let mut hits = HitMap::default();
        hits.push(Rect::new(0.0, 0.0, 100.0, 100.0), HitAction::TrackList);
        hits.push(Rect::new(10.0, 10.0, 20.0, 20.0), HitAction::TrackRow(3));
        assert_eq!(hits.hit_test(15.0, 15.0), Some(&HitAction::TrackRow(3)));
        assert_eq!(hits.hit_test(50.0, 50.0), Some(&HitAction::TrackList));
    }

    #[test]
    fn hit_test_where_finds_broader_region_under_specific_one() {
        let mut hits = HitMap::default();
        hits.push(Rect::new(0.0, 0.0, 100.0, 100.0), HitAction::TrackList);
        hits.push(Rect::new(10.0, 10.0, 20.0, 20.0), HitAction::TrackRow(3));
        let found = hits.hit_test_where(15.0, 15.0, |a| matches!(a, HitAction::TrackList));
        assert_eq!(found, Some(&HitAction::TrackList));
    }

    #[test]
    fn bounds_left_top_inclusive_right_bottom_exclusive() {
        let mut hits = HitMap::default();
        hits.push(Rect::new(10.0, 10.0, 10.0, 10.0), HitAction::TrackRow(0));
        assert_eq!(hits.hit_test(10.0, 10.0), Some(&HitAction::TrackRow(0))); // left/top inclusive
        assert_eq!(hits.hit_test(19.99, 19.99), Some(&HitAction::TrackRow(0)));
        assert_eq!(hits.hit_test(20.0, 15.0), None); // right exclusive
        assert_eq!(hits.hit_test(15.0, 20.0), None); // bottom exclusive
        assert_eq!(hits.hit_test(9.99, 15.0), None); // left of rect
        assert_eq!(hits.hit_test(15.0, 9.99), None); // above rect
    }

    #[test]
    fn no_match_returns_none() {
        let hits = HitMap::default();
        assert_eq!(hits.hit_test(0.0, 0.0), None);
    }

    #[test]
    fn clear_removes_all_regions() {
        let mut hits = HitMap::default();
        hits.push(Rect::new(0.0, 0.0, 10.0, 10.0), HitAction::OpenMidi);
        hits.clear();
        assert_eq!(hits.hit_test(5.0, 5.0), None);
    }
}
