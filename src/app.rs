//! Application state: bridges shared audio state with UI.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::midi::event::{MidiData, NoteRect};
use crate::midi::parser::parse_midi;
use crate::midi::tempo_map::TempoMap;
use crate::sequencer::Sequencer;
use crate::state::{SharedState, TrackInfoSnapshot};
use crate::synth::audio::AudioOutput;
use crate::synth::engine::SynthPool;
use crate::ui::file_browser::FileBrowser;
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
    pub show_piano_roll: bool,
    pub piano_roll_vertical: bool,
    pub midi_monitor: bool,
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

    // Config
    pub config: Config,
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
        let show_piano_roll = config.display.show_piano_roll.unwrap_or(true);
        let piano_roll_vertical = config.display.piano_roll_vertical.unwrap_or(false);
        let midi_monitor = config.display.midi_monitor.unwrap_or(false);
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
            used_channels: midi_data.used_channels,
            port_count,
            current_port: 0,
            screen: AppScreen::Player,
            focus: FocusPanel::TrackList,
            track_cursor: 0,
            show_piano_roll,
            piano_roll_vertical,
            midi_monitor,
            show_help: false,
            track_view_mode,
            zoom_level: 1.0,
            seek_step_secs: 5.0,
            file_browser: None,
            last_browser_dir: None,
            midi_file_path: None,
            sf2_file_path: None,
            sample_rate,
            config,
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

        let show_piano_roll = config.display.show_piano_roll.unwrap_or(true);
        let piano_roll_vertical = config.display.piano_roll_vertical.unwrap_or(false);
        let midi_monitor = config.display.midi_monitor.unwrap_or(false);
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
            used_channels: 0,
            port_count: 1,
            current_port: 0,
            screen: AppScreen::FileBrowser,
            focus: FocusPanel::TrackList,
            track_cursor: 0,
            show_piano_roll,
            piano_roll_vertical,
            midi_monitor,
            show_help: false,
            track_view_mode,
            zoom_level: 1.0,
            seek_step_secs: 5.0,
            file_browser: None,
            last_browser_dir: None,
            midi_file_path: None,
            sf2_file_path: None,
            sample_rate,
            config,
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
        let detected_mode = crate::midi::mode_detect::detect_mode(&midi_data.events);

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
        self.note_rects = midi_data.note_rects;
        self.used_channels = midi_data.used_channels;
        self.port_count = port_count;
        self.current_port = 0;
        self.midi_mode = detected_mode.to_string();
        self.track_cursor = 0;
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
        // Load all SF2 files
        let mut sf2_data_list: Vec<Vec<u8>> = Vec::new();
        let mut names = Vec::new();
        for file_path in &bundle.files {
            let resolved = crate::config::resolve_path(file_path);
            let bytes = std::fs::read(&resolved)
                .with_context(|| format!("Failed to read SF2 file: {}", resolved))?;
            log_info!("SF2 bundle: file={}, size={} bytes", resolved, bytes.len());
            let name = std::path::Path::new(&resolved)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| resolved.clone());
            names.push(name);
            sf2_data_list.push(bytes);
        }

        // Build routing table
        let mut routing = [0usize; 16];
        if let Some(ref route_vec) = bundle.routing {
            for (ch, &idx) in route_vec.iter().enumerate() {
                if ch < 16 && (idx as usize) < sf2_data_list.len() {
                    routing[ch] = idx as usize;
                }
            }
        }

        let refs: Vec<&[u8]> = sf2_data_list.iter().map(|v| v.as_slice()).collect();
        let new_pool = if self.port_count > 1 {
            SynthPool::new_bundle(&refs, routing, self.sample_rate, self.port_count)?
        } else {
            SynthPool::new(&refs, routing, self.sample_rate)?
        };

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
        use crate::ui::track_list::RawRow;
        let tracks = self.shared.track_info.lock().unwrap();
        let rows = crate::ui::track_list::build_raw_rows(&tracks, self.port_count, self.current_port, self.track_view_mode);
        if let Some(RawRow::Channel { port, channel, .. }) = rows.get(self.track_cursor) {
            self.shared.toggle_channel_mute(*port, *channel);
        }
        // TrackHeader 行では何もしない
    }

    pub fn toggle_piano_roll(&mut self) {
        self.show_piano_roll = !self.show_piano_roll;
    }

    pub fn toggle_piano_roll_orientation(&mut self) {
        self.piano_roll_vertical = !self.piano_roll_vertical;
    }

    pub fn zoom_in(&mut self) {
        self.zoom_level = (self.zoom_level * 1.25).min(8.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_level = (self.zoom_level / 1.25).max(0.25);
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
        self.config.display.show_piano_roll = Some(self.show_piano_roll);
        self.config.display.piano_roll_vertical = Some(self.piano_roll_vertical);
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
