//! Application state: bridges shared audio state with UI.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::config::Config;
use ump_playback::midi::event::{MidiData, NoteRect};
use ump_playback::midi::parser::parse_midi;
use ump_playback::midi::tempo_map::TempoMap;
use ump_playback::sequencer::Sequencer;
use crate::state::{SharedState, TrackInfoSnapshot};
use crate::synth::audio::AudioOutput;
use ump_playback::synth::engine::SynthPool;
use crate::ui::bars::BarMap;
use crate::ui::file_browser::FileBrowser;
use crate::ui::hit::{HitAction, HitMap};
use crate::ui::track_list::raw_row_count;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    FileBrowser,
    Player,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    TrackList,
    PianoRoll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackViewMode {
    Default,
    Detail,
}

/// Direction notes travel through the vertical piano roll. The playhead is
/// a fixed line 75% of the way down the note area in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalFlow {
    /// Notes fall from the top toward a keyboard at the bottom (default).
    Down,
    /// Notes rise from the bottom toward a keyboard at the top (tracker style).
    Up,
}

impl VerticalFlow {
    /// Parse the `display.piano_roll_flow` config value. Unknown or missing
    /// values fall back to `Down`.
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("up") => VerticalFlow::Up,
            _ => VerticalFlow::Down,
        }
    }

    /// Serialize back to the `display.piano_roll_flow` config value.
    pub fn as_config_str(&self) -> &'static str {
        match self {
            VerticalFlow::Down => "down",
            VerticalFlow::Up => "up",
        }
    }
}

pub struct App {
    pub shared: Arc<SharedState>,
    pub sequencer: Arc<Mutex<Sequencer>>,
    pub synth: Arc<Mutex<Option<SynthPool>>>,
    pub audio: Option<AudioOutput>,
    pub tempo_map: TempoMap,

    // Static metadata
    pub file_name: String,
    pub sf2_name: String,
    pub format: u8,
    pub ticks_per_quarter: u16,
    pub total_notes: usize,
    pub track_count: usize,
    pub total_ticks: u64,
    pub total_duration_secs: f64,
    pub midi_mode: String,

    // Note data for piano roll
    pub note_rects: Vec<NoteRect>,
    /// Bar/beat map built from the song's time-signature events.
    pub bar_map: BarMap,
    /// Piano roll key-range scroll offset (in semitones, relative to the
    /// centered default), adjusted by `scroll_keys`. Only takes effect when
    /// the full key range doesn't fit the available space; the render pass
    /// clamps it to the valid range for the current geometry every frame and
    /// writes the clamped (effective) value back here, so overshoot never
    /// accumulates invisibly and reversing the wheel takes effect immediately.
    pub key_scroll: i32,
    /// Bitfield of channels that have at least one NoteOn event (port*16+ch).
    pub used_channels: u64,
    /// Number of MIDI ports (1-4).
    pub port_count: u8,
    /// Currently displayed port in Default mode (0-based).
    pub current_port: u8,

    // UI state
    pub screen: AppScreen,
    pub focus: FocusPanel,
    pub track_cursor: usize,
    pub piano_roll_vertical: bool,
    pub piano_roll_flow: VerticalFlow,
    pub show_help: bool,
    pub track_view_mode: TrackViewMode,
    pub zoom_level: f64,
    pub seek_step_secs: f64,

    // File browser
    pub file_browser: Option<FileBrowser>,
    /// Session-only: last directory opened in file browser (not persisted).
    pub last_browser_dir: Option<PathBuf>,

    // Paths
    pub midi_file_path: Option<String>,
    pub sf2_file_path: Option<String>,
    pub sample_rate: u32,

    // Scroll state
    /// Timestamp of last MIDI load (for header scroll animation).
    pub load_time: Instant,

    // Config
    pub config: Config,

    // Mouse hit-region map, populated each frame by the player screen.
    pub hit_map: HitMap,
    /// Action currently under the cursor, updated on `CursorMoved`. Drawn as
    /// hover styling by `transport.rs` / `fkey_bar.rs`.
    pub hover: Option<HitAction>,

    // Level meters
    /// Smoothed per-flat-channel level (0.0-1.0), decays when no note is active.
    pub channel_levels: [f32; 64],
    /// Timestamp of the last `update_channel_levels` call, for frame-rate-independent decay.
    pub last_level_update: Instant,
}

impl App {
    /// Create a fully-loaded app (MIDI + SF2 already available).
    #[allow(clippy::too_many_arguments)]
    pub fn new_loaded(
        shared: Arc<SharedState>,
        sequencer: Arc<Mutex<Sequencer>>,
        synth: Arc<Mutex<Option<SynthPool>>>,
        midi_data: &MidiData,
        tempo_map: TempoMap,
        file_name: String,
        sf2_name: String,
        sample_rate: u32,
        config: Config,
    ) -> Self {
        let total_duration_secs = tempo_map.total_duration_secs(midi_data.total_ticks);
        // Apply saved volume
        if let Some(vol) = config.audio.volume {
            shared.volume.store(vol.min(100), Ordering::Relaxed);
        }
        let piano_roll_vertical = config.display.piano_roll_vertical.unwrap_or(false);
        let piano_roll_flow = VerticalFlow::from_config(config.display.piano_roll_flow.as_deref());
        let track_view_mode = match config.display.track_view_mode.as_deref() {
            Some("Detail") => TrackViewMode::Detail,
            _ => TrackViewMode::Default,
        };

        let port_count = midi_data.port_count;
        shared.port_count.store(port_count as u32, Ordering::Relaxed);
        shared.init_drum_channels(port_count);

        App {
            shared,
            sequencer,
            synth,
            audio: None,
            tempo_map,
            file_name,
            sf2_name,
            format: midi_data.format,
            ticks_per_quarter: midi_data.ticks_per_quarter,
            total_notes: midi_data.note_rects.len(),
            track_count: midi_data.tracks.len(),
            total_ticks: midi_data.total_ticks,
            total_duration_secs,
            midi_mode: "GM".to_string(),
            note_rects: midi_data.note_rects.clone(),
            bar_map: BarMap::build(midi_data.ticks_per_quarter, &midi_data.events),
            key_scroll: 0,
            used_channels: midi_data.used_channels,
            port_count,
            current_port: 0,
            screen: AppScreen::Player,
            focus: FocusPanel::TrackList,
            track_cursor: 0,
            piano_roll_vertical,
            piano_roll_flow,
            show_help: false,
            track_view_mode,
            zoom_level: 1.0,
            seek_step_secs: 5.0,
            file_browser: None,
            last_browser_dir: None,
            midi_file_path: None,
            sf2_file_path: None,
            sample_rate,
            load_time: Instant::now(),
            config,
            hit_map: HitMap::default(),
            hover: None,
            channel_levels: [0.0; 64],
            last_level_update: Instant::now(),
        }
    }

    /// Create a minimal app for file browser startup (no MIDI/SF2 loaded yet).
    pub fn new_empty(sample_rate: u32, config: Config) -> Self {
        let shared = Arc::new(SharedState::new());
        let dummy_tempo = TempoMap::new(480, &[]);
        let dummy_seq = Sequencer::new_empty(480, dummy_tempo.clone());
        let sequencer = Arc::new(Mutex::new(dummy_seq));
        let synth = Arc::new(Mutex::new(None));

        // Apply saved volume
        if let Some(vol) = config.audio.volume {
            shared.volume.store(vol.min(100), Ordering::Relaxed);
        }

        let piano_roll_vertical = config.display.piano_roll_vertical.unwrap_or(false);
        let piano_roll_flow = VerticalFlow::from_config(config.display.piano_roll_flow.as_deref());
        let track_view_mode = match config.display.track_view_mode.as_deref() {
            Some("Detail") => TrackViewMode::Detail,
            _ => TrackViewMode::Default,
        };

        App {
            shared,
            sequencer,
            synth,
            audio: None,
            tempo_map: TempoMap::new(480, &[]),
            file_name: String::new(),
            sf2_name: String::new(),
            format: 0,
            ticks_per_quarter: 480,
            total_notes: 0,
            track_count: 0,
            total_ticks: 0,
            total_duration_secs: 0.0,
            midi_mode: String::new(),
            note_rects: Vec::new(),
            bar_map: BarMap::default(),
            key_scroll: 0,
            used_channels: 0,
            port_count: 1,
            current_port: 0,
            screen: AppScreen::FileBrowser,
            focus: FocusPanel::TrackList,
            track_cursor: 0,
            piano_roll_vertical,
            piano_roll_flow,
            show_help: false,
            track_view_mode,
            zoom_level: 1.0,
            seek_step_secs: 5.0,
            file_browser: None,
            last_browser_dir: None,
            midi_file_path: None,
            sf2_file_path: None,
            sample_rate,
            load_time: Instant::now(),
            config,
            hit_map: HitMap::default(),
            hover: None,
            channel_levels: [0.0; 64],
            last_level_update: Instant::now(),
        }
    }

    /// Reload MIDI file: parse, replace sequencer, reset shared state.
    pub fn reload_midi(&mut self, path: &str) -> Result<()> {
        let midi_bytes =
            std::fs::read(path).with_context(|| format!("Failed to read MIDI file: {}", path))?;
        let (midi_data, tempo_map) = parse_midi(&midi_bytes)?;

        log_info!(
            "MIDI: file={}, format={}, tracks={}, notes={}, ticks={}, ports={}",
            path, midi_data.format, midi_data.tracks.len(),
            midi_data.note_rects.len(), midi_data.total_ticks, midi_data.port_count
        );

        // Detect mode
        let detected_mode = ump_playback::midi::mode_detect::detect_mode(&midi_data.events);

        let port_count = midi_data.port_count;

        // Replace sequencer
        let new_seq = Sequencer::new(&midi_data, tempo_map.clone());
        {
            let mut seq = self.sequencer.lock().unwrap();
            *seq = new_seq;
        }

        // Reset synth — recreate with correct port count
        {
            let mut syn = self.synth.lock().unwrap();
            if let Some(ref mut s) = *syn {
                s.reset();
            }
        }

        // Reset shared state
        self.shared.current_tick.store(0, Ordering::Relaxed);
        self.shared.playing.store(false, Ordering::Relaxed);
        self.shared.stopped.store(false, Ordering::Relaxed);
        self.shared.finished.store(false, Ordering::Relaxed);
        self.shared.seek_tick.store(0, Ordering::Relaxed);
        self.shared.muted_channels.store(0, Ordering::Relaxed);
        self.shared.channel_states.reset();
        self.shared.port_count.store(port_count as u32, Ordering::Relaxed);
        self.shared.init_drum_channels(port_count);
        self.shared.master_volume.store(127, Ordering::Relaxed);
        self.shared
            .current_bpm_x100
            .store(12000, Ordering::Relaxed);
        self.shared.time_sig_num.store(4, Ordering::Relaxed);
        self.shared.time_sig_den.store(2, Ordering::Relaxed);

        // Populate track info
        {
            let mut info = self.shared.track_info.lock().unwrap();
            *info = midi_data
                .tracks
                .iter()
                .map(|t| TrackInfoSnapshot {
                    index: t.index,
                    name: t.name.clone(),
                    channel: t.channel,
                    port: t.port,
                    program: t.program,
                    note_count: t.note_count,
                    channel_note_counts: t.channel_note_counts,
                    channel_programs: t.channel_programs,
                })
                .collect();
        }

        // Update app metadata
        let file_name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());

        self.file_name = file_name;
        self.format = midi_data.format;
        self.ticks_per_quarter = midi_data.ticks_per_quarter;
        self.total_notes = midi_data.note_rects.len();
        self.track_count = midi_data.tracks.len();
        self.total_ticks = midi_data.total_ticks;
        self.total_duration_secs = tempo_map.total_duration_secs(midi_data.total_ticks);
        self.tempo_map = tempo_map;
        self.bar_map = BarMap::build(midi_data.ticks_per_quarter, &midi_data.events);
        self.key_scroll = 0;
        self.note_rects = midi_data.note_rects;
        self.used_channels = midi_data.used_channels;
        self.port_count = port_count;
        self.current_port = 0;
        self.midi_mode = detected_mode.to_string();
        self.track_cursor = 0;
        self.load_time = Instant::now();
        self.midi_file_path = Some(path.to_string());

        // Load mode-specific soundfont bundle, or restore default SF2
        let mode_str = detected_mode.to_string();
        if let Some(bundle) = self.config.soundfont.resolve_bundle(&mode_str).cloned() {
            if let Err(e) = self.reload_bundle(&bundle) {
                log_warn!("Failed to load bundle for detected mode {}: {:#}", mode_str, e);
            }
        } else {
            self.restore_default_sf2();
        }

        Ok(())
    }

    /// Reload SF2 file: replace synth engine with multi-port support.
    pub fn reload_sf2(&mut self, path: &str) -> Result<()> {
        let sf2_bytes =
            std::fs::read(path).with_context(|| format!("Failed to read SF2 file: {}", path))?;
        log_info!("SF2: file={}, size={} bytes", path, sf2_bytes.len());
        let new_synth = SynthPool::single(&sf2_bytes, self.sample_rate, self.port_count)?;

        {
            let mut syn = self.synth.lock().unwrap();
            *syn = Some(new_synth);
        }

        let sf2_name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());

        self.sf2_name = sf2_name;
        self.sf2_file_path = Some(path.to_string());
        log_info!("SF2 loaded: {}", self.sf2_name);

        // Update config with absolute path
        self.config.soundfont.recent_path = Some(crate::config::to_absolute_path(path));
        self.config.save();

        Ok(())
    }

    /// Reload synth from a SoundfontBundle (multiple SF2 files with routing).
    pub fn reload_bundle(&mut self, bundle: &crate::config::SoundfontBundle) -> Result<()> {
        let config_dir = crate::config::Config::config_path();
        let base_dir = config_dir.as_deref().and_then(|p| p.parent());

        let mut sf2_data_list: Vec<Vec<u8>> = Vec::new();
        let mut names = Vec::new();
        for path in bundle.resolved_files(base_dir) {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("Failed to read SF2 file: {}", path.display()))?;
            log_info!("SF2 bundle: file={}, size={} bytes", path.display(), bytes.len());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            names.push(name);
            sf2_data_list.push(bytes);
        }

        let routing = bundle.routing_table();
        let refs: Vec<&[u8]> = sf2_data_list.iter().map(|v| v.as_slice()).collect();
        let new_pool = SynthPool::new_bundle(&refs, routing, self.sample_rate, self.port_count)?;

        {
            let mut syn = self.synth.lock().unwrap();
            *syn = Some(new_pool);
        }

        self.sf2_name = names.join(" + ");
        log_info!("SF2 loaded (bundle): {}", self.sf2_name);

        Ok(())
    }

    /// Restore default (single) SF2 if a bundle was previously loaded.
    fn restore_default_sf2(&mut self) {
        let default_name = self
            .sf2_file_path
            .as_ref()
            .and_then(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            })
            .unwrap_or_default();

        if default_name.is_empty() || self.sf2_name == default_name {
            return;
        }

        // Bundle was previously loaded — restore default SF2
        let path = self.sf2_file_path.clone().unwrap();
        log_info!("SF2 restoring default: {}", path);
        if let Err(e) = self.reload_sf2(&path) {
            log_warn!("Failed to restore default SF2: {:#}", e);
        }
    }

    /// Start audio output if not already running.
    pub fn ensure_audio(&mut self) -> Result<()> {
        if self.audio.is_some() {
            return Ok(());
        }
        let audio = AudioOutput::start(
            self.sequencer.clone(),
            self.synth.clone(),
            self.shared.clone(),
            self.sample_rate,
        )?;
        self.audio = Some(audio);
        Ok(())
    }

    /// Start playback.
    pub fn start_playback(&self) {
        self.shared
            .playing
            .store(true, Ordering::Relaxed);
    }

    /// Check if a MIDI file is loaded.
    pub fn has_midi(&self) -> bool {
        self.midi_file_path.is_some()
    }

    /// Check if an SF2 file is loaded.
    pub fn has_sf2(&self) -> bool {
        self.sf2_file_path.is_some()
    }

    pub fn current_tick(&self) -> u64 {
        self.shared.current_tick.load(Ordering::Relaxed)
    }

    pub fn current_time_secs(&self) -> f64 {
        self.tempo_map.tick_to_secs(self.current_tick())
    }

    pub fn current_bpm(&self) -> f64 {
        self.shared.current_bpm_x100.load(Ordering::Relaxed) as f64 / 100.0
    }

    pub fn time_signature(&self) -> (u32, u32) {
        let num = self.shared.time_sig_num.load(Ordering::Relaxed);
        let den = self.shared.time_sig_den.load(Ordering::Relaxed);
        (num, den)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.is_playing()
    }

    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Relaxed)
    }

    pub fn volume(&self) -> u32 {
        self.shared.volume.load(Ordering::Relaxed)
    }

    pub fn toggle_play(&self) {
        self.shared.toggle_play();
    }

    pub fn stop(&self) {
        self.shared.stop();
    }

    pub fn seek_forward(&self) {
        let current_secs = self.current_time_secs();
        let target_secs = (current_secs + self.seek_step_secs).min(self.total_duration_secs);
        let target_tick = self.tempo_map.secs_to_tick(target_secs);
        self.shared.request_seek(target_tick);
    }

    pub fn seek_backward(&self) {
        let current_secs = self.current_time_secs();
        let target_secs = (current_secs - self.seek_step_secs).max(0.0);
        let target_tick = self.tempo_map.secs_to_tick(target_secs);
        self.shared.request_seek(target_tick);
    }

    /// Seek to an absolute tick (e.g. a piano-roll ruler click), clamped to the song's length.
    pub fn seek_to_tick(&self, tick: u64) {
        self.shared.request_seek(tick.min(self.total_ticks));
    }

    /// Set the volume directly (e.g. from a clicked volume block), clamped 0-100.
    pub fn set_volume(&self, v: u32) {
        self.shared.volume.store(v.min(100), Ordering::Relaxed);
    }

    pub fn volume_up(&self) {
        let v = self.shared.volume.load(Ordering::Relaxed);
        self.shared.volume.store((v + 5).min(100), Ordering::Relaxed);
    }

    pub fn volume_down(&self) {
        let v = self.shared.volume.load(Ordering::Relaxed);
        self.shared
            .volume
            .store(v.saturating_sub(5), Ordering::Relaxed);
    }

    pub fn move_cursor_up(&mut self) {
        if self.track_cursor > 0 {
            self.track_cursor -= 1;
        }
    }

    pub fn move_cursor_down(&mut self) {
        let tracks = self.shared.track_info.lock().unwrap();
        let max = raw_row_count(&tracks, self.port_count, self.current_port, self.track_view_mode);
        if self.track_cursor + 1 < max {
            self.track_cursor += 1;
        }
    }

    pub fn toggle_mute_selected(&self) {
        self.toggle_mute_row(self.track_cursor);
    }

    /// Toggle mute for the channel backing raw row `row`. No-op on `TrackHeader` rows.
    pub fn toggle_mute_row(&self, row: usize) {
        use crate::ui::track_list::RawRow;
        let tracks = self.shared.track_info.lock().unwrap();
        let rows = crate::ui::track_list::build_raw_rows(&tracks, self.port_count, self.current_port, self.track_view_mode);
        if let Some(RawRow::Channel { port, channel, .. }) = rows.get(row) {
            self.shared.toggle_channel_mute(*port, *channel);
        }
        // TrackHeader 行では何もしない
    }

    /// Solo the channel backing raw row `row` among currently used channels:
    /// muting every other used channel, or clearing all mutes if it is already
    /// the sole unmuted channel. No-op on `TrackHeader` rows.
    pub fn toggle_solo_row(&self, row: usize) {
        use crate::ui::track_list::RawRow;
        let tracks = self.shared.track_info.lock().unwrap();
        let rows = crate::ui::track_list::build_raw_rows(&tracks, self.port_count, self.current_port, self.track_view_mode);
        if let Some(RawRow::Channel { port, channel, .. }) = rows.get(row) {
            let flat = *port as u64 * 16 + *channel as u64;
            let muted = self.shared.muted_channels.load(Ordering::Relaxed);
            let new_mask = solo_mask(self.used_channels, muted, flat);
            self.shared.set_muted_channels(new_mask);
        }
    }

    /// Select raw row `row`, clamped to the current row count.
    pub fn select_track_row(&mut self, row: usize) {
        let tracks = self.shared.track_info.lock().unwrap();
        let max = raw_row_count(&tracks, self.port_count, self.current_port, self.track_view_mode);
        drop(tracks);
        if max == 0 {
            self.track_cursor = 0;
        } else {
            self.track_cursor = row.min(max - 1);
        }
    }

    /// Recompute smoothed per-channel meter levels; call once per frame.
    pub fn update_channel_levels(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_level_update).as_secs_f32().min(0.1);
        self.last_level_update = now;

        let targets = level_targets(
            &self.note_rects,
            self.current_tick(),
            self.total_ticks,
            self.is_playing(),
        );
        for (level, &target) in self.channel_levels.iter_mut().zip(targets.iter()) {
            *level = if target > *level {
                target
            } else {
                target.max(*level - dt * 1.8)
            };
        }
    }

    pub fn toggle_piano_roll_orientation(&mut self) {
        self.piano_roll_vertical = !self.piano_roll_vertical;
    }

    pub fn toggle_piano_roll_flow(&mut self) {
        self.piano_roll_flow = match self.piano_roll_flow {
            VerticalFlow::Down => VerticalFlow::Up,
            VerticalFlow::Up => VerticalFlow::Down,
        };
    }

    pub fn zoom_in(&mut self) {
        self.zoom_level = (self.zoom_level * 1.25).min(8.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_level = (self.zoom_level / 1.25).max(0.25);
    }

    /// Adjust the piano roll's key-range scroll offset (positive = toward
    /// higher pitches). The valid range depends on the current key-range
    /// geometry, which only the render pass knows, so this just bounds the
    /// raw offset to more than any real key range (127 semitones) and lets
    /// `visible_key_range` clamp it precisely every frame.
    pub fn scroll_keys(&mut self, delta_keys: i32) {
        self.key_scroll = (self.key_scroll + delta_keys).clamp(-127, 127);
    }

    pub fn next_port(&mut self) {
        if self.port_count > 1 && self.current_port + 1 < self.port_count {
            self.current_port += 1;
            self.track_cursor = 0;
        }
    }

    pub fn prev_port(&mut self) {
        if self.current_port > 0 {
            self.current_port -= 1;
            self.track_cursor = 0;
        }
    }

    /// Switch to port `p` directly (e.g. from a clicked port tab). No-op if
    /// `p` is out of range or already current.
    pub fn set_port(&mut self, p: u8) {
        if p < self.port_count && p != self.current_port {
            self.current_port = p;
            self.track_cursor = 0;
        }
    }

    pub fn set_midi_mode(&mut self, mode: &str) {
        self.midi_mode = mode.to_string();

        // Check if a bundle is configured for this mode
        if let Some(bundle) = self.config.soundfont.resolve_bundle(mode).cloned() {
            match self.reload_bundle(&bundle) {
                Ok(()) => {
                    log_info!("Loaded bundle for mode: {}", mode);
                }
                Err(e) => {
                    log_error!("Failed to load bundle for mode {}: {:#}", mode, e);
                    // Fall back to standard reset
                }
            }
        } else {
            self.restore_default_sf2();
        }

        // Reset synth to clean state for the new mode
        if let Ok(mut guard) = self.synth.lock() {
            if let Some(ref mut synth) = *guard {
                synth.system_reset();
            }
        }

        // Reset shared channel state
        self.shared.channel_states.reset();
        self.shared.init_drum_channels(self.port_count);
        self.shared.master_volume.store(127, Ordering::Relaxed);

        // Re-seek to current position to replay all state-changing events
        // (Bank Select, Program Change, SysEx) from the beginning
        let current = self.current_tick();
        self.shared.request_seek(current);
    }

    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            FocusPanel::TrackList => FocusPanel::PianoRoll,
            FocusPanel::PianoRoll => FocusPanel::TrackList,
        };
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        if self.show_help {
            // No player hit region is active while help is shown; drop any
            // stale hover so `transport.rs`/`fkey_bar.rs` don't keep drawing
            // a highlight for an action that's no longer reachable.
            self.hover = None;
        }
    }

    pub fn toggle_track_view_mode(&mut self) {
        self.track_view_mode = match self.track_view_mode {
            TrackViewMode::Default => TrackViewMode::Detail,
            TrackViewMode::Detail => TrackViewMode::Default,
        };
    }

    /// Reset SF2 to default_path from config. Returns Ok(true) if reset, Ok(false) if no default.
    pub fn reset_sf2_to_default(&mut self) -> Result<bool> {
        let default = self
            .config
            .soundfont
            .default_path
            .as_deref()
            .map(crate::config::resolve_path);
        if let Some(path) = default {
            self.reload_sf2(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Save current state to config (call on exit).
    pub fn save_config(&mut self) {
        self.config.audio.volume = Some(self.volume());
        self.config.display.piano_roll_vertical = Some(self.piano_roll_vertical);
        self.config.display.piano_roll_flow = Some(self.piano_roll_flow.as_config_str().to_string());
        self.config.display.track_view_mode = Some(
            match self.track_view_mode {
                TrackViewMode::Default => "Default",
                TrackViewMode::Detail => "Detail",
            }
            .to_string(),
        );
        self.config.save();
    }
}

/// Compute the new mute mask for soloing flat channel `flat` among `used`
/// channels: mute every other used channel, unless `flat` is already the
/// sole unmuted used channel, in which case clear all mutes. Soloing a
/// channel that isn't in `used` (e.g. an empty track row) leaves `muted`
/// unchanged instead of muting every used channel.
fn solo_mask(used: u64, muted: u64, flat: u64) -> u64 {
    let bit = 1u64 << flat;
    if used & bit == 0 {
        return muted;
    }
    let others = used & !bit;
    let already_soloed = muted & others == others && muted & bit == 0;
    if already_soloed {
        0
    } else {
        others
    }
}

/// Compute the meter target level (0.0-1.0) for each flat channel (port*16+channel):
/// the max velocity/127 among notes covering `tick`, or all zero when not playing.
fn level_targets(notes: &[NoteRect], tick: u64, total_ticks: u64, playing: bool) -> [f32; 64] {
    let mut targets = [0.0f32; 64];
    if !playing {
        return targets;
    }

    // Same scan-window technique as piano_roll.rs: skip notes far before `tick`.
    let max_dur = total_ticks / 4;
    let scan_start = if tick > max_dur {
        notes.partition_point(|n| n.start_tick < tick - max_dur)
    } else {
        0
    };

    for note in &notes[scan_start..] {
        if note.start_tick > tick {
            break;
        }
        if note.end_tick <= tick {
            continue;
        }
        let flat = note.port as usize * 16 + note.channel as usize;
        if flat >= 64 {
            continue;
        }
        let v = note.velocity as f32 / 127.0;
        if v > targets[flat] {
            targets[flat] = v;
        }
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(port: u8, channel: u8, start: u64, end: u64, velocity: u8) -> NoteRect {
        NoteRect {
            key: 60,
            channel,
            port,
            start_tick: start,
            end_tick: end,
            velocity,
            track: 0,
        }
    }

    #[test]
    fn solo_mask_mutes_all_other_used_channels() {
        let used = 0b1011; // channels 0, 1, 3
        let muted = 0;
        assert_eq!(solo_mask(used, muted, 0), 0b1010); // channels 1, 3 muted
    }

    #[test]
    fn solo_mask_clears_when_already_sole_unmuted() {
        let used = 0b1011; // channels 0, 1, 3
        let muted = 0b1010; // 1 and 3 muted, 0 unmuted
        assert_eq!(solo_mask(used, muted, 0), 0);
    }

    #[test]
    fn solo_mask_single_used_channel_clears() {
        let used = 0b0001;
        let muted = 0;
        // No other used channels to mute; already the sole unmuted channel.
        assert_eq!(solo_mask(used, muted, 0), 0);
    }

    #[test]
    fn solo_mask_ignores_bits_outside_used() {
        let used = 0b0011; // channels 0, 1
        let muted = 0b0100; // channel 2 muted but not "used"
        assert_eq!(solo_mask(used, muted, 0), 0b0010);
    }

    #[test]
    fn solo_mask_on_unused_channel_leaves_mute_state_unchanged() {
        let used = 0b1011; // channels 0, 1, 3
        let muted = 0b0010; // channel 1 already muted
        // Channel 2 isn't used: soloing it must not touch any mute state.
        assert_eq!(solo_mask(used, muted, 2), muted);
    }

    #[test]
    fn toggle_help_clears_stale_hover_when_opening() {
        let mut app = App::new_empty(44100, Config::default());
        app.hover = Some(HitAction::PlayPause);
        app.toggle_help();
        assert!(app.show_help);
        assert_eq!(app.hover, None);
    }

    #[test]
    fn level_targets_zero_when_not_playing() {
        let notes = [note(0, 0, 0, 10, 127)];
        let targets = level_targets(&notes, 5, 100, false);
        assert_eq!(targets, [0.0; 64]);
    }

    #[test]
    fn level_targets_picks_max_velocity_of_overlapping_notes() {
        let notes = [
            note(0, 2, 0, 10, 64),
            note(0, 2, 2, 8, 127),
        ];
        let targets = level_targets(&notes, 5, 100, true);
        assert_eq!(targets[2], 1.0);
    }

    #[test]
    fn level_targets_excludes_notes_outside_start_end_window() {
        let notes = [note(0, 0, 0, 5, 127)];
        // end_tick is exclusive: tick == end_tick should not count.
        let targets = level_targets(&notes, 5, 100, true);
        assert_eq!(targets[0], 0.0);
    }

    #[test]
    fn vertical_flow_from_config_parses_known_values() {
        assert_eq!(VerticalFlow::from_config(Some("down")), VerticalFlow::Down);
        assert_eq!(VerticalFlow::from_config(Some("up")), VerticalFlow::Up);
    }

    #[test]
    fn vertical_flow_from_config_defaults_to_down() {
        assert_eq!(VerticalFlow::from_config(None), VerticalFlow::Down);
        assert_eq!(VerticalFlow::from_config(Some("sideways")), VerticalFlow::Down);
    }

    #[test]
    fn level_targets_applies_port_offset() {
        let notes = [note(1, 0, 0, 10, 127)];
        let targets = level_targets(&notes, 5, 100, true);
        assert_eq!(targets[16], 1.0);
        assert_eq!(targets[0], 0.0);
    }
}
