//! Tempo map for tick-to-seconds conversion.

#[derive(Debug, Clone)]
struct TempoEntry {
    tick: u64,
    /// Cumulative time in seconds at this tick.
    time_secs: f64,
    /// Microseconds per quarter note at this point.
    us_per_quarter: u32,
}

#[derive(Debug, Clone)]
pub struct TempoMap {
    entries: Vec<TempoEntry>,
    ticks_per_quarter: u16,
}

impl TempoMap {
    /// Build a tempo map from a sequence of (tick, microseconds_per_quarter) pairs.
    /// The pairs must be sorted by tick. If no tempo events exist, default 120 BPM is used.
    pub fn new(ticks_per_quarter: u16, tempo_changes: &[(u64, u32)]) -> Self {
        let mut entries = Vec::new();

        if tempo_changes.is_empty() || tempo_changes[0].0 != 0 {
            // Default 120 BPM = 500000 us/quarter
            entries.push(TempoEntry {
                tick: 0,
                time_secs: 0.0,
                us_per_quarter: 500_000,
            });
        }

        let tpq = ticks_per_quarter as f64;

        for &(tick, us_per_q) in tempo_changes {
            let time = if let Some(last) = entries.last() {
                let delta_ticks = tick.saturating_sub(last.tick) as f64;
                let secs_per_tick = last.us_per_quarter as f64 / 1_000_000.0 / tpq;
                last.time_secs + delta_ticks * secs_per_tick
            } else {
                0.0
            };
            entries.push(TempoEntry {
                tick,
                time_secs: time,
                us_per_quarter: us_per_q,
            });
        }

        TempoMap {
            entries,
            ticks_per_quarter,
        }
    }

    /// Convert a tick position to seconds.
    pub fn tick_to_secs(&self, tick: u64) -> f64 {
        let idx = match self.entries.binary_search_by_key(&tick, |e| e.tick) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let entry = &self.entries[idx];
        let delta = tick.saturating_sub(entry.tick) as f64;
        let tpq = self.ticks_per_quarter as f64;
        let secs_per_tick = entry.us_per_quarter as f64 / 1_000_000.0 / tpq;
        entry.time_secs + delta * secs_per_tick
    }

    /// Convert a time in seconds to the nearest tick.
    pub fn secs_to_tick(&self, secs: f64) -> u64 {
        if secs <= 0.0 {
            return 0;
        }

        // Find the tempo entry that covers this time
        let idx = match self
            .entries
            .binary_search_by(|e| e.time_secs.partial_cmp(&secs).unwrap())
        {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };

        let entry = &self.entries[idx];
        let remaining_secs = secs - entry.time_secs;
        let tpq = self.ticks_per_quarter as f64;
        let secs_per_tick = entry.us_per_quarter as f64 / 1_000_000.0 / tpq;
        let delta_ticks = (remaining_secs / secs_per_tick) as u64;
        entry.tick + delta_ticks
    }

    /// Get the BPM at a given tick.
    #[allow(dead_code)]
    pub fn bpm_at_tick(&self, tick: u64) -> f64 {
        let idx = match self.entries.binary_search_by_key(&tick, |e| e.tick) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let us_per_q = self.entries[idx].us_per_quarter as f64;
        60_000_000.0 / us_per_q
    }

    /// Total duration in seconds for a given total tick count.
    pub fn total_duration_secs(&self, total_ticks: u64) -> f64 {
        self.tick_to_secs(total_ticks)
    }
}
