//! Mirrors sequencer events into `SharedState` for the UI thread.

use std::sync::atomic::Ordering;

use ump_playback::midi::event::{MidiEvent, TimedMidiEvent};
use ump_playback::midi::sysex::{parse_sysex, SysExCommand};
use ump_playback::sequencer::EventSink;

use super::SharedState;

pub struct SharedStateSink<'a>(pub &'a SharedState);

impl EventSink for SharedStateSink<'_> {
    fn on_event(&mut self, evt: &TimedMidiEvent) {
        let shared = self.0;
        match &evt.event {
            MidiEvent::NoteOn { port, channel, vel, .. } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                shared.channel_states.velocity[flat_ch].store(*vel as u32, Ordering::Relaxed);
            }
            MidiEvent::NoteOff { .. } => {}
            MidiEvent::ProgramChange { port, channel, program } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                shared.channel_states.program[flat_ch].store(*program as u32, Ordering::Relaxed);
            }
            MidiEvent::ControlChange { port, channel, controller, value } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                let cs = &shared.channel_states;
                let v = *value as u32;
                match *controller {
                    0 => cs.bank[flat_ch].store(v, Ordering::Relaxed),
                    1 => cs.modulation[flat_ch].store(v, Ordering::Relaxed),
                    7 => cs.volume[flat_ch].store(v, Ordering::Relaxed),
                    10 => cs.pan[flat_ch].store(v, Ordering::Relaxed),
                    11 => cs.expression[flat_ch].store(v, Ordering::Relaxed),
                    64 => cs.pedal[flat_ch].store(v, Ordering::Relaxed),
                    91 => cs.reverb[flat_ch].store(v, Ordering::Relaxed),
                    93 => cs.chorus[flat_ch].store(v, Ordering::Relaxed),
                    _ => {}
                }
            }
            MidiEvent::PitchBend { port, channel, value } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                shared.channel_states.pitch_bend[flat_ch].store(*value as i32 as u32, Ordering::Relaxed);
            }
            MidiEvent::TempoChange(us_per_q) => {
                let bpm_x100 = (60_000_000.0 / *us_per_q as f64 * 100.0) as u32;
                shared.current_bpm_x100.store(bpm_x100, Ordering::Relaxed);
            }
            MidiEvent::TimeSignature { numerator, denominator } => {
                shared.time_sig_num.store(*numerator as u32, Ordering::Relaxed);
                shared.time_sig_den.store(*denominator as u32, Ordering::Relaxed);
            }
            MidiEvent::PolyAftertouch { .. } => {}
            MidiEvent::ChannelAftertouch { port, channel, pressure } => {
                let flat_ch = *port as usize * 16 + *channel as usize;
                shared.channel_states.aftertouch[flat_ch].store(*pressure as u32, Ordering::Relaxed);
            }
            // Drum channels are read back from the synthesizer, so only the
            // controller display needs resetting here.
            MidiEvent::SysEx(data) => {
                if let Some(SysExCommand::SystemReset(_)) = parse_sysex(data) {
                    shared.channel_states.reset();
                }
            }
        }
    }

    fn on_seek_reset(&mut self) {
        self.0.channel_states.reset();
    }
}
