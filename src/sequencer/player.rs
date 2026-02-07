//! MIDI sequencer: schedules events against the audio clock.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::midi::event::{MidiData, MidiEvent, TimedMidiEvent};
use crate::midi::sysex::{parse_sysex, SysExCommand};
use crate::midi::tempo_map::TempoMap;
use crate::state::SharedState;
use crate::synth::engine::SynthEngine;

#[allow(dead_code)]
pub struct Sequencer {
    events: Vec<TimedMidiEvent>,
    tempo_map: TempoMap,
    ticks_per_quarter: u16,
    total_ticks: u64,

    /// Current position index into events list.
    event_index: usize,
    /// Current time in seconds (audio clock).
    current_time_secs: f64,
    /// Current tick (derived from current_time_secs).
    current_tick: u64,
    /// Current microseconds per quarter note.
    us_per_quarter: u32,
}

impl Sequencer {
    pub fn new(midi_data: &MidiData, tempo_map: TempoMap) -> Self {
        Sequencer {
            events: midi_data.events.clone(),
            tempo_map,
            ticks_per_quarter: midi_data.ticks_per_quarter,
            total_ticks: midi_data.total_ticks,
            event_index: 0,
            current_time_secs: 0.0,
            current_tick: 0,
            us_per_quarter: 500_000, // default 120 BPM
        }
    }

    /// Create an empty sequencer (no events, used as placeholder).
    pub fn new_empty(ticks_per_quarter: u16, tempo_map: TempoMap) -> Self {
        Sequencer {
            events: Vec::new(),
            tempo_map,
            ticks_per_quarter,
            total_ticks: 0,
            event_index: 0,
            current_time_secs: 0.0,
            current_tick: 0,
            us_per_quarter: 500_000,
        }
    }

    /// Fill the audio buffer with rendered samples.
    /// Called from the cpal audio callback.
    pub fn fill_buffer(
        &mut self,
        synth: &mut SynthEngine,
        left: &mut [f32],
        right: &mut [f32],
        shared: &Arc<SharedState>,
    ) {
        let sample_rate = synth.sample_rate() as f64;
        let buf_len = left.len();

        // Handle seek request
        let seek_raw = shared.seek_tick.swap(0, Ordering::Relaxed);
        if seek_raw > 0 {
            let target_tick = if seek_raw == 1 { 0 } else { seek_raw - 1 };
            self.seek_to_tick(target_tick, synth, shared);
        }

        // Handle stop
        if shared.stopped.load(Ordering::Relaxed) {
            left.fill(0.0);
            right.fill(0.0);
            return;
        }

        // If not playing or finished, output silence
        if !shared.is_playing() || shared.finished.load(Ordering::Relaxed) {
            left.fill(0.0);
            right.fill(0.0);
            return;
        }

        let volume = shared.get_volume_f32();
        let muted = shared.muted_channels.load(Ordering::Relaxed);

        // Process events and render in small chunks to maintain timing accuracy
        let chunk_size = 64;
        let mut offset = 0;

        while offset < buf_len {
            let remaining = buf_len - offset;
            let this_chunk = remaining.min(chunk_size);

            // Advance time by this_chunk samples
            let time_advance = this_chunk as f64 / sample_rate;
            let new_time = self.current_time_secs + time_advance;

            // Convert new_time to tick
            let new_tick = self.tempo_map.secs_to_tick(new_time);

            // Process all events up to new_tick
            while self.event_index < self.events.len() {
                if self.events[self.event_index].tick > new_tick {
                    break;
                }

                let evt = self.events[self.event_index].clone();
                self.event_index += 1;

                // Process this event
                Self::dispatch_event(&evt, &mut self.us_per_quarter, synth, muted, shared);
            }

            // Render audio for this chunk
            synth.render(
                &mut left[offset..offset + this_chunk],
                &mut right[offset..offset + this_chunk],
            );

            // Apply volume
            for i in offset..offset + this_chunk {
                left[i] *= volume;
                right[i] *= volume;
            }

            self.current_time_secs = new_time;
            self.current_tick = new_tick;

            offset += this_chunk;
        }

        // Update shared state
        shared.current_tick.store(self.current_tick, Ordering::Relaxed);

        // Check if finished
        if self.event_index >= self.events.len() && self.current_tick >= self.total_ticks {
            shared.finished.store(true, Ordering::Relaxed);
            shared.playing.store(false, Ordering::Relaxed);
        }
    }

    fn dispatch_event(
        evt: &TimedMidiEvent,
        us_per_quarter: &mut u32,
        synth: &mut SynthEngine,
        muted: u32,
        shared: &Arc<SharedState>,
    ) {
        match &evt.event {
            MidiEvent::NoteOn { channel, key, vel } => {
                if muted & (1 << *channel) == 0 {
                    synth.note_on(*channel as i32, *key as i32, *vel as i32);
                }
                shared.channel_states.velocity[*channel as usize]
                    .store(*vel as u32, Ordering::Relaxed);
            }
            MidiEvent::NoteOff { channel, key } => {
                synth.note_off(*channel as i32, *key as i32);
            }
            MidiEvent::ProgramChange { channel, program } => {
                synth.program_change(*channel as i32, *program as i32);
                shared.channel_states.program[*channel as usize]
                    .store(*program as u32, Ordering::Relaxed);
            }
            MidiEvent::ControlChange {
                channel,
                controller,
                value,
            } => {
                synth.control_change(*channel as i32, *controller as i32, *value as i32);
                let ch = *channel as usize;
                let v = *value as u32;
                match *controller {
                    1 => shared.channel_states.modulation[ch].store(v, Ordering::Relaxed),
                    7 => shared.channel_states.volume[ch].store(v, Ordering::Relaxed),
                    10 => shared.channel_states.pan[ch].store(v, Ordering::Relaxed),
                    11 => shared.channel_states.expression[ch].store(v, Ordering::Relaxed),
                    64 => shared.channel_states.pedal[ch].store(v, Ordering::Relaxed),
                    _ => {}
                }
            }
            MidiEvent::PitchBend { channel, value } => {
                synth.pitch_bend(*channel as i32, *value);
                // pitch_bend value from midly is i16 mapped to 0-16383 range
                shared.channel_states.pitch_bend[*channel as usize]
                    .store(*value as i32 as u32, Ordering::Relaxed);
            }
            MidiEvent::TempoChange(us_per_q) => {
                *us_per_quarter = *us_per_q;
                let bpm_x100 = (60_000_000.0 / *us_per_q as f64 * 100.0) as u32;
                shared.current_bpm_x100.store(bpm_x100, Ordering::Relaxed);
            }
            MidiEvent::TimeSignature {
                numerator,
                denominator,
            } => {
                shared
                    .time_sig_num
                    .store(*numerator as u32, Ordering::Relaxed);
                shared
                    .time_sig_den
                    .store(*denominator as u32, Ordering::Relaxed);
            }
            MidiEvent::PolyAftertouch {
                channel,
                key,
                pressure,
            } => {
                synth.poly_aftertouch(*channel as i32, *key as i32, *pressure as i32);
            }
            MidiEvent::ChannelAftertouch { channel, pressure } => {
                synth.channel_aftertouch(*channel as i32, *pressure as i32);
                shared.channel_states.aftertouch[*channel as usize]
                    .store(*pressure as u32, Ordering::Relaxed);
            }
            MidiEvent::SysEx(data) => {
                // Forward to rustysynth for master tune, scale tuning, etc.
                synth.process_sysex(data);

                if let Some(cmd) = parse_sysex(data) {
                    match cmd {
                        SysExCommand::SystemReset(_) => {
                            synth.system_reset();
                            shared.channel_states.reset();
                            shared.drum_channels.store(1 << 9, Ordering::Relaxed);
                            shared.master_volume.store(127, Ordering::Relaxed);
                        }
                        SysExCommand::GsDrumMap { channel, is_drum } => {
                            let ch = channel as i32;
                            let bank = if is_drum { 128 } else { 0 };
                            synth.control_change(ch, 0, bank);
                            synth.program_change(ch, 0);
                            if is_drum {
                                let _ = shared
                                    .drum_channels
                                    .fetch_or(1 << channel, Ordering::Relaxed);
                            } else {
                                let _ = shared
                                    .drum_channels
                                    .fetch_and(!(1 << channel), Ordering::Relaxed);
                            }
                        }
                        SysExCommand::MasterVolume(msb) => {
                            shared.master_volume.store(msb as u32, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }

    /// Seek to a specific tick position.
    fn seek_to_tick(
        &mut self,
        target_tick: u64,
        synth: &mut SynthEngine,
        shared: &Arc<SharedState>,
    ) {
        // Reset synthesizer
        synth.reset();

        // Reset sequencer state
        self.event_index = 0;
        self.us_per_quarter = 500_000;

        // Reset channel states
        shared.channel_states.reset();
        shared.drum_channels.store(1 << 9, Ordering::Relaxed);

        // Replay all state-setting events (program changes, control changes, tempo)
        // up to target_tick without sounding notes
        while self.event_index < self.events.len() {
            let evt = &self.events[self.event_index];
            if evt.tick > target_tick {
                break;
            }

            match &evt.event {
                MidiEvent::ProgramChange { channel, program } => {
                    synth.program_change(*channel as i32, *program as i32);
                    shared.channel_states.program[*channel as usize]
                        .store(*program as u32, Ordering::Relaxed);
                }
                MidiEvent::ControlChange {
                    channel,
                    controller,
                    value,
                } => {
                    synth.control_change(*channel as i32, *controller as i32, *value as i32);
                    let ch = *channel as usize;
                    let v = *value as u32;
                    match *controller {
                        1 => shared.channel_states.modulation[ch].store(v, Ordering::Relaxed),
                        7 => shared.channel_states.volume[ch].store(v, Ordering::Relaxed),
                        10 => shared.channel_states.pan[ch].store(v, Ordering::Relaxed),
                        11 => shared.channel_states.expression[ch].store(v, Ordering::Relaxed),
                        64 => shared.channel_states.pedal[ch].store(v, Ordering::Relaxed),
                        _ => {}
                    }
                }
                MidiEvent::TempoChange(us_per_q) => {
                    self.us_per_quarter = *us_per_q;
                    let bpm_x100 = (60_000_000.0 / *us_per_q as f64 * 100.0) as u32;
                    shared.current_bpm_x100.store(bpm_x100, Ordering::Relaxed);
                }
                MidiEvent::TimeSignature {
                    numerator,
                    denominator,
                } => {
                    shared
                        .time_sig_num
                        .store(*numerator as u32, Ordering::Relaxed);
                    shared
                        .time_sig_den
                        .store(*denominator as u32, Ordering::Relaxed);
                }
                MidiEvent::PitchBend { channel, value } => {
                    shared.channel_states.pitch_bend[*channel as usize]
                        .store(*value as u32, Ordering::Relaxed);
                }
                MidiEvent::ChannelAftertouch { channel, pressure } => {
                    synth.channel_aftertouch(*channel as i32, *pressure as i32);
                    shared.channel_states.aftertouch[*channel as usize]
                        .store(*pressure as u32, Ordering::Relaxed);
                }
                MidiEvent::SysEx(data) => {
                    // Forward to rustysynth for master tune, scale tuning, etc.
                    synth.process_sysex(data);

                    if let Some(cmd) = parse_sysex(data) {
                        match cmd {
                            SysExCommand::SystemReset(_) => {
                                synth.system_reset();
                                shared.channel_states.reset();
                                shared.drum_channels.store(1 << 9, Ordering::Relaxed);
                                shared.master_volume.store(127, Ordering::Relaxed);
                            }
                            SysExCommand::GsDrumMap { channel, is_drum } => {
                                let ch = channel as i32;
                                let bank = if is_drum { 128 } else { 0 };
                                synth.control_change(ch, 0, bank);
                                synth.program_change(ch, 0);
                                if is_drum {
                                    let _ = shared
                                        .drum_channels
                                        .fetch_or(1 << channel, Ordering::Relaxed);
                                } else {
                                    let _ = shared
                                        .drum_channels
                                        .fetch_and(!(1 << channel), Ordering::Relaxed);
                                }
                            }
                            SysExCommand::MasterVolume(msb) => {
                                shared.master_volume.store(msb as u32, Ordering::Relaxed);
                            }
                        }
                    }
                }
                // Skip note events during seek
                _ => {}
            }

            self.event_index += 1;
        }

        // Set time position
        self.current_tick = target_tick;
        self.current_time_secs = self.tempo_map.tick_to_secs(target_tick);

        // Update shared state
        shared.current_tick.store(target_tick, Ordering::Relaxed);
        shared.finished.store(false, Ordering::Relaxed);
    }
}
