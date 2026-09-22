//! Wrapper around rustysynth for MIDI synthesis.

use anyhow::{Context, Result};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::sync::Arc;

use crate::midi::event::MAX_PORTS;

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
            log::warn!("SF2 sanitize: {}", warn);
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
            log::warn!("SF2 sanitize: {}", warn);
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

    /// Reset all channels: stop every voice and restore controller, tuning and
    /// system-mode defaults. Drum channel assignments and the reverb/chorus
    /// parameters are preserved, so a caller that wants the defaults must
    /// restore them itself.
    pub fn reset(&mut self) {
        self.synth.reset();
    }

    /// Process a SysEx message (raw data without F0/F7 framing).
    ///
    /// rustysynth applies every message it recognizes itself: system resets and
    /// the system mode they select, master volume and tuning, GS and XG part
    /// parameters, and the drum map switches. Callers must not replay those on
    /// top of it, because the synthesizer interprets them per system mode.
    pub fn process_sysex(&mut self, data: &[u8]) {
        self.synth.process_sysex(data);
    }

    /// Full system reset, equivalent to a GM System On: restore the default drum
    /// map and the reverb/chorus defaults on top of [`reset`](Self::reset).
    ///
    /// Only for resets ump initiates itself. A reset arriving as SysEx is already
    /// applied by [`process_sysex`](Self::process_sysex), which also selects the
    /// system mode the message declares.
    pub fn system_reset(&mut self) {
        for ch in 0..16usize {
            self.synth.set_percussion_channel(ch, ch == 9);
        }
        self.synth.reset();
        // Otherwise a GS effect macro from the previous file stays in effect.
        self.synth.reset_effect_parameters();
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
        Self::new_bundle(sf2_data_list, routing, sample_rate, 1)
    }

    /// Create a pool with a single SF2, supporting multiple ports.
    /// Each port gets its own SynthEngine sharing the same parsed SoundFont.
    pub fn single(sf2_data: &[u8], sample_rate: u32, port_count: u8) -> Result<Self> {
        let sf = SynthEngine::parse_soundfont(sf2_data)?;
        Self::from_soundfonts(std::slice::from_ref(&sf), [0; 16], sample_rate, port_count)
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
        // Parse each SF2 once into Arc<SoundFont>
        let mut soundfonts = Vec::with_capacity(sf2_data_list.len());
        for (i, data) in sf2_data_list.iter().enumerate() {
            soundfonts.push(
                SynthEngine::parse_soundfont(data)
                    .with_context(|| format!("Failed to parse SF2 {}", i))?,
            );
        }

        Self::from_soundfonts(&soundfonts, routing, sample_rate, port_count)
    }

    /// Create a multi-port bundle pool from already-parsed SoundFonts.
    ///
    /// Each port gets its own set of engines over the same `soundfonts`, so a
    /// caller that keeps a SoundFont cache pays the parse cost once per file
    /// rather than once per pool. `routing` maps channels 0-15 to an index into
    /// `soundfonts`; `port_count` is clamped to 1..=[`MAX_PORTS`].
    ///
    /// [`MAX_PORTS`]: crate::midi::event::MAX_PORTS
    pub fn from_soundfonts(
        soundfonts: &[Arc<SoundFont>],
        routing: [usize; 16],
        sample_rate: u32,
        port_count: u8,
    ) -> Result<Self> {
        if soundfonts.is_empty() {
            anyhow::bail!("SynthPool requires at least one SoundFont");
        }
        if let Some(&bad) = routing.iter().find(|&&i| i >= soundfonts.len()) {
            anyhow::bail!(
                "Routing entry {} is out of range for {} SoundFont(s)",
                bad,
                soundfonts.len()
            );
        }
        let port_count = port_count.clamp(1, MAX_PORTS);

        let engines_per_port = soundfonts.len();
        let mut engines = Vec::with_capacity(engines_per_port * port_count as usize);
        let mut port_routing = Vec::with_capacity(port_count as usize);

        for p in 0..port_count as usize {
            let base_offset = p * engines_per_port;
            for sf in soundfonts {
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

    /// Whether a channel currently sounds the drum bank.
    ///
    /// Read from the synthesizer rather than tracked alongside it, because a
    /// channel reaches the drum bank several ways: the default map, GS Use for
    /// Rhythm Part, XG Part Mode and, in XG mode, a Bank Select MSB of 126/127.
    pub fn is_percussion_channel(&self, port: u8, channel: usize) -> bool {
        let idx = self.engine_idx(port, channel as u8);
        self.engines[idx]
            .get_channel(channel & 0xF)
            .is_some_and(|c| c.get_is_percussion_channel())
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
