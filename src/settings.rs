//! Small, UI-independent settings model and JSON persistence.
//!
//! Native settings UI and Windows registry/profile discovery are intentionally
//! outside this module. A future Win32 settings window can validate values with
//! [`Settings::validate`] and save them through [`SettingsStore`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CANDIDATE_PAGE_SIZE: usize = 9;
pub const MIN_CANDIDATE_PAGE_SIZE: usize = 1;
pub const MAX_CANDIDATE_PAGE_SIZE: usize = 99;

/// Persisted settings shared by the TSF service and future settings window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub candidate_page_size: usize,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            candidate_page_size: DEFAULT_CANDIDATE_PAGE_SIZE,
            theme: Theme::System,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(MIN_CANDIDATE_PAGE_SIZE..=MAX_CANDIDATE_PAGE_SIZE).contains(&self.candidate_page_size)
        {
            return Err(SettingsError::InvalidCandidatePageSize {
                actual: self.candidate_page_size,
                min: MIN_CANDIDATE_PAGE_SIZE,
                max: MAX_CANDIDATE_PAGE_SIZE,
            });
        }
        Ok(())
    }
}

/// Candidate-window theme choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// JSON-backed settings store at a caller-selected path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads settings, returning defaults when the file does not exist.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        let json = match fs::read_to_string(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(error) => return Err(error.into()),
        };

        let settings: Settings = serde_json::from_str(&json)?;
        settings.validate()?;
        Ok(settings)
    }

    /// Validates and writes settings as pretty-printed UTF-8 JSON.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        settings.validate()?;

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let json = serde_json::to_string_pretty(settings)?;
        fs::write(&self.path, json)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("candidate page size must be between {min} and {max}, got {actual}")]
    InvalidCandidatePageSize {
        actual: usize,
        min: usize,
        max: usize,
    },

    #[error("could not read or write settings: {0}")]
    Io(#[from] io::Error),

    #[error("settings JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_store() -> SettingsStore {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "flower-settings-test-{}-{nonce}",
            std::process::id()
        ));
        SettingsStore::new(directory.join("settings.json"))
    }

    #[test]
    fn defaults_match_the_mvp_contract() {
        let settings = Settings::default();
        assert_eq!(settings.candidate_page_size, 9);
        assert_eq!(settings.theme, Theme::System);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn missing_file_loads_defaults() {
        let store = temporary_store();
        assert_eq!(store.load().expect("defaults"), Settings::default());
    }

    #[test]
    fn settings_round_trip_as_json() {
        let store = temporary_store();
        let settings = Settings {
            candidate_page_size: 7,
            theme: Theme::Dark,
        };
        store.save(&settings).expect("save settings");
        assert_eq!(store.load().expect("load settings"), settings);
        let _ = fs::remove_dir_all(store.path().parent().expect("parent"));
    }

    #[test]
    fn invalid_page_size_is_rejected() {
        let settings = Settings {
            candidate_page_size: 0,
            theme: Theme::System,
        };
        assert!(matches!(
            settings.validate(),
            Err(SettingsError::InvalidCandidatePageSize { .. })
        ));
    }
}
