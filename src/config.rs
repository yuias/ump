//! Settings persistence via settings.toml.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub soundfont: SoundfontConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FontConfig {
    pub path: Option<String>,
    pub size: Option<f32>,
}

impl FontConfig {
    #[cfg(feature = "d2d")]
    pub fn family(&self) -> &str {
        "Consolas"
    }

    pub fn size_or_default(&self) -> f32 {
        self.size.unwrap_or(14.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub volume: Option<u32>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig { volume: Some(80) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WindowConfig {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayConfig {
    pub show_piano_roll: Option<bool>,
    pub track_view_mode: Option<String>,
    pub piano_roll_vertical: Option<bool>,
}


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DebugConfig {
    pub verbose: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SoundfontConfig {
    pub default_path: Option<String>,
    pub recent_path: Option<String>,
}

impl Config {
    /// Return the path to the config file.
    /// Windows: %APPDATA%/ump/settings.toml
    /// Linux/macOS: ~/.config/ump/settings.toml
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("ump").join("settings.toml"))
    }

    /// Load config from disk. Returns default if file does not exist or parse fails.
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        let Ok(content) = fs::read_to_string(&path) else {
            return Self::default();
        };
        match toml::from_str(&content) {
            Ok(config) => {
                log_info!("Config loaded: {}", path.display());
                config
            }
            Err(e) => {
                log_warn!("Config parse error (using defaults): {}", e);
                Self::default()
            }
        }
    }

    /// Save config to disk. Creates parent directories if needed.
    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match toml::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = fs::write(&path, content) {
                    log_warn!("Config save failed: {}", e);
                }
            }
            Err(e) => {
                log_warn!("Config serialize failed: {}", e);
            }
        }
    }

    /// Resolve the SF2 path from config (recent > default).
    /// Relative paths are resolved against the config directory.
    pub fn resolve_sf2(&self) -> Option<String> {
        self.soundfont
            .recent_path
            .as_deref()
            .or(self.soundfont.default_path.as_deref())
            .map(resolve_path)
    }
}

/// Canonicalize a path to an absolute path string. Falls back to the original on failure.
pub fn to_absolute_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Resolve a path that may be relative to the config directory.
/// Absolute paths are returned as-is. Relative paths are joined with config_dir.
pub fn resolve_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    // Relative: resolve against config directory
    if let Some(config_path) = Config::config_path() {
        if let Some(config_dir) = config_path.parent() {
            let resolved = config_dir.join(p);
            return resolved.to_string_lossy().to_string();
        }
    }
    path.to_string()
}
