//! MIDI sequencer: schedules events against the audio clock.

use crate::midi::event::{MidiData, MidiEvent, TimedMidiEvent};
use crate::midi::tempo_map::TempoMap;
use crate::synth::engine::SynthPool;

/// Samples rendered between event dispatches; bounds timing error to
/// ~1.5 ms at 44.1 kHz regardless of the host's buffer size.
const CHUNK_SIZE: usize = 64;

/// Receives events as the sequencer applies them to the synth, so hosts can
/// mirror channel state for display without the sequencer knowing about it.
pub trait EventSink {
    /// Called for every event applied to the synth, including events
    /// replayed during a seek. NoteOn is reported even on muted channels.
    fn on_event(&mut self, event: &TimedMidiEvent);

    /// Called when a seek resets the synth, before replayed events are
    /// reported. Hosts should restore their channel state to defaults here.
    fn on_seek_reset(&mut self);
}

/// Discards all notifications.
impl EventSink for () {
    fn on_event(&mut self, _event: &TimedMidiEvent) {}
    fn on_seek_reset(&mut self) {}
}

/// Collects events for hosts that forward them elsewhere, such as a log view.
impl EventSink for Vec<TimedMidiEvent> {
    fn on_event(&mut self, event: &TimedMidiEvent) {
        self.push(event.clone());
    }
    fn on_seek_reset(&mut self) {}
}

/// Plays a parsed MIDI file through a [`SynthPool`].
///
/// Holds no transport state: every [`fill_buffer`](Self::fill_buffer) call
/// advances playback, and the host decides when to call it.
pub struct Sequencer {
    events: Vec<TimedMidiEvent>,
    tempo_map: TempoMap,
    ticks_per_quarter: u16,
    total_ticks: u64,
    port_count: u8,

    /// Current position index into events list.
    event_index: usize,
    /// Current time in seconds (audio clock).
    current_time_secs: f64,
    /// Current tick (derived from current_time_secs).
    current_tick: u64,
    /// Current microseconds per quarter note.
    us_per_quarter: u32,
    /// Muted flat channels (bit N = port * 16 + channel).
    muted: u64,
}

impl Sequencer {
    /// Create a sequencer positioned at the start of `midi_data`.
    pub fn new(midi_data: &MidiData, tempo_map: TempoMap) -> Self {
        Sequencer {
            events: midi_data.events.clone(),
            tempo_map,
            ticks_per_quarter: midi_data.ticks_per_quarter,
            total_ticks: midi_data.total_ticks,
            port_count: midi_data.port_count,
            event_index: 0,
            current_time_secs: 0.0,
            current_tick: 0,
            us_per_quarter: 500_000, // default 120 BPM
            muted: 0,
        }
    }

    /// Create an empty sequencer (no events, used as placeholder).
    pub fn new_empty(ticks_per_quarter: u16, tempo_map: TempoMap) -> Self {
        Sequencer {
            events: Vec::new(),
            tempo_map,
            ticks_per_quarter,
            total_ticks: 0,
            port_count: 1,
            event_index: 0,
            current_time_secs: 0.0,
            current_tick: 0,
            us_per_quarter: 500_000,
            muted: 0,
        }
    }

    /// Set muted flat channels (bit N = port * 16 + channel). Takes effect on
    /// the next [`fill_buffer`](Self::fill_buffer) or seek.
    pub fn set_muted_channels(&mut self, mask: u64) {
        self.muted = mask;
    }

    /// Current tick position.
    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// Current position in seconds.
    pub fn current_time_secs(&self) -> f64 {
        self.current_time_secs
    }

    /// Length of the file in ticks.
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks
    }

    /// Length of the file in seconds.
    pub fn total_duration_secs(&self) -> f64 {
        self.tempo_map.total_duration_secs(self.total_ticks)
    }

    /// Number of MIDI ports used by the file.
    pub fn port_count(&self) -> u8 {
        self.port_count
    }

    /// Ticks per quarter note from the file header.
    pub fn ticks_per_quarter(&self) -> u16 {
        self.ticks_per_quarter
    }

    /// Tempo at the current position.
    pub fn current_bpm(&self) -> f64 {
        60_000_000.0 / self.us_per_quarter as f64
    }

    /// Whether all events have been dispatched and the end tick reached.
    pub fn is_finished(&self) -> bool {
        self.event_index >= self.events.len() && self.current_tick >= self.total_ticks
    }

    /// Render `left.len()` samples, dispatching events that fall within them.
    /// `left` and `right` must have the same length.
    pub fn fill_buffer(
        &mut self,
        synth: &mut SynthPool,
        left: &mut [f32],
        right: &mut [f32],
        sink: &mut impl EventSink,
    ) {
        let sample_rate = synth.sample_rate() as f64;
        let buf_len = left.len();

        self.sync_mute_masks(synth);

        let mut offset = 0;
        while offset < buf_len {
            let this_chunk = (buf_len - offset).min(CHUNK_SIZE);

            let new_time = self.current_time_secs + this_chunk as f64 / sample_rate;
            let new_tick = self.tempo_map.secs_to_tick(new_time);

            while let Some(evt) = self.events.get(self.event_index) {
                if evt.tick > new_tick {
                    break;
                }
                Self::apply_event(evt, &mut self.us_per_quarter, synth, self.muted);
                sink.on_event(evt);
                self.event_index += 1;
            }

            synth.render(
                &mut left[offset..offset + this_chunk],
                &mut right[offset..offset + this_chunk],
            );

            self.current_time_secs = new_time;
            self.current_tick = new_tick;
            offset += this_chunk;
        }
    }

    /// Jump to `target_tick`, rebuilding synth state by replaying every
    /// state-setting event before it. Notes are not replayed, so the seek is
    /// silent.
    pub fn seek_to_tick(
        &mut self,
        target_tick: u64,
        synth: &mut SynthPool,
        sink: &mut impl EventSink,
    ) {
        // Back to the GM baseline: the replay below re-applies whatever the file
        // declares, including the system mode and any drum map it moves.
        synth.system_reset();
        sink.on_seek_reset();

        self.event_index = 0;
        self.us_per_quarter = 500_000;

        while let Some(evt) = self.events.get(self.event_index) {
            if evt.tick > target_tick {
                break;
            }
            let replay = !matches!(
                evt.event,
                MidiEvent::NoteOn { .. } | MidiEvent::NoteOff { .. } | MidiEvent::PolyAftertouch { .. }
            );
            if replay {
                Self::apply_event(evt, &mut self.us_per_quarter, synth, self.muted);
                sink.on_event(evt);
            }
            self.event_index += 1;
        }

        // The reset above may clear the mute masks.
        self.sync_mute_masks(synth);

        self.current_tick = target_tick;
        self.current_time_secs = self.tempo_map.tick_to_secs(target_tick);
    }

    fn sync_mute_masks(&self, synth: &mut SynthPool) {
        for p in 0..synth.port_count() as u64 {
            let port_mask = ((self.muted >> (p * 16)) & 0xFFFF) as u16;
            synth.set_channel_mute_mask_for_port(p as u8, port_mask);
        }
    }

    // Associated fn rather than a method so callers can hold a borrow of
    // `self.events` while mutating other fields.
    fn apply_event(evt: &TimedMidiEvent, us_per_quarter: &mut u32, synth: &mut SynthPool, muted: u64) {
        match &evt.event {
            MidiEvent::NoteOn { port, channel, key, vel } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                if muted & (1u64 << flat_ch) == 0 {
                    synth.note_on(*port, *channel as i32, *key as i32, *vel as i32);
                }
            }
            MidiEvent::NoteOff { port, channel, key } => {
                synth.note_off(*port, *channel as i32, *key as i32);
            }
            MidiEvent::ProgramChange { port, channel, program } => {
                synth.program_change(*port, *channel as i32, *program as i32);
            }
            MidiEvent::ControlChange { port, channel, controller, value } => {
                synth.control_change(*port, *channel as i32, *controller as i32, *value as i32);
            }
            MidiEvent::PitchBend { port, channel, value } => {
                synth.pitch_bend(*port, *channel as i32, *value);
            }
            MidiEvent::TempoChange(us_per_q) => {
                *us_per_quarter = *us_per_q;
            }
            MidiEvent::TimeSignature { .. } => {}
            MidiEvent::PolyAftertouch { port, channel, key, pressure } => {
                synth.poly_aftertouch(*port, *channel as i32, *key as i32, *pressure as i32);
            }
            MidiEvent::ChannelAftertouch { port, channel, pressure } => {
                synth.channel_aftertouch(*port, *channel as i32, *pressure as i32);
            }
            MidiEvent::SysEx(data) => {
                // rustysynth applies the whole recognized set itself: resets and the
                // system mode they select, master volume and tuning, GS and XG part
                // parameters, and the drum map switches. Replaying any of them here
                // would undo the mode-dependent part, notably an XG reset that leaves
                // channel 10 on the drum bank.
                synth.process_sysex(data);
            }
        }
    }
}
