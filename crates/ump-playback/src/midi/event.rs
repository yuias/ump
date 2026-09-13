//! Internal MIDI event types, decoupled from midly.

pub const MAX_PORTS: u8 = 4;
pub const CHANNELS_PER_PORT: u8 = 16;
pub const MAX_CHANNELS: usize = (MAX_PORTS as usize) * (CHANNELS_PER_PORT as usize); // 64

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TimedMidiEvent {
    /// Absolute time in ticks from the start of the MIDI file.
    pub tick: u64,
    /// The MIDI event payload.
    pub event: MidiEvent,
    /// Original track index (0-based).
    pub track: usize,
}

#[derive(Debug, Clone)]
pub enum MidiEvent {
    NoteOn { port: u8, channel: u8, key: u8, vel: u8 },
    NoteOff { port: u8, channel: u8, key: u8 },
    ProgramChange { port: u8, channel: u8, program: u8 },
    ControlChange { port: u8, channel: u8, controller: u8, value: u8 },
    PitchBend { port: u8, channel: u8, value: i16 },
    /// Polyphonic key pressure (aftertouch per note).
    PolyAftertouch { port: u8, channel: u8, key: u8, pressure: u8 },
    /// Channel pressure (aftertouch for entire channel).
    ChannelAftertouch { port: u8, channel: u8, pressure: u8 },
    /// Raw SysEx payload (without F0/F7 framing).
    SysEx(Vec<u8>),
    /// Tempo change in microseconds per quarter note.
    TempoChange(u32),
    /// Time signature: numerator, denominator (as power of 2).
    TimeSignature { numerator: u8, denominator: u8 },
}

/// A note rectangle for piano roll visualization.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NoteRect {
    /// MIDI key number (0-127).
    pub key: u8,
    /// MIDI channel (0-15).
    pub channel: u8,
    /// MIDI port (0-3).
    pub port: u8,
    /// Start tick.
    pub start_tick: u64,
    /// End tick (exclusive).
    pub end_tick: u64,
    /// Velocity (1-127).
    pub velocity: u8,
    /// Track index.
    pub track: usize,
}

/// Metadata about a single MIDI track.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub index: usize,
    pub name: String,
    /// Primary channel used by this track (heuristic: most frequent channel).
    pub channel: Option<u8>,
    /// MIDI port assigned to this track (from FF 21 meta event).
    pub port: u8,
    /// Program number on the primary channel, if any.
    pub program: Option<u8>,
    /// Total number of note-on events in this track.
    pub note_count: u32,
    /// Note counts per channel (index = channel number 0..15).
    pub channel_note_counts: [u32; 16],
    /// Program number per channel (index = channel number 0..15).
    pub channel_programs: [Option<u8>; 16],
}

/// Top-level parsed MIDI data.
#[derive(Debug, Clone)]
pub struct MidiData {
    /// Ticks per quarter note.
    pub ticks_per_quarter: u16,
    /// SMF format (0, 1, or 2).
    pub format: u8,
    /// All timed events, sorted by tick then by original order.
    pub events: Vec<TimedMidiEvent>,
    /// Note rectangles for piano roll.
    pub note_rects: Vec<NoteRect>,
    /// Per-track metadata.
    pub tracks: Vec<TrackInfo>,
    /// Total duration in ticks.
    pub total_ticks: u64,
    /// Bitfield of channels that have at least one NoteOn event (port*16+ch).
    pub used_channels: u64,
    /// Number of MIDI ports used (1-4).
    pub port_count: u8,
}
