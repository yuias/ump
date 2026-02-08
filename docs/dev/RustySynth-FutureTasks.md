# RustySynth Fork — Future Improvement Tasks

Improvement tasks for the forked RustySynth synthesizer, ordered by priority.
Each task is self-contained and can be implemented independently unless noted otherwise.

**Reference files are relative to `rustysynth/src/`.**

---

## Task 1: Cubic (Hermite) Sample Interpolation

- **Priority**: S (Critical)
- **Effort**: Small
- **Impact**: All voices — most cost-effective single improvement
- **Status**: [x] ✅ Completed (`feature/cubic-interpolation` branch, commit `7980e13`)

### Problem

The oscillator (`oscillator.rs:120-124`) uses linear interpolation only:

```rust
x1 + frac * (x2 - x1)
```

Linear interpolation produces significant aliasing noise, especially in upper registers and during pitch bends. This manifests as audible "graininess" across all timbres.

### Proposed Solution

Implement 4-point Hermite (cubic) interpolation:

```
// Given samples x0, x1, x2, x3 and fractional position t:
c0 = x1
c1 = 0.5 * (x2 - x0)
c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3
c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2)
output = ((c3 * t + c2) * t + c1) * t + c0
```

### Implementation Notes

- **Files to modify**: `oscillator.rs`
- Requires access to 4 samples (x0, x1, x2, x3) instead of 2
  - `fill_block_no_loop`: handle boundary at `start` (x0 may be before start) and `end` (x3 may be past end) — clamp or mirror
  - `fill_block_continuous`: handle loop boundary wrap for x0 and x3
- Consider adding `InterpolationQuality` enum to `SynthesizerSettings` for runtime selection (Linear / Cubic / Sinc)
- Computational overhead: ~2.5x per voice vs linear (6 extra multiplies + 6 adds per sample)
- **Future extension**: 7th-order sinc interpolation (FluidSynth-equivalent) can be added later as a third option

### Verification

- A/B comparison: render the same MIDI with linear vs cubic, inspect spectrograms for alias reduction
- Focus on: high-pitched instruments (piccolo, glockenspiel), pitch bend passages, portamento

---

## Task 2: SF2 Default Modulators

- **Priority**: A (High)
- **Effort**: Medium
- **Impact**: Velocity sensitivity and dynamic expression accuracy
- **Status**: [x] ✅ Phase 1 completed (`feature/default-modulators`, PR #9)

### Problem

SF2 2.01 spec §8.4 defines 10 Default Modulators that should always be active. Currently, RustySynth hardcodes some modulation paths but misses several important ones:

| # | Source | Destination | Status |
|---|--------|------------|--------|
| 1 | Note-On Velocity → Initial Attenuation | Partially (custom formula in `voice.rs:143-154`) | ~Implemented |
| 2 | Note-On Velocity → Filter Cutoff | **Missing** |
| 3 | Channel Pressure → Vibrato LFO Pitch Depth | **Missing** |
| 4 | CC#1 (Mod Wheel) → Vibrato LFO Pitch Depth | Implemented |
| 5 | CC#7 (Volume) → Initial Attenuation (concave) | **Wrong curve** (squared linear, not concave) |
| 6 | CC#10 (Pan) → Pan Position | Implemented |
| 7 | CC#11 (Expression) → Initial Attenuation (concave) | **Wrong curve** (squared linear) |
| 8 | CC#91 (Reverb) → Reverb Send | Implemented |
| 9 | CC#93 (Chorus) → Chorus Send | Implemented |
| 10 | Pitch Wheel → Initial Pitch | Implemented |

### Proposed Solution

**Phase 1**: Implement the missing/incorrect default modulators:
- Add velocity → filter cutoff mapping (concave curve, SF2 spec)
- Add channel pressure → vibrato depth
- Fix CC#7/CC#11 to use SF2 concave mapping instead of squared linear

**Phase 2** (optional, separate task):
- Parse `pmod`/`imod` chunks from SF2 file (indices already tracked in `zone_info.rs`)
- Build per-voice modulation matrix at note-on
- Support all source types (linear, concave, convex, switch) and transforms

### Implementation Notes

- **Files to modify**: `voice.rs`, `channel.rs`, `region_pair.rs`, `soundfont_parameters.rs`
- SF2 concave mapping: `concave(x) = 20 * log10(x/127)` (cB, for 0 < x <= 127)
- Phase 2 requires new `modulator.rs` module + parser changes

### Verification

- Compare velocity response curves with FluidSynth / Polyphone
- Test with SF2 files that use custom modulators (e.g., velocity-to-filter sweep sounds)

---

## Task 3: LFO Sample-Rate Interpolation

- **Priority**: B (Medium)
- **Effort**: Small
- **Impact**: Vibrato / tremolo smoothness
- **Status**: [x] ✅ Completed (`feature/lfo-interpolation` branch, PR #10)

### Problem

LFO (`lfo.rs:48-68`) updates its value once per block (64 samples = ~1.45ms at 44.1kHz). For fast vibrato rates (8Hz+), this produces audible staircase artifacts in the pitch modulation path.

### Proposed Solution

Compute LFO value at block boundaries (start and end), then linearly interpolate per-sample within the voice processing loop.

```rust
// In Lfo:
pub fn process(&mut self) -> (f32, f32) {  // returns (value_start, value_end)
    let start = self.compute_value(self.processed_sample_count);
    self.processed_sample_count += self.block_size;
    let end = self.compute_value(self.processed_sample_count);
    self.value = end;
    (start, end)
}
```

Then in `voice.rs`, use `slope = (end - start) / block_size` to ramp per sample.

### Implementation Notes

- **Files to modify**: `lfo.rs`, `voice.rs`, `oscillator.rs` (pitch needs per-sample application)
- Current architecture applies pitch as a single value per block in `oscillator.rs:92-93` — this must be changed to accept per-sample pitch modulation
- A simpler alternative: increase LFO update rate to per-sample within `lfo.rs` without changing the voice interface (but wastes cycles on non-pitch paths)
- **Tradeoff**: full per-sample pitch requires `2^(pitch/12)` per sample — expensive. Consider SIMD or lookup table.

### Verification

- Render a sustained note with fast vibrato (8-10Hz, 50+ cents depth)
- Compare waveforms: block-rate should show staircase, interpolated should be smooth

---

## Task 4: Reverb Algorithm Upgrade (FDN)

- **Priority**: B (Medium)
- **Effort**: Medium-Large
- **Impact**: Spatial quality and naturalness of reverb tail
- **Status**: [x] ✅ Completed (`feature/fdn-reverb` branch, PR #11)

### Problem

Current reverb (`reverb.rs`) is Jezar's Freeverb:
- 8 parallel comb filters + 4 series allpass filters
- No early reflections — room "shape" is absent
- Fixed delay times (prime-number-based) produce metallic ringing on long tails
- Simple 1-pole damping in comb filters — unnatural high-frequency decay

### Proposed Solution

Replace with a **Feedback Delay Network (FDN)**, specifically an 8x8 Hadamard FDN with:

1. **Early reflections**: Tapped delay line (6-12 taps, Moorer-style) to simulate first reflections
2. **Late reverb**: 8-channel FDN with Hadamard mixing matrix
3. **Per-channel damping**: 1-pole or 2-pole LP per delay line for frequency-dependent decay
4. **Modulated delay lengths**: Slight random modulation to break up metallic modes

### Alternative Options

| Algorithm | Quality | Effort | Notes |
|-----------|---------|--------|-------|
| Freeverb + early reflections | Medium | Small | Quick win, still metallic on long tails |
| 8x8 Hadamard FDN | High | Medium | Best quality/effort ratio |
| Dattorro Plate Reverb | High | Medium | Bright, lush — good for GM/GS |
| Convolution | Very High | Large | Requires IR loading, high CPU/memory |

Recommendation: FDN or Dattorro. Both are well-documented algorithms with clear implementation paths.

### Implementation Notes

- **Files to modify**: `reverb.rs` (full rewrite), `synthesizer.rs` (interface unchanged)
- Maintain the existing `set_room_size`, `set_damp`, `set_wet`, `set_width` interface for backward compatibility
- Map GM reverb types (Room1/2/3, Hall1/2, Plate, Delay) to parameter presets
- Delay line lengths should scale with sample_rate (current approach is correct)

### Verification

- Render the same passage with old/new reverb, compare spectrograms for metallic artifacts
- Subjective listening test: focus on string ensembles and piano with high reverb send

---

## Task 5: Chorus Multi-Voice Enhancement

- **Priority**: B (Medium)
- **Effort**: Medium
- **Impact**: Ensemble richness for chorus-heavy patches
- **Status**: [x] ✅ Completed (`feature/chorus-multivoice` branch, PR #12)

### Problem

Current chorus (`chorus.rs`) uses a single delay voice per channel (L/R) with 90-degree phase offset sine LFO. This produces a thin, mechanical chorus effect.

GM Level 2 defines multiple chorus types (Chorus1-4, Feedback Chorus, Flanger) that are not distinguished.

### Proposed Solution

Expand to 3-voice chorus per channel (120-degree phase spacing):

```
Voice 1: delay + depth * sin(phase)
Voice 2: delay + depth * sin(phase + 2π/3)
Voice 3: delay + depth * sin(phase + 4π/3)
output = (voice1 + voice2 + voice3) / 3
```

Additionally, add GM chorus type presets:

| Type | Depth | Rate | Feedback | Delay |
|------|-------|------|----------|-------|
| Chorus 1 | 0.15ms | 1.0Hz | 0 | 3.0ms |
| Chorus 2 | 0.25ms | 0.8Hz | 0 | 4.0ms |
| Chorus 3 | 0.35ms | 1.2Hz | 0 | 5.0ms |
| Chorus 4 | 0.50ms | 1.5Hz | 0.2 | 6.0ms |
| FB Chorus | 0.30ms | 0.7Hz | 0.5 | 4.0ms |
| Flanger | 0.10ms | 0.3Hz | 0.7 | 1.0ms |

### Implementation Notes

- **Files to modify**: `chorus.rs`, `synthesizer.rs` (for type selection API)
- Each additional voice requires its own delay buffer and LFO phase offset
- Feedback path: feed output back into input with configurable gain (for FB Chorus and Flanger)
- Consider exposing chorus type via SysEx (GS: `40 01 38`, XG: chorus type MSB/LSB)

### Verification

- Compare with hardware GM module (SC-88 / MU-80) on string ensemble patches
- Flanger type: verify sweep rate and depth on electric piano

---

## Task 6: Volume Envelope Attack Curve (Convex)

- **Priority**: C (Low)
- **Effort**: Small
- **Impact**: Pad and string attack naturalness
- **Status**: [x] ✅ Completed (`feature/convex-attack` branch, PR #13)

### Problem

Volume envelope attack (`volume_envelope.rs:111`) uses a linear ramp:

```rust
value = attack_slope * (current_time - attack_start_time)
```

SF2 spec §8.1.3 recommends a convex curve for the attack phase. Linear attack sounds perceptually "sudden" on slow-attack timbres (strings, pads, choir).

### Proposed Solution

Replace with convex curve (FluidSynth-compatible):

```rust
// t = normalized time (0..1) within attack phase
value = 1.0 - (1.0 - t).powi(3)  // cubic convex
```

Or use the SF2 "concave" dB mapping:

```rust
// t = 0..1
value_db = -200.0 * (1.0 - t)  // centibels
value = 10.0f32.powf(value_db / 200.0)
```

### Implementation Notes

- **Files to modify**: `volume_envelope.rs`
- Only affects `EnvelopeStage::Attack` branch
- Consider making this configurable (linear vs convex) for compatibility testing
- Do NOT change `modulation_envelope.rs` — mod env attack is spec'd as linear

### Verification

- Render slow-attack pad (attack time = 500ms+)
- Compare envelope shape on oscilloscope view: linear should be straight line, convex should curve

---

## Task 7: Voice Stealing Strategy Improvement

- **Priority**: C (Low)
- **Effort**: Small
- **Impact**: Polyphony management under heavy load
- **Status**: [x] ✅ Completed (`feature/voice-stealing` branch, PR #14)

### Problem

Current voice stealing (`voice_collection.rs`) uses envelope-stage-based priority + age. This works well in most cases but has edge cases:

1. Stealing a voice from a different channel may cause audible note cutoff in an unrelated part
2. Rapid note repetition (drum rolls, trills) can exhaust polyphony without reusing same-key voices

### Proposed Solution

Enhanced priority calculation:

```rust
fn priority(&self, requesting_channel: i32, requesting_key: i32) -> f32 {
    let base = self.vol_env.get_priority();

    // Prefer stealing from the same channel
    let channel_bonus = if self.channel == requesting_channel { 2.0 } else { 0.0 };

    // Strongly prefer stealing same key (re-trigger)
    let key_bonus = if self.key == requesting_key && self.channel == requesting_channel {
        10.0
    } else {
        0.0
    };

    base - channel_bonus - key_bonus  // lower = steal first
}
```

### Implementation Notes

- **Files to modify**: `voice.rs`, `voice_collection.rs`, `synthesizer.rs`
- The `request_new_voice` method in `voice_collection.rs` needs the requesting channel/key as parameters
- Must not regress the existing exclusive_class killing behavior

### Verification

- Test with dense orchestral MIDI (16 channels, 64+ simultaneous notes)
- Verify that drum channel notes do not steal from melody channels

---

## Task 8: BiQuad Filter Stability Improvement (SVF)

- **Priority**: C (Low)
- **Effort**: Medium
- **Impact**: Filter stability under rapid parameter changes
- **Status**: [x] ✅ Completed (`feature/svf-filter` branch, PR #15)

### Problem

Current biquad LPF (`bi_quad_filter.rs`) uses Direct Form I with RBJ cookbook coefficients. Known issues:

1. Coefficient quantization sensitivity at high Q values
2. Potential instability when cutoff frequency changes rapidly (mitigated by current smoothing)
3. No high-pass or band-pass modes (SF2 only requires LP, but extended modes are useful)

### Proposed Solution

Replace with **Topology-Preserving Transform (TPT) State Variable Filter** (Andrew Simper / Cytomic):

- Inherently stable under parameter modulation
- Simultaneous LP/HP/BP outputs from single computation
- Integrator-based design avoids coefficient discontinuity issues
- Similar computational cost to biquad

### Implementation Notes

- **Files to modify**: `bi_quad_filter.rs` (rewrite internals, keep public interface)
- SVF state: two integrator states (`ic1eq`, `ic2eq`) instead of four delay states
- The `set_low_pass_filter` interface remains unchanged
- Cutoff smoothing in `voice.rs` may become unnecessary (SVF handles modulation natively)

### Verification

- Sweep cutoff from 20Hz to 20kHz with high resonance (Q=10)
- Verify no clicks, pops, or instability at any sweep rate
- Compare frequency response with current biquad implementation

---

## Implementation Order (Recommended)

```
Task 1 (Cubic Interpolation)
    │
    ├── Task 3 (LFO Interpolation)     ← can be done in parallel
    │
    v
Task 2 (Default Modulators)
    │
    v
Task 4 (Reverb FDN)
    │
    ├── Task 5 (Chorus Multi-Voice)     ← can be done in parallel
    │
    v
Task 6 (Attack Curve)
    │
    ├── Task 7 (Voice Stealing)         ← can be done in parallel
    ├── Task 8 (SVF Filter)             ← can be done in parallel
    v
  Done
```

Tasks 1 and 3 are independent and can be parallelized.
Tasks 4 and 5 (effects) are independent and can be parallelized.
Tasks 6, 7, 8 are all independent low-priority items.

---

## References

- SF2 2.01 Specification: <https://www.synthfont.com/sfspec24.pdf>
- FluidSynth source (interpolation, modulators): <https://github.com/FluidSynth/fluidsynth>
- Freeverb: <https://ccrma.stanford.edu/~jos/pasp/Freeverb.html>
- FDN Reverb: Jot, J.M. (1992) "Efficient models for reverberation and distance rendering"
- Dattorro Plate Reverb: <https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf>
- Cytomic SVF: <https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf>
- Hermite Interpolation: <https://www.musicdsp.org/en/latest/Other/93-hermite-interpollation.html>
