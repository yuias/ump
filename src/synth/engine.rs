//! Wrapper around rustysynth for MIDI synthesis.

use anyhow::{Context, Result};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::Arc;

pub struct SynthEngine {
    synth: Synthesizer,
    sample_rate: u32,
}

impl SynthEngine {
    pub fn new(sf2_data: &[u8], sample_rate: u32) -> Result<Self> {
        let mut cursor = std::io::Cursor::new(sf2_data);
        let sound_font =
            Arc::new(SoundFont::new(&mut cursor).context("Failed to load SoundFont")?);

        let mut settings = SynthesizerSettings::new(sample_rate as i32);
        settings.enable_reverb_and_chorus = true;

        let synth =
            Synthesizer::new(&sound_font, &settings).context("Failed to create synthesizer")?;

        Ok(SynthEngine { synth, sample_rate })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn note_on(&mut self, channel: i32, key: i32, velocity: i32) {
        self.synth.note_on(channel, key, velocity);
    }

    pub fn note_off(&mut self, channel: i32, key: i32) {
        self.synth.note_off(channel, key);
    }

    pub fn program_change(&mut self, channel: i32, program: i32) {
        self.synth
            .process_midi_message(channel, 0xC0, program, 0);
    }

    pub fn control_change(&mut self, channel: i32, controller: i32, value: i32) {
        self.synth
            .process_midi_message(channel, 0xB0, controller, value);
    }

    pub fn pitch_bend(&mut self, channel: i32, value: i16) {
        // rustysynth expects pitch bend as two 7-bit values
        let raw = (value as i32 + 8192) as u16;
        let lsb = (raw & 0x7F) as i32;
        let msb = ((raw >> 7) & 0x7F) as i32;
        self.synth.process_midi_message(channel, 0xE0, lsb, msb);
    }

    /// Polyphonic aftertouch (key pressure). 0xA0.
    pub fn poly_aftertouch(&mut self, channel: i32, key: i32, pressure: i32) {
        self.synth
            .process_midi_message(channel, 0xA0, key, pressure);
    }

    /// Channel aftertouch (channel pressure). 0xD0.
    pub fn channel_aftertouch(&mut self, channel: i32, pressure: i32) {
        self.synth
            .process_midi_message(channel, 0xD0, pressure, 0);
    }

    /// Render `sample_count` stereo samples into interleaved output buffer.
    /// Returns the number of samples written (always == sample_count if buffer is large enough).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.synth.render(left, right);
    }

    /// Reset all channels (all notes off, reset controllers).
    pub fn reset(&mut self) {
        for ch in 0..16 {
            // All Sound Off (CC 120)
            self.synth.process_midi_message(ch, 0xB0, 120, 0);
            // Reset All Controllers (CC 121)
            self.synth.process_midi_message(ch, 0xB0, 121, 0);
            // All Notes Off (CC 123)
            self.synth.process_midi_message(ch, 0xB0, 123, 0);
        }
    }

    /// Process a SysEx message (raw data without F0/F7 framing).
    /// Handles master tune, scale tuning, and system resets internally.
    /// Master volume is managed externally by ump, so it is restored after processing.
    pub fn process_sysex(&mut self, data: &[u8]) {
        self.synth.process_sysex(data);
        // ump manages master volume via shared state (app_volume * sysex_volume);
        // restore rustysynth's internal master volume to avoid double-application
        self.synth.set_master_volume(0.5);
    }

    /// Full system reset: silence all channels, reset controllers,
    /// set program 0, and configure bank (drum on Ch9, normal on others).
    pub fn system_reset(&mut self) {
        for ch in 0..16i32 {
            // All Sound Off (CC 120)
            self.synth.process_midi_message(ch, 0xB0, 120, 0);
            // Reset All Controllers (CC 121)
            self.synth.process_midi_message(ch, 0xB0, 121, 0);
            // All Notes Off (CC 123)
            self.synth.process_midi_message(ch, 0xB0, 123, 0);
            // Bank Select MSB (CC 0): 128 for drum (ch9), 0 for normal
            let bank = if ch == 9 { 128 } else { 0 };
            self.synth.process_midi_message(ch, 0xB0, 0, bank);
            // Program Change to 0
            self.synth.process_midi_message(ch, 0xC0, 0, 0);
        }
    }

    /// Sync channel mute mask to the synthesizer.
    /// Each bit in `mask` corresponds to a channel (bit 0 = ch0, bit 15 = ch15).
    /// Muted channels have their voice gain set to 0 for immediate silencing.
    pub fn set_channel_mute_mask(&mut self, mask: u16) {
        self.synth.set_channel_mute_mask(mask);
    }

    /// Get a reference to a channel's state.
    pub fn get_channel(&self, channel: usize) -> Option<&rustysynth::Channel> {
        self.synth.get_channel(channel)
    }
}
