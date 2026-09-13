//! Playback core shared by ump and its web port: MIDI parsing, mode
//! detection, and SoundFont synthesis via rustysynth.
//!
//! Audio output, UI, and rendering stay in the host application so this
//! crate builds for both native and `wasm32` targets. Logging goes through
//! the `log` facade; hosts install whatever logger fits their platform.

pub mod midi;
pub mod synth;
