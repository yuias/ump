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
    /// Font size in logical px. Multiplied by the display's scale factor to
    /// get the physical px size actually rendered.
    pub size: Option<f32>,
}

impl FontConfig {
    /// Font size given whether the font came from auto-detection. Auto-detected
    /// dot fonts default to 16px (a multiple of their 16-dot grid renders
    /// crisply); explicit or absent fonts keep the 14px default.
    fn size_for(&self, auto: bool) -> f32 {
        self.size.unwrap_or(if auto { 16.0 } else { 14.0 })
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
    /// Vertical piano roll flow direction: "down" (falling, default) or "up"
    /// (rising, tracker style). Unknown values fall back to "down".
    pub piano_roll_flow: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DebugConfig {
    pub verbose: Option<bool>,
}

// The soundfont bundle shape lives in ump-playback so that the foobar2000
// component reads the same settings.toml.
pub use ump_playback::synth::bundle::{SoundfontBundle, SoundfontConfig};

impl Config {
    /// Return the path to the config file.
    /// Windows: %LOCALAPPDATA%/ump/settings.toml
    /// Linux/macOS: ~/.config/ump/settings.toml
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_local_dir().map(|d| d.join("ump").join("settings.toml"))
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

    /// Directory that holds user-installed fonts (not bundled with the app).
    /// Kept next to settings.toml so a relative `font.path` like `fonts/x.ttf`
    /// and auto-detection look in the same place.
    /// Windows: %LOCALAPPDATA%/ump/fonts
    /// Linux:   ~/.config/ump/fonts
    /// macOS:   ~/Library/Application Support/ump/fonts
    pub fn fonts_dir() -> Option<PathBuf> {
        dirs::config_local_dir().map(|d| d.join("ump").join("fonts"))
    }

    /// Resolve the font to load and its size, in priority order:
    /// explicit `font.path` > auto-detected file in `fonts_dir()` > system default.
    /// Auto-detected dot fonts default to 16px unless `font.size` overrides it.
    pub fn resolve_font(&self) -> (Option<String>, f32) {
        let candidates: Vec<PathBuf> = Self::fonts_dir()
            .and_then(|dir| fs::read_dir(&dir).ok())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();

        match pick_font(self.font.path.as_deref(), &candidates) {
            FontChoice::Explicit(path) => {
                log_info!("Font: using configured font.path = {}", path);
                (Some(resolve_path(&path)), self.font.size_for(false))
            }
            FontChoice::Auto(path) => {
                let size = self.font.size_for(true);
                log_info!("Font: auto-detected '{}' (size {})", path.display(), size);
                (Some(path.to_string_lossy().to_string()), size)
            }
            FontChoice::None => {
                log_info!("Font: no font.path set and nothing found in fonts dir; using system default");
                (None, self.font.size_for(false))
            }
        }
    }
}

/// Outcome of automatic font resolution (see `pick_font`).
#[derive(Debug, Clone, PartialEq)]
enum FontChoice {
    /// Explicit `font.path` from config, not yet resolved against config dir.
    Explicit(String),
    /// Auto-detected file from `fonts_dir()`.
    Auto(PathBuf),
    /// No font.path set and nothing usable in `fonts_dir()`.
    None,
}

/// Pick a font from `candidates` (files in `fonts_dir()`), pure and testable.
///
/// Priority when `explicit` is unset, by lowercased file name:
/// 1. contains "jiskan16s" (public-domain JF Dot variant; plain "jiskan16"
///    is excluded since its half-width glyphs are Sony-licensed)
/// 2. contains "shinonome" and "16" (Shinonome Gothic 16, public domain)
/// 3. contains "dotgothic16" (OFL-licensed, recommended default)
///
/// Ties within a priority resolve to the lexicographically first name.
fn pick_font(explicit: Option<&str>, candidates: &[PathBuf]) -> FontChoice {
    if let Some(path) = explicit {
        return FontChoice::Explicit(path.to_string());
    }

    fn priority(lower_name: &str) -> Option<u8> {
        if lower_name.contains("jiskan16s") {
            Some(0)
        } else if lower_name.contains("shinonome") && lower_name.contains("16") {
            Some(1)
        } else if lower_name.contains("dotgothic16") {
            Some(2)
        } else {
            None
        }
    }

    let best = candidates
        .iter()
        .filter(|path| {
            matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_lowercase)
                    .as_deref(),
                Some("ttf") | Some("otf") | Some("ttc")
            )
        })
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_lowercase();
            priority(&name).map(|pri| (pri, name, path))
        })
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    match best {
        Some((_, _, path)) => FontChoice::Auto(path.clone()),
        None => FontChoice::None,
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
    if let Some(config_path) = Config::config_path()
        && let Some(config_dir) = config_path.parent()
    {
        let resolved = config_dir.join(p);
        return resolved.to_string_lossy().to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_font_explicit_path_wins_over_candidates() {
        let candidates = vec![PathBuf::from("dotgothic16-regular.ttf")];
        let choice = pick_font(Some("custom.ttf"), &candidates);
        assert_eq!(choice, FontChoice::Explicit("custom.ttf".to_string()));
    }

    #[test]
    fn pick_font_priority_jiskan16s_beats_shinonome_beats_dotgothic16() {
        let all = vec![
            PathBuf::from("dotgothic16-regular.ttf"),
            PathBuf::from("shinonome-gothic-16.ttf"),
            PathBuf::from("jf-dot-jiskan16s-1990.ttf"),
        ];
        assert_eq!(
            pick_font(None, &all),
            FontChoice::Auto(PathBuf::from("jf-dot-jiskan16s-1990.ttf"))
        );

        let without_jiskan16s = vec![
            PathBuf::from("dotgothic16-regular.ttf"),
            PathBuf::from("shinonome-gothic-16.ttf"),
        ];
        assert_eq!(
            pick_font(None, &without_jiskan16s),
            FontChoice::Auto(PathBuf::from("shinonome-gothic-16.ttf"))
        );
    }

    #[test]
    fn pick_font_plain_jiskan16_is_ignored() {
        let candidates = vec![PathBuf::from("jf-dot-jiskan16.ttf")];
        assert_eq!(pick_font(None, &candidates), FontChoice::None);
    }

    #[test]
    fn pick_font_is_case_insensitive() {
        let candidates = vec![PathBuf::from("DotGothic16-Regular.TTF")];
        assert_eq!(
            pick_font(None, &candidates),
            FontChoice::Auto(PathBuf::from("DotGothic16-Regular.TTF"))
        );
    }

    #[test]
    fn pick_font_empty_dir_returns_none() {
        assert_eq!(pick_font(None, &[]), FontChoice::None);
    }

    #[test]
    fn pick_font_ties_within_priority_break_lexicographically() {
        let candidates = vec![
            PathBuf::from("z-dotgothic16-extra.ttf"),
            PathBuf::from("a-dotgothic16-extra.ttf"),
        ];
        assert_eq!(
            pick_font(None, &candidates),
            FontChoice::Auto(PathBuf::from("a-dotgothic16-extra.ttf"))
        );
    }

    #[test]
    fn pick_font_ignores_non_font_extensions() {
        let candidates = vec![PathBuf::from("dotgothic16-regular.png")];
        assert_eq!(pick_font(None, &candidates), FontChoice::None);
    }

    #[test]
    fn font_config_size_defaults_to_16_when_auto_and_unset() {
        let cfg = FontConfig { path: None, size: None };
        assert_eq!(cfg.size_for(true), 16.0);
        assert_eq!(cfg.size_for(false), 14.0);
    }

    #[test]
    fn font_config_size_override_wins_regardless_of_auto() {
        let cfg = FontConfig { path: None, size: Some(20.0) };
        assert_eq!(cfg.size_for(true), 20.0);
        assert_eq!(cfg.size_for(false), 20.0);
    }
}
