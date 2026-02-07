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

## Changed Files

| File | Phases | Changes |
|------|--------|---------|
| `rustysynth/src/channel.rs` | 1, 2 | pub visibility, raw getters, new CC fields |
| `rustysynth/src/synthesizer.rs` | 1, 2, 3, 4 | get_channel, mute, CC dispatch, reverb/chorus delegation |
| `rustysynth/src/reverb.rs` | 3 | set_* visibility (fn → pub(crate) fn) |
| `rustysynth/src/chorus.rs` | 4 | set_params method added |
| `rustysynth/src/lib.rs` | 1 | Channel re-export |

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
```
