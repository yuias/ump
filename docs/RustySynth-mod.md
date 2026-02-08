# RustySynth Fork Modifications

## Overview

This document describes the modifications made to the [rustysynth](https://github.com/sinshu/rustysynth) fork at `yuiAs/rustysynth` (branch: `develop`) for use in the ump MIDI player.

All changes are **additive only** — no existing public API is broken.

## Fork URL

- Repository: `https://github.com/yuiAs/rustysynth`
- Branch: `develop`
- Upstream: `https://github.com/sinshu/rustysynth`

## Modification Summary

### Phase 1: Channel State Exposure + Channel Mute

**Purpose:** Allow external code (ump UI) to read channel state and mute individual channels.

#### channel.rs
- `Channel` struct visibility changed from `pub(crate)` to `pub`
- All existing getters changed from `pub(crate)` to `pub`:
  - `get_bank_number`, `get_patch_number`, `get_volume`, `get_pan`, `get_expression`
  - `get_modulation`, `get_hold_pedal`, `get_reverb_send`, `get_chorus_send`
  - `get_pitch_bend`, `get_pitch_bend_range`, `get_tune`
- New raw getters added (return 0-127 values for UI display):
  - `get_volume_raw() -> u8`
  - `get_pan_raw() -> u8`
  - `get_expression_raw() -> u8`
  - `get_reverb_send_raw() -> u8`
  - `get_chorus_send_raw() -> u8`
  - `get_is_percussion_channel() -> bool`

#### synthesizer.rs
- `get_channel(channel: usize) -> Option<&Channel>` — access channel state
- `set_channel_mute(channel: usize, muted: bool)` — mute/unmute a channel
- `is_channel_muted(channel: usize) -> bool` — query mute state
- `set_channel_mute_mask(mask: u16)` — set all 16 channels at once (bitmask)
- `get_channel_mute_mask() -> u16` — get current mute bitmask
- `render_block()` modified: muted channels have their gain set to 0 in all three voice loops (dry/chorus/reverb), preserving voice internal state

#### lib.rs
- `pub use self::channel::Channel;` added

### Phase 2: Additional CC Support

**Purpose:** Handle additional MIDI CC messages for better compatibility.

#### channel.rs — New fields
| Field | CC# | Type | Default |
|-------|-----|------|---------|
| `bank_lsb` | 32 (0x20) | `i32` | 0 |
| `sostenuto_pedal` | 66 (0x42) | `bool` | false |
| `soft_pedal` | 67 (0x43) | `bool` | false |
| `variation_send` | 94 (0x5E) | `u8` | 0 |

- Setters: `pub(crate)` (internal)
- Getters: `pub` (external) — `get_bank_lsb`, `get_sostenuto_pedal`, `get_soft_pedal`, `get_variation_send` (normalized), `get_variation_send_raw`
- `reset()`: all new fields reset to defaults
- `reset_all_controllers()`: `sostenuto_pedal`, `soft_pedal` reset (bank_lsb, variation_send preserved)

#### synthesizer.rs — CC dispatch
- `0x20` → `set_bank_lsb`
- `0x42` → `set_sostenuto_pedal`
- `0x43` → `set_soft_pedal`
- `0x5E` → `set_variation_send`

### Phase 3: Reverb Parameter Exposure

**Purpose:** Allow runtime adjustment of reverb parameters.

#### reverb.rs
- `set_room_size`, `set_damp`, `set_wet`, `set_width` changed from `fn` to `pub(crate) fn`

#### synthesizer.rs — Delegation methods
- `set_reverb_room_size(value: f32)` — 0.0-1.0, default 0.5
- `set_reverb_damp(value: f32)` — 0.0-1.0, default 0.5
- `set_reverb_wet(value: f32)` — 0.0-1.0, default ~0.33
- `set_reverb_width(value: f32)` — 0.0-1.0, default 1.0

All guarded by `if let Some(effects) = self.effects.as_mut()`.

### Phase 4: Chorus Parameter Exposure

**Purpose:** Allow runtime adjustment of chorus parameters.

#### chorus.rs
- `set_params(sample_rate: i32, delay: f64, depth: f64, frequency: f64)` added
  - Reallocates buffers, recomputes delay table, resets indices
  - Same logic as constructor

#### synthesizer.rs — Delegation method
- `set_chorus_params(delay: f64, depth: f64, frequency: f64)` — defaults: 0.002, 0.0019, 0.4

### Phase 5: Sound Controller CCs (CC#71-75)

**Purpose:** GM2/GS/XG sound controller support for filter and envelope modulation.

#### channel.rs — New fields
| Field | CC# | Type | Default | Effect |
|-------|-----|------|---------|--------|
| `filter_resonance` | 71 (0x47) | `u8` | 64 | Filter Q offset (dB) |
| `release_time` | 72 (0x48) | `u8` | 64 | Envelope release multiplier |
| `attack_time` | 73 (0x49) | `u8` | 64 | Envelope attack multiplier |
| `brightness` | 74 (0x4A) | `u8` | 64 | Filter cutoff offset (cents) |
| `decay_time` | 75 (0x4B) | `u8` | 64 | Envelope decay multiplier |

- All use center value 64 = no change
- Brightness: each unit = ~50 cents offset on cutoff frequency
- Resonance: each unit = ~0.5 dB offset on filter Q
- Attack/Decay/Release: each unit = ~50 timecents (2^(offset*50/1200) multiplier)

#### voice.rs — Filter modulation
- `process()`: applies brightness (cents) and resonance (dB) offsets to filter per block
- Real-time modulation — changes take effect on the next audio block

#### region_ex.rs — Envelope modulation
- `start_volume_envelope()` now takes `&Channel` and applies attack/decay/release multipliers
- Applied at note-on time — CC changes affect subsequent notes only

#### synthesizer.rs
- CC dispatch: `0x47`-`0x4B` → respective channel setters
- `voice.start()` signature extended with `&Channel` parameter

### Phase 6: NRPN Processing

**Purpose:** Support GS/XG tone modify parameters via NRPN.

#### channel.rs
- `nrpn: i16` field — tracks current NRPN address (MSB<<7|LSB)
- `DataType` enum (None/Rpn/Nrpn) — routes Data Entry to correct handler
- NRPN dispatch (MSB=1): vibrato rate/depth/delay (new fields), TVF cutoff/resonance, TVA attack/decay/release (mapped to existing CC fields)
- `vibrato_rate`, `vibrato_depth`, `vibrato_delay` fields (0-127, default 64)
- Multiplier getters for voice integration

#### region_ex.rs
- `start_vibrato()` takes `&Channel`, applies rate/delay multipliers to LFO

#### voice.rs
- `vib_lfo_to_pitch` scaled by depth multiplier at note-on

### Phase 7: Master Tuning + SysEx Processing

**Purpose:** Global pitch tuning and SysEx message handling.

#### synthesizer.rs
- `master_tune: f32` field (semitones, default 0.0)
- `get_master_tune()` / `set_master_tune(value)` public API
- `process_sysex(data: &[u8])` — handles Universal Real-Time SysEx:
  - Master Volume (04 01), Master Fine Tune (04 03), Master Coarse Tune (04 04)
  - GM System On (7E xx 09 01/02/03), GS Reset, XG System On → full reset

#### voice.rs / voice_collection.rs
- `master_tune` threaded through process() as pitch offset

### Phase 8: Scale Tuning

**Purpose:** Per-channel microtuning support.

#### channel.rs
- `scale_tuning: [f32; 12]` — cents offset per pitch class (C..B)
- `get_scale_tuning_for_key(key)` converts to semitones

#### synthesizer.rs
- `set_scale_tuning(channel, &[f32; 12])` / `get_scale_tuning(channel)` public API
- GS SysEx dispatcher: `process_gs_sysex()` with Scale Tuning (addr 40 1X 40)
- GS part-to-MIDI channel mapping

#### voice.rs
- Scale tuning offset added to pitch calculation

### Phase 9: Portamento

**Purpose:** Smooth pitch glide between notes.

#### channel.rs
- `portamento_on` (CC#65), `portamento_time` (CC#5), `portamento_control` (CC#84)
- `last_note_on_key` tracking for automatic source detection
- `get_portamento_speed(sample_rate)` — exponential time mapping
- `consume_portamento_source()` — CC#84 one-shot with last-note fallback

#### synthesizer.rs
- CC dispatch: 0x05, 0x41 (portamento on/off), 0x54 (portamento control)
- `note_on()` extracts portamento info, passes to voice

#### voice.rs
- `portamento_offset` / `portamento_speed` fields
- Pitch offset decays towards 0 per audio block

### Phase 10: Dynamic Drum Map (Percussion Channel Toggle)

**Purpose:** Enable GS SysEx drum map switching (`40 1x 15`) to dynamically reassign any channel as percussion/normal at runtime.

#### channel.rs
- `set_percussion_channel(is_percussion: bool)` — `pub(crate)` setter for `is_percussion_channel` flag
- Flag drives `reset()` (bank 128 vs 0) and `set_bank()` (+128 auto-offset)

#### synthesizer.rs
- `set_percussion_channel(channel: usize, is_percussion: bool)` — public API delegating to `Channel`

#### Usage in ump
- `SynthEngine::set_percussion_channel()` wrapper
- `system_reset()`: resets all channels to default (ch9 = percussion, others = normal) before bank/program reset
- `seek_to_tick()`: resets percussion flags after `synth.reset()`, then replays GsDrumMap events
- `dispatch_event()` GsDrumMap: calls `set_percussion_channel()` + bank 0 (auto-offset handles +128)

### Phase 11: Cubic Hermite Sample Interpolation

**Purpose:** Replace linear interpolation with higher-quality 4-point Hermite cubic interpolation for reduced aliasing.

#### oscillator.rs
- Linear interpolation (`x1 + frac * (x2 - x1)`) replaced with 4-point Hermite cubic
- Uses samples `x0, x1, x2, x3` with fractional position `t` for 3rd-order polynomial
- Loop boundary handling: wrap-around sample access for `fill_block_continuous`, clamping for `fill_block_no_loop`
- Result: ~20dB reduction in harmonic distortion vs linear, especially audible in upper registers and pitch bends

### Phase 12: SF2 Default Modulators (Phase 1)

**Purpose:** Implement missing SF2 2.01 §8.4 Default Modulators for correct velocity sensitivity and CC response.

#### voice.rs / channel.rs / region_pair.rs
- Velocity → filter cutoff (Default Modulator #2): concave mapping applied at note-on
- Channel Pressure → vibrato LFO pitch depth (Default Modulator #3)
- CC#7 (Volume) → initial attenuation: corrected from squared linear to SF2 concave curve
- CC#11 (Expression) → initial attenuation: corrected from squared linear to SF2 concave curve
- SF2 concave mapping: `20 * log10(x/127)` (centibels)

### Phase 13: LFO Per-Sample Pitch Interpolation

**Purpose:** Eliminate staircase artifacts in vibrato by interpolating LFO values between block boundaries.

#### lfo.rs
- `process()` returns `(value_start, value_end)` tuple for block boundary values
- Previous block-rate update (64 samples ≈ 1.45ms) caused audible steps at fast vibrato rates (8Hz+)

#### voice.rs / oscillator.rs
- Per-sample linear interpolation: `slope = (end - start) / block_size`, applied per sample
- Smooth pitch modulation path eliminates block-boundary discontinuities

### Phase 14: FDN Reverb

**Purpose:** Replace Freeverb with 8×8 Hadamard Feedback Delay Network for natural, artifact-free reverberation.

#### reverb.rs — Full rewrite
- **Early reflections**: Moorer-style tapped delay line (6-12 taps) for room shape simulation
- **Late reverb**: 8-channel FDN with Hadamard orthogonal mixing matrix
- **Per-channel damping**: frequency-dependent decay via 1-pole LP per delay line
- **Modulated delays**: slight random modulation to break up metallic modes
- Existing API preserved: `set_room_size`, `set_damp`, `set_wet`, `set_width`

### Phase 15: Multi-Voice Chorus

**Purpose:** Expand from single-voice to 3-voice chorus with feedback and GM preset support.

#### chorus.rs — Major rewrite
- 3 chorus voices per channel with 120° phase spacing
- Feedback path for FB Chorus and Flanger types
- GM chorus type presets:

| Type | Depth | Rate | Feedback | Delay |
|------|-------|------|----------|-------|
| Chorus 1 | 0.15ms | 1.0Hz | 0 | 3.0ms |
| Chorus 2 | 0.25ms | 0.8Hz | 0 | 4.0ms |
| Chorus 3 | 0.35ms | 1.2Hz | 0 | 5.0ms |
| Chorus 4 | 0.50ms | 1.5Hz | 0.2 | 6.0ms |
| FB Chorus | 0.30ms | 0.7Hz | 0.5 | 4.0ms |
| Flanger | 0.10ms | 0.3Hz | 0.7 | 1.0ms |

### Phase 16: Convex Volume Envelope Attack

**Purpose:** SF2-compliant convex attack curve for natural pad/string onset.

#### volume_envelope.rs
- Linear attack ramp replaced with convex curve (FluidSynth-compatible)
- Cubic convex: `value = 1.0 - (1.0 - t).powi(3)` where `t` = normalized attack time
- Only affects `EnvelopeStage::Attack`; modulation envelope unchanged (spec: linear)
- Primary benefit: slow-attack timbres (strings, pads, choir) sound more natural

### Phase 17: Context-Aware Voice Stealing

**Purpose:** Improved polyphony management that considers channel and key context.

#### voice.rs / voice_collection.rs / synthesizer.rs
- Enhanced priority calculation with channel and key bonuses:
  - Same-channel voices preferred for stealing (reduces cross-channel note cutoff)
  - Same-key voices strongly preferred (efficient re-trigger for drum rolls, trills)
- `request_new_voice` extended with requesting channel/key parameters
- Exclusive class killing behavior preserved

### Phase 18: SVF Filter (Cytomic TPT)

**Purpose:** Replace Direct Form I biquad with Topology-Preserving Transform State Variable Filter for stability under modulation.

#### bi_quad_filter.rs — Internal rewrite
- Cytomic TPT SVF (Andrew Simper) replaces RBJ cookbook biquad
- Integrator-based design: two states (`ic1eq`, `ic2eq`) instead of four delay states
- Inherently stable under rapid parameter modulation (no coefficient discontinuity)
- Simultaneous LP/HP/BP outputs from single computation
- Public interface (`set_low_pass_filter`) unchanged

## Changed Files

| File | Phases | Changes |
|------|--------|---------|
| `rustysynth/src/channel.rs` | 1-2, 5-10, 12 | pub visibility, raw getters, CC fields, NRPN, scale tuning, portamento, percussion toggle, pressure routing |
| `rustysynth/src/synthesizer.rs` | 1-4, 6-10, 17 | get_channel, mute, CC dispatch, reverb/chorus, SysEx, master tune, scale tuning, portamento, percussion toggle, voice stealing context |
| `rustysynth/src/reverb.rs` | 3, 14 | Phase 3: set_* visibility; Phase 14: full FDN rewrite |
| `rustysynth/src/chorus.rs` | 4, 15 | Phase 4: set_params; Phase 15: 3-voice rewrite with feedback and presets |
| `rustysynth/src/lib.rs` | 1 | Channel re-export |
| `rustysynth/src/voice.rs` | 5-9, 11-13, 17 | Filter offset, master tune, scale tuning, portamento, cubic interpolation, LFO interpolation, modulator routing, voice stealing priority |
| `rustysynth/src/voice_collection.rs` | 7, 17 | master_tune parameter, context-aware voice stealing |
| `rustysynth/src/region_ex.rs` | 5-6, 12 | &Channel in start_volume_envelope/start_vibrato, velocity-cutoff modulator |
| `rustysynth/src/oscillator.rs` | 11, 13 | Hermite cubic interpolation, per-sample pitch from LFO |
| `rustysynth/src/lfo.rs` | 13 | Block-boundary value pair output for interpolation |
| `rustysynth/src/volume_envelope.rs` | 16 | Convex attack curve |
| `rustysynth/src/bi_quad_filter.rs` | 18 | Cytomic TPT SVF rewrite (internal) |
| `rustysynth/src/region_pair.rs` | 12 | Modulator parameter integration |

## Usage in ump

```toml
# Cargo.toml
rustysynth = { git = "https://github.com/yuiAs/rustysynth", branch = "develop" }
```

```rust
// Channel state reading
if let Some(ch) = synth.get_channel(0) {
    let vol = ch.get_volume_raw();    // 0-127
    let pan = ch.get_pan_raw();       // 0-127
    let is_drum = ch.get_is_percussion_channel();
}

// Channel mute
synth.set_channel_mute(9, true);      // Mute percussion
let mask = synth.get_channel_mute_mask();

// Reverb/Chorus parameters
synth.set_reverb_room_size(0.7);
synth.set_chorus_params(0.003, 0.002, 0.5);

// SysEx processing (raw data without F0/F7)
synth.process_sysex(&sysex_data);

// Master tuning
synth.set_master_tune(0.5);  // +50 cents

// Scale tuning (per-channel, cents per pitch class)
synth.set_scale_tuning(0, &[0.0, -10.0, 0.0, 0.0, -5.0, 0.0, 0.0, 2.0, 0.0, 0.0, -8.0, 0.0]);

// Dynamic drum map — set channel 1 as percussion
synth.set_percussion_channel(1, true);
```
