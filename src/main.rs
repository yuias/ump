#![windows_subsystem = "windows"]

#[cfg(not(feature = "wgpu-backend"))]
compile_error!("Feature \"wgpu-backend\" must be enabled");

#[macro_use]
mod debug;
mod app;
mod cli;
mod config;
mod renderer;
mod state;
mod synth;
mod ui;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, AppScreen};
use crate::cli::Args;
use crate::config::Config;
use ump_playback::midi::parser::parse_midi;
#[cfg(feature = "wgpu-backend")]
use crate::renderer::wgpu_backend::WgpuRenderer;
use crate::renderer::Renderer;
use crate::ui::layout::{px, ROW_HEIGHT};
use crate::ui::theme;
use ump_playback::sequencer::Sequencer;
use crate::state::{SharedState, TrackInfoSnapshot};
use crate::synth::audio::{query_sample_rate, AudioOutput};
use ump_playback::synth::engine::SynthPool;
use crate::ui::file_browser::{BrowseTarget, FileBrowser};
use crate::ui::input::{handle_winit_input, InputResult};
use crate::ui::mouse::MouseState;
use crate::ui::render::render;

/// Frame intervals by state.
const FRAME_PIANO_ROLL: Duration = Duration::from_millis(16); // ~60fps
const FRAME_IDLE: Duration = Duration::from_millis(200);

struct UmpApp {
    window: Option<Arc<Window>>,
    #[cfg(feature = "wgpu-backend")]
    renderer: Option<WgpuRenderer>,
    app: Option<App>,
    config: Config,
    args: Args,
    sample_rate: u32,
    last_draw: Instant,
    needs_draw: bool,
    initialized: bool,
    modifiers: ModifiersState,
    mouse: MouseState,
}

impl UmpApp {
    fn new(args: Args, config: Config, sample_rate: u32) -> Self {
        UmpApp {
            window: None,
            renderer: None,
            app: None,
            config,
            args,
            sample_rate,
            last_draw: Instant::now(),
            needs_draw: true,
            initialized: false,
            modifiers: ModifiersState::default(),
            mouse: MouseState::default(),
        }
    }

    /// Redraw immediately after a handled mouse action, mirroring the
    /// keyboard path's `InputResult::Handled` behavior.
    fn mark_input_handled(&mut self) {
        if let Some(ref window) = self.window {
            window.request_redraw();
            self.last_draw = Instant::now();
            self.needs_draw = false;
        }
    }

    fn frame_interval(&self) -> Duration {
        match &self.app {
            Some(app) => {
                if app.is_playing() {
                    FRAME_PIANO_ROLL
                } else {
                    FRAME_IDLE
                }
            }
            None => FRAME_IDLE,
        }
    }

    fn initialize_app(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;

        let sf2_path = self
            .args
            .sf2_file
            .clone()
            .or_else(|| self.config.resolve_sf2());
        let midi_path = self.args.midi_file.clone();

        if let (Some(midi_p), Some(sf2_p)) = (&midi_path, &sf2_path) {
            match self.full_load(midi_p, sf2_p) {
                Ok(()) => {}
                Err(e) => {
                    log_error!("Failed to load: {:#}", e);
                    self.browser_start(midi_path, sf2_path);
                }
            }
        } else {
            self.browser_start(midi_path, sf2_path);
        }
    }

    fn full_load(&mut self, midi_path: &str, sf2_path: &str) -> Result<()> {
        let midi_bytes = std::fs::read(midi_path)
            .with_context(|| format!("Failed to read MIDI file: {}", midi_path))?;
        let sf2_bytes = std::fs::read(sf2_path)
            .with_context(|| format!("Failed to read SF2 file: {}", sf2_path))?;

        log_info!("SF2: file={}, size={} bytes", sf2_path, sf2_bytes.len());

        let (midi_data, tempo_map) = parse_midi(&midi_bytes)?;
        let synth = SynthPool::single(&sf2_bytes, self.sample_rate, midi_data.port_count)?;
        let shared = Arc::new(SharedState::new());

        {
            let mut info = shared.track_info.lock().unwrap();
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

        let seq = Sequencer::new(&midi_data, tempo_map.clone());
        let seq = Arc::new(Mutex::new(seq));
        let synth = Arc::new(Mutex::new(Some(synth)));

        let file_name = std::path::Path::new(midi_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| midi_path.to_string());
        let sf2_name = std::path::Path::new(sf2_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| sf2_path.to_string());

        let detected_mode = ump_playback::midi::mode_detect::detect_mode(&midi_data.events);

        self.config.soundfont.recent_path = Some(config::to_absolute_path(sf2_path));
        self.config.save();

        let mut app = App::new_loaded(
            shared.clone(),
            seq.clone(),
            synth.clone(),
            &midi_data,
            tempo_map,
            file_name.clone(),
            sf2_name,
            self.sample_rate,
            self.config.clone(),
        );
        app.midi_mode = detected_mode.to_string();
        app.midi_file_path = Some(midi_path.to_string());
        app.sf2_file_path = Some(sf2_path.to_string());

        // Load mode-specific soundfont bundle if configured
        let mode_str = detected_mode.to_string();
        if let Some(bundle) = app.config.soundfont.resolve_bundle(&mode_str).cloned() {
            if let Err(e) = app.reload_bundle(&bundle) {
                log_warn!("Failed to load bundle for detected mode {}: {:#}", mode_str, e);
            }
        }

        let audio = AudioOutput::start(seq, synth, shared, self.sample_rate)?;
        app.audio = Some(audio);
        app.start_playback();

        // Set window title
        if let Some(ref window) = self.window {
            window.set_title(&format!("ump - {}", file_name));
        }

        self.app = Some(app);
        Ok(())
    }

    fn browser_start(&mut self, midi_path: Option<String>, sf2_path: Option<String>) {
        let mut app = App::new_empty(self.sample_rate, self.config.clone());

        if let Some(ref sf2_p) = sf2_path {
            if let Err(e) = app.reload_sf2(sf2_p) {
                log_warn!("Failed to load SF2: {:#}", e);
            }
        }
        if let Some(ref midi_p) = midi_path {
            if let Err(e) = app.reload_midi(midi_p) {
                log_warn!("Failed to load MIDI: {:#}", e);
            }
        }

        let target = if !app.has_sf2() {
            BrowseTarget::Sf2
        } else {
            BrowseTarget::Midi
        };

        app.file_browser = Some(FileBrowser::new(target, app.last_browser_dir.as_deref()));
        app.screen = AppScreen::FileBrowser;

        if let Some(ref window) = self.window {
            window.set_title("ump");
        }

        self.app = Some(app);
    }
}

impl ApplicationHandler for UmpApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let default_size = winit::dpi::PhysicalSize::new(
            self.config.window.width.unwrap_or(1280),
            self.config.window.height.unwrap_or(720),
        );
        let mut attrs = Window::default_attributes()
            .with_title("ump")
            .with_inner_size(default_size);

        if let (Some(x), Some(y)) = (self.config.window.x, self.config.window.y) {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log_error!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let font_path = self.config.font.path.as_deref().map(crate::config::resolve_path);
        let font_size = self.config.font.size_or_default();

        #[cfg(feature = "wgpu-backend")]
        {
            match WgpuRenderer::new(
                window.clone(),
                size.width,
                size.height,
                font_path.as_deref(),
                font_size,
                window.scale_factor() as f32,
            ) {
                Ok(renderer) => self.renderer = Some(renderer),
                Err(e) => {
                    log_error!("Failed to create wgpu renderer: {}", e);
                    event_loop.exit();
                    return;
                }
            }
        }

        self.window = Some(window);
        self.initialize_app();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(ref mut app) = self.app {
                    if let Some(ref window) = self.window {
                        let size = window.inner_size();
                        app.config.window.width = Some(size.width);
                        app.config.window.height = Some(size.height);
                        if let Ok(pos) = window.outer_position() {
                            app.config.window.x = Some(pos.x);
                            app.config.window.y = Some(pos.y);
                        }
                    }
                    app.save_config();
                }
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(ref mut renderer) = self.renderer {
                    let _ = renderer.resize(size.width, size.height);
                }
                self.needs_draw = true;
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(ref mut renderer) = self.renderer {
                    renderer.set_scale_factor(scale_factor as f32);
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                self.needs_draw = true;
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(ref mut app) = self.app {
                    match handle_winit_input(app, &event, self.modifiers) {
                        InputResult::Quit => {
                            // Save window geometry to app config before persisting
                            if let Some(ref window) = self.window {
                                let size = window.inner_size();
                                app.config.window.width = Some(size.width);
                                app.config.window.height = Some(size.height);
                                if let Ok(pos) = window.outer_position() {
                                    app.config.window.x = Some(pos.x);
                                    app.config.window.y = Some(pos.y);
                                }
                            }
                            app.save_config();
                            event_loop.exit();
                        }
                        InputResult::Handled => {
                            // Immediate redraw on user input (bypass frame interval)
                            if let Some(ref window) = self.window {
                                window.request_redraw();
                                self.last_draw = Instant::now();
                                self.needs_draw = false;
                                // Update window title on file load
                                if app.screen == AppScreen::Player && !app.file_name.is_empty() {
                                    window.set_title(&format!("ump - {}", app.file_name));
                                }
                            }
                        }
                        InputResult::None => {}
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse.set_cursor_pos(position.x as f32, position.y as f32);
                let mut changed = false;
                if let Some(ref mut app) = self.app
                    && app.screen == AppScreen::Player
                {
                    if let Some(ref window) = self.window {
                        changed |= self.mouse.update_hover(app, window);
                    }
                    changed |= self.mouse.handle_drag(app);
                }
                if changed {
                    self.mark_input_handled();
                }
            }

            // Otherwise a hover highlight stays lit until the pointer comes back.
            WindowEvent::CursorLeft { .. } => {
                if let Some(ref mut app) = self.app
                    && app.hover.take().is_some()
                {
                    self.mark_input_handled();
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let handled = if let Some(ref mut app) = self.app {
                    app.screen == AppScreen::Player && self.mouse.handle_left_press(app)
                } else {
                    false
                };
                if handled {
                    self.mark_input_handled();
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.mouse.end_drag();
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (row_px, roll_step_px) = self
                    .renderer
                    .as_ref()
                    .map(|r| (r.cell_size().1 * ROW_HEIGHT, px(40.0, r.scale_factor())))
                    .unwrap_or((0.0, 0.0));
                let shift = self.modifiers.shift_key();
                let handled = if let Some(ref mut app) = self.app {
                    app.screen == AppScreen::Player
                        && self.mouse.handle_wheel(app, delta, row_px, roll_step_px, shift)
                } else {
                    false
                };
                if handled {
                    self.mark_input_handled();
                }
            }

            WindowEvent::RedrawRequested => {
                if let (Some(renderer), Some(app)) =
                    (&mut self.renderer, &mut self.app)
                {
                    match renderer.begin_frame() {
                        Ok(()) => {
                            renderer.clear(theme::GROUND);
                            render(renderer, app);
                            if let Err(e) = renderer.end_frame() {
                                log_warn!("Render error: {}", e);
                            }
                        }
                        Err(e) => {
                            log_warn!("Begin frame error: {}", e);
                        }
                    }
                }
            }

            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::ResumeTimeReached { .. } | StartCause::Poll => {
                // Check if playing state changed
                if let Some(ref app) = self.app {
                    if app.is_playing() {
                        self.needs_draw = true;
                    }
                }

                if self.needs_draw && self.last_draw.elapsed() >= self.frame_interval() {
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    self.last_draw = Instant::now();
                    self.needs_draw = false;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let interval = self.frame_interval();
        let next = self.last_draw + interval;
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load();
    debug::init(config.debug.verbose.unwrap_or(false));
    let sample_rate = query_sample_rate()?;

    let event_loop = EventLoop::new().context("Failed to create event loop")?;
    let mut ump_app = UmpApp::new(args, config, sample_rate);

    event_loop
        .run_app(&mut ump_app)
        .context("Event loop error")?;

    Ok(())
}
