//! MIDI sequencer: schedules events against the audio clock.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::midi::event::{MidiData, MidiEvent, TimedMidiEvent};
use crate::midi::sysex::{parse_sysex, SysExCommand};
use crate::midi::tempo_map::TempoMap;
use crate::state::SharedState;
use crate::synth::engine::SynthPool;

#[allow(dead_code)]
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
}

impl Sequencer {
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
        }
    }

    /// Fill the audio buffer with rendered samples.
    /// Called from the cpal audio callback.
    pub fn fill_buffer(
        &mut self,
        synth: &mut SynthPool,
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

        // Sync per-port mute masks to synth engines
        for p in 0..synth.port_count() as u64 {
            let port_mask = ((muted >> (p * 16)) & 0xFFFF) as u16;
            synth.set_channel_mute_mask_for_port(p as u8, port_mask);
        }

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
        synth: &mut SynthPool,
        muted: u64,
        shared: &Arc<SharedState>,
    ) {
        match &evt.event {
            MidiEvent::NoteOn { port, channel, key, vel } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                if muted & (1u64 << flat_ch) == 0 {
                    synth.note_on(*port, *channel as i32, *key as i32, *vel as i32);
                }
                shared.channel_states.velocity[flat_ch]
                    .store(*vel as u32, Ordering::Relaxed);
                // Update monitor state
                let prev_tick = shared.monitor.note_tick[flat_ch].load(Ordering::Relaxed);
                let st = if prev_tick > 0 { evt.tick.saturating_sub(prev_tick as u64) as u32 } else { 0 };
                shared.monitor.note_key[flat_ch].store(*key as u32, Ordering::Relaxed);
                shared.monitor.note_vel[flat_ch].store(*vel as u32, Ordering::Relaxed);
                shared.monitor.note_tick[flat_ch].store(evt.tick as u32, Ordering::Relaxed);
                shared.monitor.step_time[flat_ch].store(st, Ordering::Relaxed);
            }
            MidiEvent::NoteOff { port, channel, key } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                synth.note_off(*port, *channel as i32, *key as i32);
                // Update gate time: duration from NoteOn to NoteOff
                let on_tick = shared.monitor.note_tick[flat_ch].load(Ordering::Relaxed);
                if on_tick > 0 {
                    let gt = evt.tick.saturating_sub(on_tick as u64) as u32;
                    shared.monitor.gate_time[flat_ch].store(gt, Ordering::Relaxed);
                }
            }
            MidiEvent::ProgramChange { port, channel, program } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                synth.program_change(*port, *channel as i32, *program as i32);
                shared.channel_states.program[flat_ch]
                    .store(*program as u32, Ordering::Relaxed);
            }
            MidiEvent::ControlChange {
                port,
                channel,
                controller,
                value,
            } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                synth.control_change(*port, *channel as i32, *controller as i32, *value as i32);
                let v = *value as u32;
                match *controller {
                    0 => shared.channel_states.bank[flat_ch].store(v, Ordering::Relaxed),
                    1 => shared.channel_states.modulation[flat_ch].store(v, Ordering::Relaxed),
                    7 => shared.channel_states.volume[flat_ch].store(v, Ordering::Relaxed),
                    10 => shared.channel_states.pan[flat_ch].store(v, Ordering::Relaxed),
                    11 => shared.channel_states.expression[flat_ch].store(v, Ordering::Relaxed),
                    64 => shared.channel_states.pedal[flat_ch].store(v, Ordering::Relaxed),
                    91 => shared.channel_states.reverb[flat_ch].store(v, Ordering::Relaxed),
                    93 => shared.channel_states.chorus[flat_ch].store(v, Ordering::Relaxed),
                    _ => {}
                }
            }
            MidiEvent::PitchBend { port, channel, value } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                synth.pitch_bend(*port, *channel as i32, *value);
                // pitch_bend value from midly is i16 mapped to 0-16383 range
                shared.channel_states.pitch_bend[flat_ch]
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
                port,
                channel,
                key,
                pressure,
            } => {
                synth.poly_aftertouch(*port, *channel as i32, *key as i32, *pressure as i32);
            }
            MidiEvent::ChannelAftertouch { port, channel, pressure } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                synth.channel_aftertouch(*port, *channel as i32, *pressure as i32);
                shared.channel_states.aftertouch[flat_ch]
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
                            let pc = shared.port_count.load(Ordering::Relaxed) as u8;
                            shared.init_drum_channels(pc);
                            shared.master_volume.store(127, Ordering::Relaxed);
                        }
                        SysExCommand::GsDrumMap { channel, is_drum } => {
                            // SysEx has no port context — apply to port 0
                            synth.set_percussion_channel(0, channel as usize, is_drum);
                            synth.control_change(0, channel as i32, 0, 0);
                            synth.program_change(0, channel as i32, 0);
                            let flat_ch = channel as u64;
                            if is_drum {
                                let _ = shared
                                    .drum_channels
                                    .fetch_or(1u64 << flat_ch, Ordering::Relaxed);
                            } else {
                                let _ = shared
                                    .drum_channels
                                    .fetch_and(!(1u64 << flat_ch), Ordering::Relaxed);
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
        synth: &mut SynthPool,
        shared: &Arc<SharedState>,
    ) {
        // Reset synthesizer
        synth.reset();
        let pc = synth.port_count();
        for p in 0..pc {
            for ch in 0..16usize {
                synth.set_percussion_channel(p, ch, ch == 9);
            }
        }

        // Reset sequencer state
        self.event_index = 0;
        self.us_per_quarter = 500_000;

        // Reset channel states
        shared.channel_states.reset();
        shared.init_drum_channels(pc);
        shared.monitor.reset();

        // Replay all state-setting events (program changes, control changes, tempo)
        // up to target_tick without sounding notes
        while self.event_index < self.events.len() {
            let evt = &self.events[self.event_index];
            if evt.tick > target_tick {
                break;
            }

            match &evt.event {
                MidiEvent::ProgramChange { port, channel, program } => {
                    let flat_ch = *port as usize * 16 + *channel as usize;
                    synth.program_change(*port, *channel as i32, *program as i32);
                    shared.channel_states.program[flat_ch]
                        .store(*program as u32, Ordering::Relaxed);
                }
                MidiEvent::ControlChange {
                    port,
                    channel,
                    controller,
                    value,
                } => {
                    let flat_ch = *port as usize * 16 + *channel as usize;
                    synth.control_change(*port, *channel as i32, *controller as i32, *value as i32);
                    let v = *value as u32;
                    match *controller {
                        0 => shared.channel_states.bank[flat_ch].store(v, Ordering::Relaxed),
                        1 => shared.channel_states.modulation[flat_ch].store(v, Ordering::Relaxed),
                        7 => shared.channel_states.volume[flat_ch].store(v, Ordering::Relaxed),
                        10 => shared.channel_states.pan[flat_ch].store(v, Ordering::Relaxed),
                        11 => shared.channel_states.expression[flat_ch].store(v, Ordering::Relaxed),
                        64 => shared.channel_states.pedal[flat_ch].store(v, Ordering::Relaxed),
                        91 => shared.channel_states.reverb[flat_ch].store(v, Ordering::Relaxed),
                        93 => shared.channel_states.chorus[flat_ch].store(v, Ordering::Relaxed),
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
                MidiEvent::PitchBend { port, channel, value } => {
                    let flat_ch = *port as usize * 16 + *channel as usize;
                    shared.channel_states.pitch_bend[flat_ch]
                        .store(*value as u32, Ordering::Relaxed);
                }
                MidiEvent::ChannelAftertouch { port, channel, pressure } => {
                    let flat_ch = *port as usize * 16 + *channel as usize;
                    synth.channel_aftertouch(*port, *channel as i32, *pressure as i32);
                    shared.channel_states.aftertouch[flat_ch]
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
                                shared.init_drum_channels(pc);
                                shared.master_volume.store(127, Ordering::Relaxed);
                            }
                            SysExCommand::GsDrumMap { channel, is_drum } => {
                                synth.set_percussion_channel(0, channel as usize, is_drum);
                                synth.control_change(0, channel as i32, 0, 0);
                                synth.program_change(0, channel as i32, 0);
                                let flat_ch = channel as u64;
                                if is_drum {
                                    let _ = shared
                                        .drum_channels
                                        .fetch_or(1u64 << flat_ch, Ordering::Relaxed);
                                } else {
                                    let _ = shared
                                        .drum_channels
                                        .fetch_and(!(1u64 << flat_ch), Ordering::Relaxed);
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

        // Re-sync mute mask after state rebuild (synth.reset() may clear it)
        let muted = shared.muted_channels.load(Ordering::Relaxed);
        for p in 0..pc as u64 {
            let port_mask = ((muted >> (p * 16)) & 0xFFFF) as u16;
            synth.set_channel_mute_mask_for_port(p as u8, port_mask);
        }

        // Set time position
        self.current_tick = target_tick;
        self.current_time_secs = self.tempo_map.tick_to_secs(target_tick);

        // Update shared state
        shared.current_tick.store(target_tick, Ordering::Relaxed);
        shared.finished.store(false, Ordering::Relaxed);
    }
}
