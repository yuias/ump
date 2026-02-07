//! Shared state between audio thread and UI thread.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// Per-channel MIDI state for Extended track display.
/// All values updated atomically from the audio thread.
pub struct ChannelStates {
    /// Current program number (0-127).
    pub program: [AtomicU32; 16],
    /// CC7 Volume (0-127).
    pub volume: [AtomicU32; 16],
    /// CC11 Expression (0-127).
    pub expression: [AtomicU32; 16],
    /// CC10 Pan (0-127, 64=center).
    pub pan: [AtomicU32; 16],
    /// CC1 Modulation (0-127).
    pub modulation: [AtomicU32; 16],
    /// Pitch bend (0-16383, 8192=center).
    pub pitch_bend: [AtomicU32; 16],
    /// CC64 Sustain pedal (0=off, 127=on).
    pub pedal: [AtomicU32; 16],
    /// Channel aftertouch / pressure (0-127).
    pub aftertouch: [AtomicU32; 16],
    /// Last note-on velocity (0-127), for activity display.
    pub velocity: [AtomicU32; 16],
    /// CC0 Bank Select MSB (0-127).
    pub bank: [AtomicU32; 16],
    /// CC91 Reverb Send (0-127).
    pub reverb: [AtomicU32; 16],
    /// CC93 Chorus Send (0-127).
    pub chorus: [AtomicU32; 16],
}

impl ChannelStates {
    pub fn new() -> Self {
        ChannelStates {
            program: std::array::from_fn(|_| AtomicU32::new(0)),
            volume: std::array::from_fn(|_| AtomicU32::new(100)),
            expression: std::array::from_fn(|_| AtomicU32::new(127)),
            pan: std::array::from_fn(|_| AtomicU32::new(64)),
            modulation: std::array::from_fn(|_| AtomicU32::new(0)),
            pitch_bend: std::array::from_fn(|_| AtomicU32::new(0)),
            pedal: std::array::from_fn(|_| AtomicU32::new(0)),
            aftertouch: std::array::from_fn(|_| AtomicU32::new(0)),
            velocity: std::array::from_fn(|_| AtomicU32::new(0)),
            bank: std::array::from_fn(|_| AtomicU32::new(0)),
            reverb: std::array::from_fn(|_| AtomicU32::new(40)),
            chorus: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// Reset all channel states to defaults.
    pub fn reset(&self) {
        for i in 0..16 {
            self.program[i].store(0, Ordering::Relaxed);
            self.volume[i].store(100, Ordering::Relaxed);
            self.expression[i].store(127, Ordering::Relaxed);
            self.pan[i].store(64, Ordering::Relaxed);
            self.modulation[i].store(0, Ordering::Relaxed);
            self.pitch_bend[i].store(0, Ordering::Relaxed);
            self.pedal[i].store(0, Ordering::Relaxed);
            self.aftertouch[i].store(0, Ordering::Relaxed);
            self.velocity[i].store(0, Ordering::Relaxed);
            self.bank[i].store(0, Ordering::Relaxed);
            self.reverb[i].store(40, Ordering::Relaxed);
            self.chorus[i].store(0, Ordering::Relaxed);
        }
    }
}

pub struct SharedState {
    /// Current playback position in ticks.
    pub current_tick: AtomicU64,
    /// Whether playback is active (not paused).
    pub playing: AtomicBool,
    /// Whether playback has been stopped (reset to beginning).
    pub stopped: AtomicBool,
    /// App volume (0-100), controlled by user +/- keys.
    pub volume: AtomicU32,
    /// MIDI Master Volume from SysEx (0-127, default 127).
    pub master_volume: AtomicU32,
    /// Seek target in ticks. 0 = no pending seek.
    /// Audio thread reads and clears this.
    pub seek_tick: AtomicU64,
    /// Whether the sequencer has reached the end of the file.
    pub finished: AtomicBool,
    /// Per-channel mute flags (bitfield, bit N = channel N muted).
    pub muted_channels: AtomicU32,
    /// Current BPM * 100 (to avoid floats in atomic).
    pub current_bpm_x100: AtomicU32,
    /// Current time signature numerator.
    pub time_sig_num: AtomicU32,
    /// Current time signature denominator (as power of 2).
    pub time_sig_den: AtomicU32,
    /// Track info (updated once at load time).
    pub track_info: Mutex<Vec<super::TrackInfoSnapshot>>,
    /// Per-channel MIDI state (updated in real-time from audio thread).
    pub channel_states: ChannelStates,
    /// Drum channel bitfield: bit N = channel N is drum. Default: 1 << 9.
    pub drum_channels: AtomicU32,
}

/// Snapshot of track info for UI display.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TrackInfoSnapshot {
    pub index: usize,
    pub name: String,
    pub channel: Option<u8>,
    pub program: Option<u8>,
    pub note_count: u32,
    pub channel_note_counts: [u32; 16],
    pub channel_programs: [Option<u8>; 16],
}

impl SharedState {
    pub fn new() -> Self {
        SharedState {
            current_tick: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            volume: AtomicU32::new(80),
            master_volume: AtomicU32::new(127),
            seek_tick: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            muted_channels: AtomicU32::new(0),
            current_bpm_x100: AtomicU32::new(12000), // 120.00 BPM
            time_sig_num: AtomicU32::new(4),
            time_sig_den: AtomicU32::new(2), // 2^2 = 4
            track_info: Mutex::new(Vec::new()),
            channel_states: ChannelStates::new(),
            drum_channels: AtomicU32::new(1 << 9),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn toggle_play(&self) {
        let was_playing = self.playing.load(Ordering::Relaxed);
        self.playing.store(!was_playing, Ordering::Relaxed);
        if !was_playing {
            self.stopped.store(false, Ordering::Relaxed);
            self.finished.store(false, Ordering::Relaxed);
        }
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::Relaxed);
        self.stopped.store(true, Ordering::Relaxed);
        self.seek_tick.store(1, Ordering::Relaxed); // Seek to beginning (tick 0 means no seek, so use 1 as special marker handled in sequencer)
        self.current_tick.store(0, Ordering::Relaxed);
    }

    pub fn request_seek(&self, tick: u64) {
        // We use tick+1 internally to distinguish from "no seek" (0).
        // The sequencer will subtract 1.
        self.seek_tick.store(tick + 1, Ordering::Relaxed);
    }

    pub fn get_volume_f32(&self) -> f32 {
        let app_vol = self.volume.load(Ordering::Relaxed) as f32 / 100.0;
        let master_vol = self.master_volume.load(Ordering::Relaxed) as f32 / 127.0;
        app_vol * master_vol
    }

    #[allow(dead_code)]
    pub fn is_channel_muted(&self, channel: u8) -> bool {
        let mask = self.muted_channels.load(Ordering::Relaxed);
        mask & (1 << channel) != 0
    }

    pub fn toggle_channel_mute(&self, channel: u8) {
        let bit = 1u32 << channel;
        let _ = self
            .muted_channels
            .fetch_xor(bit, Ordering::Relaxed);
    }
}
