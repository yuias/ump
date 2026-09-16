//! SoundFont bundle configuration.
//!
//! A bundle is a set of SF2 files plus a per-channel routing table, selected by
//! the detected MIDI mode. The shape lives here rather than in a host so that
//! ump and the foobar2000 component read the same configuration.
//!
//! Enable the `serde` feature to deserialize these from a settings file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::midi::mode_detect::MidiMode;

/// A set of SF2 files with per-channel routing.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SoundfontBundle {
    /// SF2 file paths. Relative paths are resolved against the settings file.
    pub files: Vec<String>,
    /// Channels 0-15 mapped to an index into `files`. Absent means every
    /// channel plays through the first file.
    pub routing: Option<Vec<u8>>,
}

impl SoundfontBundle {
    /// Routing table for [`SynthPool::from_soundfonts`], with out-of-range
    /// entries clamped to the first file so a mistyped index cannot panic.
    ///
    /// [`SynthPool::from_soundfonts`]: crate::synth::engine::SynthPool::from_soundfonts
    pub fn routing_table(&self) -> [usize; 16] {
        let mut table = [0usize; 16];
        let Some(routing) = &self.routing else {
            return table;
        };
        for (channel, &index) in routing.iter().take(16).enumerate() {
            let index = index as usize;
            if index < self.files.len() {
                table[channel] = index;
            }
        }
        table
    }

    /// File paths resolved against `base_dir`, which is where the settings file
    /// lives. Absolute paths are returned unchanged.
    pub fn resolved_files(&self, base_dir: Option<&Path>) -> Vec<PathBuf> {
        self.files
            .iter()
            .map(|file| {
                let path = Path::new(file);
                match base_dir {
                    Some(dir) if path.is_relative() => dir.join(path),
                    _ => path.to_path_buf(),
                }
            })
            .collect()
    }
}

/// The `[soundfont]` section of a settings file.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SoundfontConfig {
    pub default_path: Option<String>,
    pub recent_path: Option<String>,
    /// Bundles keyed by MIDI mode: `"GM"`, `"GS"`, `"XG"`, `"GM2"`.
    pub bundles: Option<HashMap<String, SoundfontBundle>>,
}

impl SoundfontConfig {
    /// Look up the bundle for a mode name as written in the settings file.
    pub fn resolve_bundle(&self, mode: &str) -> Option<&SoundfontBundle> {
        self.bundles.as_ref()?.get(mode)
    }

    /// Look up the bundle for a detected mode.
    pub fn bundle_for_mode(&self, mode: MidiMode) -> Option<&SoundfontBundle> {
        self.resolve_bundle(&mode.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(files: usize, routing: Option<Vec<u8>>) -> SoundfontBundle {
        SoundfontBundle {
            files: (0..files).map(|i| format!("{i}.sf2")).collect(),
            routing,
        }
    }

    #[test]
    fn missing_routing_puts_every_channel_on_the_first_file() {
        assert_eq!(bundle(2, None).routing_table(), [0; 16]);
    }

    #[test]
    fn routing_is_clamped_to_the_file_count() {
        let table = bundle(2, Some(vec![1, 9, 0])).routing_table();
        assert_eq!(table[0], 1);
        assert_eq!(table[1], 0, "index 9 has no file and falls back");
        assert_eq!(table[2], 0);
    }

    #[test]
    fn extra_routing_entries_are_ignored() {
        let table = bundle(2, Some(vec![1; 20])).routing_table();
        assert_eq!(table, [1; 16]);
    }

    #[test]
    fn relative_files_resolve_against_the_settings_directory() {
        let base = Path::new("/etc/ump");
        let resolved = bundle(1, None).resolved_files(Some(base));
        assert_eq!(resolved[0], base.join("0.sf2"));
    }

    #[test]
    fn bundles_are_looked_up_by_mode_name() {
        let mut bundles = HashMap::new();
        bundles.insert("XG".to_string(), bundle(1, None));
        let config = SoundfontConfig {
            bundles: Some(bundles),
            ..Default::default()
        };
        assert!(config.bundle_for_mode(MidiMode::XG).is_some());
        assert!(config.bundle_for_mode(MidiMode::GM).is_none());
    }
}
