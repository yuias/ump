//! SysEx message parsing: converts raw SysEx byte sequences to typed commands.

use super::mode_detect::MidiMode;

/// A parsed SysEx command that the sequencer can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysExCommand {
    /// System reset (GM/GM2/GS/XG).
    SystemReset(MidiMode),
    /// GS drum map assignment: set a channel as drum or normal.
    GsDrumMap { channel: u8, is_drum: bool },
    /// Universal Master Volume (0-127).
    MasterVolume(u8),
}

/// Parse a raw SysEx payload (F0 already stripped by midly) into a typed command.
/// Returns `None` if the SysEx is not recognized.
pub fn parse_sysex(data: &[u8]) -> Option<SysExCommand> {
    // Strip trailing F7 if present
    let data = data.strip_suffix(&[0xF7]).unwrap_or(data);

    if data.len() < 4 {
        return None;
    }

    // Universal Non-Real-Time: 7E 7F 09 xx
    if data[0] == 0x7E && data[1] == 0x7F && data[2] == 0x09 {
        return match data[3] {
            0x01 => Some(SysExCommand::SystemReset(MidiMode::GM)),
            0x03 => Some(SysExCommand::SystemReset(MidiMode::GM2)),
            _ => None,
        };
    }

    // Universal Real-Time Master Volume: 7F 7F 04 01 [lsb] [msb]
    if data.len() >= 6
        && data[0] == 0x7F
        && data[1] == 0x7F
        && data[2] == 0x04
        && data[3] == 0x01
    {
        let msb = data[5];
        return Some(SysExCommand::MasterVolume(msb));
    }

    // Roland GS messages: 41 10 42 12 ...
    if data.len() >= 5
        && data[0] == 0x41
        && data[1] == 0x10
        && data[2] == 0x42
        && data[3] == 0x12
    {
        return parse_gs_sysex(&data[4..]);
    }

    // Yamaha XG System On: 43 10 4C 00 00 7E 00
    if data.len() >= 7
        && data[0] == 0x43
        && data[1] == 0x10
        && data[2] == 0x4C
        && data[3] == 0x00
        && data[4] == 0x00
        && data[5] == 0x7E
        && data[6] == 0x00
    {
        return Some(SysExCommand::SystemReset(MidiMode::XG));
    }

    None
}

/// Parse the body of a Roland GS SysEx (after 41 10 42 12).
fn parse_gs_sysex(body: &[u8]) -> Option<SysExCommand> {
    if body.len() < 4 {
        return None;
    }

    // GS Reset: 40 00 7F 00 41
    if body.len() >= 5
        && body[0] == 0x40
        && body[1] == 0x00
        && body[2] == 0x7F
        && body[3] == 0x00
        && body[4] == 0x41
    {
        return Some(SysExCommand::SystemReset(MidiMode::GS));
    }

    // GS Drum Map: 40 1x 15 [val] [checksum]
    // x = part number (0-15)
    if body.len() >= 4
        && body[0] == 0x40
        && (body[1] & 0xF0) == 0x10
        && body[2] == 0x15
    {
        let part = body[1] & 0x0F;
        let channel = gs_part_to_channel(part);
        let is_drum = body[3] != 0;
        return Some(SysExCommand::GsDrumMap { channel, is_drum });
    }

    None
}

/// Convert GS part number to MIDI channel.
/// Part 0 → Ch 9, Part 1-9 → Ch 0-8, Part 10-15 → Ch 10-15.
fn gs_part_to_channel(part: u8) -> u8 {
    match part {
        0 => 9,
        1..=9 => part - 1,
        _ => part,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gm_reset() {
        let data = [0x7E, 0x7F, 0x09, 0x01];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::SystemReset(MidiMode::GM))
        );
    }

    #[test]
    fn test_gm2_reset() {
        let data = [0x7E, 0x7F, 0x09, 0x03];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::SystemReset(MidiMode::GM2))
        );
    }

    #[test]
    fn test_gs_reset() {
        let data = [0x41, 0x10, 0x42, 0x12, 0x40, 0x00, 0x7F, 0x00, 0x41];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::SystemReset(MidiMode::GS))
        );
    }

    #[test]
    fn test_xg_system_on() {
        let data = [0x43, 0x10, 0x4C, 0x00, 0x00, 0x7E, 0x00];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::SystemReset(MidiMode::XG))
        );
    }

    #[test]
    fn test_gs_drum_map() {
        // Part 0 (Ch9) → drum on
        let data = [0x41, 0x10, 0x42, 0x12, 0x40, 0x10, 0x15, 0x02, 0x19];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::GsDrumMap {
                channel: 9,
                is_drum: true,
            })
        );

        // Part 2 (Ch1) → drum on
        let data = [0x41, 0x10, 0x42, 0x12, 0x40, 0x12, 0x15, 0x01, 0x18];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::GsDrumMap {
                channel: 1,
                is_drum: true,
            })
        );
    }

    #[test]
    fn test_master_volume() {
        let data = [0x7F, 0x7F, 0x04, 0x01, 0x00, 0x7F];
        assert_eq!(parse_sysex(&data), Some(SysExCommand::MasterVolume(0x7F)));
    }

    #[test]
    fn test_trailing_f7_stripped() {
        let data = [0x7E, 0x7F, 0x09, 0x01, 0xF7];
        assert_eq!(
            parse_sysex(&data),
            Some(SysExCommand::SystemReset(MidiMode::GM))
        );
    }

    #[test]
    fn test_unknown_sysex() {
        let data = [0x00, 0x01, 0x02];
        assert_eq!(parse_sysex(&data), None);
    }

    #[test]
    fn test_gs_part_to_channel() {
        assert_eq!(gs_part_to_channel(0), 9);
        assert_eq!(gs_part_to_channel(1), 0);
        assert_eq!(gs_part_to_channel(9), 8);
        assert_eq!(gs_part_to_channel(10), 10);
        assert_eq!(gs_part_to_channel(15), 15);
    }
}
