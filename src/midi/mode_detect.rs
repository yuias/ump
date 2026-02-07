//! GM/GS/XG/GM2 MIDI mode detection from SysEx messages.

use crate::midi::event::{MidiEvent, TimedMidiEvent};
use crate::midi::sysex::{parse_sysex, SysExCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMode {
    GM,
    GS,
    XG,
    GM2,
}

impl std::fmt::Display for MidiMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiMode::GM => write!(f, "GM"),
            MidiMode::GS => write!(f, "GS"),
            MidiMode::XG => write!(f, "XG"),
            MidiMode::GM2 => write!(f, "GM2"),
        }
    }
}

/// Detect the MIDI mode from SysEx and Bank Select patterns.
///
/// Phase 1: First reset SysEx message (GS → XG → GM2 → GM priority).
/// Phase 2: Bank Select (CC#0 MSB) pattern inference.
/// Default: GM if nothing detected.
pub fn detect_mode(events: &[TimedMidiEvent]) -> MidiMode {
    // Phase 1: SysEx reset messages
    for evt in events {
        if let MidiEvent::SysEx(data) = &evt.event {
            if let Some(SysExCommand::SystemReset(mode)) = parse_sysex(data) {
                log_info!("Mode detect: Phase 1 SysEx reset -> {}", mode);
                return mode;
            }
        }
    }

    // Phase 2: Infer from Bank Select (CC#0 MSB) patterns
    let mut bank_msb_values: Vec<u8> = Vec::new();
    for evt in events {
        if let MidiEvent::ControlChange {
            controller: 0,
            value,
            ..
        } = &evt.event
        {
            if !bank_msb_values.contains(value) {
                bank_msb_values.push(*value);
            }
        }
    }

    if !bank_msb_values.is_empty()
        && !(bank_msb_values.len() == 1 && bank_msb_values[0] == 0)
    {
        log_info!("Mode detect: Phase 2 Bank MSB values={:?}", bank_msb_values);

        // XG-specific banks: 64 (SFX Kit), 126 (SFX), 127 (Drum SFX)
        for &bank in &bank_msb_values {
            if bank == 64 || bank == 126 || bank == 127 {
                log_info!("Mode detect: Phase 2 -> XG (bank={})", bank);
                return MidiMode::XG;
            }
        }

        // GM2: 121 (GM2 melody variations)
        if bank_msb_values.contains(&121) {
            log_info!("Mode detect: Phase 2 -> GM2 (bank=121)");
            return MidiMode::GM2;
        }

        // GS: 1-63 (Capital/variation), 80-87 (SFX)
        for &bank in &bank_msb_values {
            if (1..=63).contains(&bank) || (80..=87).contains(&bank) {
                log_info!("Mode detect: Phase 2 -> GS (bank={})", bank);
                return MidiMode::GS;
            }
        }
    }

    log_info!("Mode detect: no signals -> GM default");
    MidiMode::GM
}