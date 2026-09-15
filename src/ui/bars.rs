//! Bar/beat map: converts MIDI ticks to (bar, beat) and back, built once per
//! MIDI load from `TimeSignature` events. Default 4/4 from tick 0 when a
//! song carries no time-signature meta events.

use ump_playback::midi::event::{MidiEvent, TimedMidiEvent};

/// One time-signature-stable stretch of the song: everything from
/// `start_tick` up to the next segment's `start_tick` (or the end of the
/// song) shares `ticks_per_bar`/`ticks_per_beat`, with bar numbering
/// continuing from `bar_at_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    start_tick: u64,
    /// 1-based bar number at `start_tick`.
    bar_at_start: u32,
    ticks_per_bar: u64,
    ticks_per_beat: u64,
}

/// Maps MIDI ticks to musical bar/beat position and back.
#[derive(Debug, Clone)]
pub struct BarMap {
    /// Sorted by `start_tick` (and by `bar_at_start`); always has at least
    /// one entry starting at tick 0.
    segments: Vec<Segment>,
}

/// Ticks per beat for a time signature, where `denominator` is the power-of-two
/// exponent (2 = quarter note beat, 3 = eighth note beat, ...) as used throughout
/// the codebase (see `App::time_signature`).
fn beat_ticks(ticks_per_quarter: u64, denominator: u8) -> u64 {
    ((ticks_per_quarter * 4) >> denominator).max(1)
}

fn sig_ticks(ticks_per_quarter: u64, numerator: u8, denominator: u8) -> (u64, u64) {
    let ticks_per_beat = beat_ticks(ticks_per_quarter, denominator);
    let ticks_per_bar = ticks_per_beat * numerator.max(1) as u64;
    (ticks_per_beat, ticks_per_bar)
}

impl BarMap {
    /// Build from the song's PPQN and its (unsorted) event list. Only
    /// `TimeSignature` events matter; everything else is ignored.
    ///
    /// A signature change that lands mid-bar truncates the bar in progress:
    /// the next bar starts immediately at the change's tick, rather than
    /// waiting for the previous bar to finish (common SMF practice puts
    /// changes on bar lines anyway, so this only matters for malformed input).
    pub fn build(ticks_per_quarter: u16, events: &[TimedMidiEvent]) -> Self {
        let tpq = (ticks_per_quarter as u64).max(1);

        let mut ts_events: Vec<(u64, u8, u8)> = events
            .iter()
            .filter_map(|e| match e.event {
                MidiEvent::TimeSignature { numerator, denominator } => {
                    Some((e.tick, numerator, denominator))
                }
                _ => None,
            })
            .collect();
        ts_events.sort_by_key(|&(tick, _, _)| tick);

        let mut segments = Vec::new();
        let mut cur_start = 0u64;
        let mut cur_bar = 1u32;
        let (mut num, mut den) = (4u8, 2u8);

        for (tick, n, d) in ts_events {
            if tick == 0 {
                // Applies from the very start: no preceding default segment.
                num = n;
                den = d;
                continue;
            }
            if tick <= cur_start {
                continue; // duplicate/out-of-order event at or before the current segment
            }

            let (_, ticks_per_bar) = sig_ticks(tpq, num, den);
            let delta = tick - cur_start;
            let bars_full = delta / ticks_per_bar;
            let mid_bar = !delta.is_multiple_of(ticks_per_bar);
            let new_bar = cur_bar + bars_full as u32 + if mid_bar { 1 } else { 0 };

            let (ticks_per_beat, ticks_per_bar) = sig_ticks(tpq, num, den);
            segments.push(Segment { start_tick: cur_start, bar_at_start: cur_bar, ticks_per_bar, ticks_per_beat });

            cur_start = tick;
            cur_bar = new_bar;
            num = n;
            den = d;
        }

        let (ticks_per_beat, ticks_per_bar) = sig_ticks(tpq, num, den);
        segments.push(Segment { start_tick: cur_start, bar_at_start: cur_bar, ticks_per_bar, ticks_per_beat });

        BarMap { segments }
    }

    /// Index of the last segment whose `start_tick <= tick`. Always in range
    /// since `segments[0].start_tick == 0`.
    fn segment_idx_for_tick(&self, tick: u64) -> usize {
        self.segments.partition_point(|s| s.start_tick <= tick) - 1
    }

    /// Index of the last segment whose `bar_at_start <= bar`. `bar` must be >= 1.
    fn segment_idx_for_bar(&self, bar: u32) -> usize {
        self.segments.partition_point(|s| s.bar_at_start <= bar).saturating_sub(1)
    }

    /// 1-based (bar, beat) at `tick`.
    pub fn bar_at(&self, tick: u64) -> (u32, u32) {
        let seg = &self.segments[self.segment_idx_for_tick(tick)];
        let delta = tick - seg.start_tick;
        let bar = seg.bar_at_start + (delta / seg.ticks_per_bar) as u32;
        let beat = 1 + ((delta % seg.ticks_per_bar) / seg.ticks_per_beat) as u32;
        (bar, beat)
    }

    /// Tick at the start of 1-based bar `bar`.
    pub fn tick_of_bar(&self, bar: u32) -> u64 {
        let seg = &self.segments[self.segment_idx_for_bar(bar)];
        seg.start_tick + (bar - seg.bar_at_start) as u64 * seg.ticks_per_bar
    }

    /// Ticks per beat of the segment covering `tick`. Used by renderers to
    /// decide whether beat lines are dense enough to draw.
    pub fn ticks_per_beat_at(&self, tick: u64) -> u64 {
        self.segments[self.segment_idx_for_tick(tick)].ticks_per_beat
    }

    /// Ticks per bar of the segment covering `tick`. Used by renderers to
    /// decide whether bar numbers are dense enough to draw.
    pub fn ticks_per_bar_at(&self, tick: u64) -> u64 {
        self.segments[self.segment_idx_for_tick(tick)].ticks_per_bar
    }

    /// Visit every beat/bar gridline in `[from, to)`, in ascending tick order.
    pub fn lines_in(&self, from: u64, to: u64) -> GridLines<'_> {
        let idx = self.segment_idx_for_tick(from);
        let seg = &self.segments[idx];
        let delta = from.saturating_sub(seg.start_tick);
        let rem = delta % seg.ticks_per_beat;
        let start_tick = if rem == 0 { from } else { from + (seg.ticks_per_beat - rem) };
        GridLines { segments: &self.segments, idx, tick: start_tick, to }
    }
}

impl Default for BarMap {
    /// Plain 4/4 at 480 TPQN, matching `App::new_empty`.
    fn default() -> Self {
        BarMap::build(480, &[])
    }
}

/// Iterator over `(tick, is_bar, bar_number)` grid lines, yielded by `BarMap::lines_in`.
pub struct GridLines<'a> {
    segments: &'a [Segment],
    idx: usize,
    tick: u64,
    to: u64,
}

impl<'a> Iterator for GridLines<'a> {
    type Item = (u64, bool, u32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.tick >= self.to {
            return None;
        }
        while self.idx + 1 < self.segments.len() && self.tick >= self.segments[self.idx + 1].start_tick {
            self.idx += 1;
        }
        let seg = &self.segments[self.idx];
        let delta = self.tick - seg.start_tick;
        let is_bar = delta.is_multiple_of(seg.ticks_per_bar);
        let bar_number = seg.bar_at_start + (delta / seg.ticks_per_bar) as u32;
        let result = (self.tick, is_bar, bar_number);
        self.tick += seg.ticks_per_beat;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts_event(tick: u64, numerator: u8, denominator: u8) -> TimedMidiEvent {
        TimedMidiEvent { tick, event: MidiEvent::TimeSignature { numerator, denominator }, track: 0 }
    }

    #[test]
    fn plain_4_4_at_480_tpqn() {
        let map = BarMap::build(480, &[]);
        assert_eq!(map.bar_at(0), (1, 1));
        assert_eq!(map.bar_at(480), (1, 2));
        assert_eq!(map.bar_at(1919), (1, 4));
        assert_eq!(map.bar_at(1920), (2, 1));
        assert_eq!(map.tick_of_bar(1), 0);
        assert_eq!(map.tick_of_bar(2), 1920);
        assert_eq!(map.tick_of_bar(3), 3840);
    }

    #[test]
    fn time_signature_change_at_bar_line() {
        // 3/4 from tick 0, switching to 6/8 exactly at the start of bar 4 (tick 4320).
        let events = [ts_event(0, 3, 2), ts_event(4320, 6, 3)];
        let map = BarMap::build(480, &events);
        assert_eq!(map.bar_at(4319), (3, 3));
        assert_eq!(map.bar_at(4320), (4, 1));
        assert_eq!(map.tick_of_bar(4), 4320);
        // 6/8 bar length: ticks_per_beat = 480*4/8 = 240, * 6 = 1440.
        assert_eq!(map.bar_at(4320 + 1440), (5, 1));
    }

    #[test]
    fn time_signature_change_mid_bar_starts_new_bar() {
        // Default 4/4 (bar 2 spans 1920..3839), but a 3/4 change lands at 2000,
        // mid-way through bar 2: bar 2 is truncated and bar 3 starts at 2000.
        let events = [ts_event(2000, 3, 2)];
        let map = BarMap::build(480, &events);
        assert_eq!(map.bar_at(1919), (1, 4));
        assert_eq!(map.bar_at(1920), (2, 1));
        assert_eq!(map.bar_at(1999), (2, 1));
        assert_eq!(map.bar_at(2000), (3, 1));
        assert_eq!(map.tick_of_bar(3), 2000);
    }

    #[test]
    fn bar_at_on_exact_beat_boundaries() {
        let map = BarMap::build(480, &[]);
        for beat in 0..4u32 {
            assert_eq!(map.bar_at(beat as u64 * 480), (1, beat + 1));
        }
        assert_eq!(map.bar_at(1920), (2, 1));
    }

    #[test]
    fn tick_of_bar_round_trips_with_bar_at() {
        let events = [ts_event(2000, 3, 2)];
        let map = BarMap::build(480, &events);
        for bar in 1..=6u32 {
            let tick = map.tick_of_bar(bar);
            assert_eq!(map.bar_at(tick), (bar, 1));
        }
    }

    #[test]
    fn grid_lines_flag_bar_starts() {
        let map = BarMap::build(480, &[]);
        let lines: Vec<(u64, bool, u32)> = map.lines_in(0, 1921).collect();
        // Beats at 0,480,960,1440 (bar 1), then 1920 starts bar 2.
        assert_eq!(lines, vec![
            (0, true, 1),
            (480, false, 1),
            (960, false, 1),
            (1440, false, 1),
            (1920, true, 2),
        ]);
    }

    #[test]
    fn grid_lines_snaps_start_to_next_beat_when_from_is_mid_beat() {
        let map = BarMap::build(480, &[]);
        let lines: Vec<(u64, bool, u32)> = map.lines_in(100, 1000).collect();
        assert_eq!(lines[0].0, 480);
    }
}
