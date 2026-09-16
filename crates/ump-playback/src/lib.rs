//! Playback core shared by ump and its web port: MIDI parsing, mode
//! detection, sequencing, and SoundFont synthesis via rustysynth.
//!
//! Audio output, UI, and rendering stay in the host application so this
//! crate builds for both native and `wasm32` targets. Logging goes through
//! the `log` facade; hosts install whatever logger fits their platform.

// Re-exported so hosts can name the SoundFont type returned by
// `synth::engine::SynthEngine::parse_soundfont` without depending on
// rustysynth directly, which would risk linking two copies of it.
pub use rustysynth;

pub mod midi;
pub mod sequencer;
pub mod synth;
