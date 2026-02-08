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
            SoundFont::new(&mut cursor).context("Failed to load SoundFont")?;

        for warn in sound_font.get_warnings() {
            log_warn!("SF2 sanitize: {}", warn);
        }

        let sound_font = Arc::new(sound_font);
        Self::new_from_soundfont(sound_font, sample_rate)
    }

    /// Create a SynthEngine sharing an already-parsed SoundFont.
    pub fn new_from_soundfont(sound_font: Arc<SoundFont>, sample_rate: u32) -> Result<Self> {
        let mut settings = SynthesizerSettings::new(sample_rate as i32);
        settings.enable_reverb_and_chorus = true;

        let synth =
            Synthesizer::new(&sound_font, &settings).context("Failed to create synthesizer")?;

        Ok(SynthEngine { synth, sample_rate })
    }

    /// Parse SF2 data and return Arc<SoundFont> for sharing across engines.
    pub fn parse_soundfont(sf2_data: &[u8]) -> Result<Arc<SoundFont>> {
        let mut cursor = std::io::Cursor::new(sf2_data);
        let sound_font =
            SoundFont::new(&mut cursor).context("Failed to load SoundFont")?;

        for warn in sound_font.get_warnings() {
            log_warn!("SF2 sanitize: {}", warn);
        }

        Ok(Arc::new(sound_font))
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
            self.synth
                .set_percussion_channel(ch as usize, ch == 9);
            self.synth.process_midi_message(ch, 0xB0, 120, 0);
            self.synth.process_midi_message(ch, 0xB0, 121, 0);
            self.synth.process_midi_message(ch, 0xB0, 123, 0);
            // Bank 0 — set_bank auto-applies +128 offset for percussion channels
            self.synth.process_midi_message(ch, 0xB0, 0, 0);
            self.synth.process_midi_message(ch, 0xC0, 0, 0);
        }
    }

    /// Set whether a channel is a percussion channel.
    pub fn set_percussion_channel(&mut self, channel: usize, is_percussion: bool) {
        self.synth.set_percussion_channel(channel, is_percussion);
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

/// Pool of multiple SynthEngines with per-port, per-channel routing.
/// Each port maps its 16 channels to one engine via the routing table.
pub struct SynthPool {
    engines: Vec<SynthEngine>,
    /// port_routing[port][channel] = engine index in `engines`
    port_routing: Vec<[usize; 16]>,
    port_count: u8,
}

impl SynthPool {
    /// Create a pool from multiple SF2 data blobs with per-channel routing (single port).
    /// For backward compatibility with bundle mode.
    pub fn new(sf2_data_list: &[&[u8]], routing: [usize; 16], sample_rate: u32) -> Result<Self> {
        if sf2_data_list.is_empty() {
            anyhow::bail!("SynthPool requires at least one SF2 file");
        }
        let mut engines = Vec::with_capacity(sf2_data_list.len());
        for (i, data) in sf2_data_list.iter().enumerate() {
            engines.push(
                SynthEngine::new(data, sample_rate)
                    .with_context(|| format!("Failed to create engine {}", i))?,
            );
        }
        Ok(SynthPool {
            engines,
            port_routing: vec![routing],
            port_count: 1,
        })
    }

    /// Create a pool with a single SF2, supporting multiple ports.
    /// Each port gets its own SynthEngine sharing the same parsed SoundFont.
    pub fn single(sf2_data: &[u8], sample_rate: u32, port_count: u8) -> Result<Self> {
        let port_count = port_count.max(1).min(4);
        let sf = SynthEngine::parse_soundfont(sf2_data)?;

        let mut engines = Vec::with_capacity(port_count as usize);
        for i in 0..port_count {
            engines.push(
                SynthEngine::new_from_soundfont(sf.clone(), sample_rate)
                    .with_context(|| format!("Failed to create engine for port {}", i))?,
            );
        }

        let mut port_routing = Vec::with_capacity(port_count as usize);
        for p in 0..port_count as usize {
            // Each port routes all 16 channels to its own engine
            port_routing.push([p; 16]);
        }

        Ok(SynthPool {
            engines,
            port_routing,
            port_count,
        })
    }

    /// Create a multi-port bundle pool from multiple SF2 data blobs with per-channel routing.
    /// Each port gets the same bundle routing, offset by port index.
    pub fn new_bundle(
        sf2_data_list: &[&[u8]],
        routing: [usize; 16],
        sample_rate: u32,
        port_count: u8,
    ) -> Result<Self> {
        if sf2_data_list.is_empty() {
            anyhow::bail!("SynthPool requires at least one SF2 file");
        }
        let port_count = port_count.max(1).min(4);

        // Parse each SF2 once into Arc<SoundFont>
        let mut soundfonts = Vec::with_capacity(sf2_data_list.len());
        for (i, data) in sf2_data_list.iter().enumerate() {
            soundfonts.push(
                SynthEngine::parse_soundfont(data)
                    .with_context(|| format!("Failed to parse SF2 {}", i))?,
            );
        }

        // Create engines: for each port, create engines matching the bundle
        let engines_per_port = soundfonts.len();
        let mut engines = Vec::with_capacity(engines_per_port * port_count as usize);
        let mut port_routing = Vec::with_capacity(port_count as usize);

        for p in 0..port_count as usize {
            let base_offset = p * engines_per_port;
            for sf in &soundfonts {
                engines.push(
                    SynthEngine::new_from_soundfont(sf.clone(), sample_rate)
                        .with_context(|| format!("Failed to create engine for port {}", p))?,
                );
            }
            let mut pr = [0usize; 16];
            for ch in 0..16 {
                pr[ch] = base_offset + routing[ch];
            }
            port_routing.push(pr);
        }

        Ok(SynthPool {
            engines,
            port_routing,
            port_count,
        })
    }

    pub fn port_count(&self) -> u8 {
        self.port_count
    }

    pub fn sample_rate(&self) -> u32 {
        self.engines[0].sample_rate()
    }

    fn engine_idx(&self, port: u8, channel: u8) -> usize {
        let p = (port as usize).min(self.port_routing.len() - 1);
        self.port_routing[p][channel as usize & 0xF]
    }

    pub fn note_on(&mut self, port: u8, channel: i32, key: i32, velocity: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].note_on(channel, key, velocity);
    }

    pub fn note_off(&mut self, port: u8, channel: i32, key: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].note_off(channel, key);
    }

    pub fn program_change(&mut self, port: u8, channel: i32, program: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].program_change(channel, program);
    }

    pub fn control_change(&mut self, port: u8, channel: i32, controller: i32, value: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].control_change(channel, controller, value);
    }

    pub fn pitch_bend(&mut self, port: u8, channel: i32, value: i16) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].pitch_bend(channel, value);
    }

    pub fn poly_aftertouch(&mut self, port: u8, channel: i32, key: i32, pressure: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].poly_aftertouch(channel, key, pressure);
    }

    pub fn channel_aftertouch(&mut self, port: u8, channel: i32, pressure: i32) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].channel_aftertouch(channel, pressure);
    }

    /// Render audio from all engines and mix into the output buffers.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.engines.len() == 1 {
            // Fast path: single engine, no mixing needed
            self.engines[0].render(left, right);
            return;
        }

        // First engine renders directly into output
        self.engines[0].render(left, right);

        // Additional engines render into temp buffers and accumulate
        let len = left.len();
        let mut tmp_left = vec![0.0f32; len];
        let mut tmp_right = vec![0.0f32; len];
        for engine in &mut self.engines[1..] {
            tmp_left.fill(0.0);
            tmp_right.fill(0.0);
            engine.render(&mut tmp_left, &mut tmp_right);
            for i in 0..len {
                left[i] += tmp_left[i];
                right[i] += tmp_right[i];
            }
        }
    }

    pub fn reset(&mut self) {
        for engine in &mut self.engines {
            engine.reset();
        }
    }

    pub fn process_sysex(&mut self, data: &[u8]) {
        for engine in &mut self.engines {
            engine.process_sysex(data);
        }
    }

    pub fn system_reset(&mut self) {
        for engine in &mut self.engines {
            engine.system_reset();
        }
    }

    pub fn set_percussion_channel(&mut self, port: u8, channel: usize, is_percussion: bool) {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx].set_percussion_channel(channel, is_percussion);
    }

    /// Set per-port channel mute mask (16-bit mask for one port's 16 channels).
    pub fn set_channel_mute_mask_for_port(&mut self, port: u8, mask: u16) {
        let p = (port as usize).min(self.port_routing.len() - 1);
        // Collect unique engine indices for this port
        let mut seen = [false; 64]; // max engines
        for ch in 0..16 {
            let idx = self.port_routing[p][ch];
            if idx < seen.len() && !seen[idx] {
                seen[idx] = true;
                self.engines[idx].set_channel_mute_mask(mask);
            }
        }
    }
}
