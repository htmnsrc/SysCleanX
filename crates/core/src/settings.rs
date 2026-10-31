//! Persistent user settings in `%APPDATA%\SysCleanX\settings.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Checked target ids. `None` = built-in defaults.
    pub selection: Option<Vec<String>>,
    /// Paths that are never cleaned.
    pub exclusions: Vec<String>,
    /// Schedule locked files for deletion at next restart.
    pub reboot_locked: bool,
    /// Create a restore point before expert actions and debloating.
    pub restore_point: bool,
    /// Show targets with nothing to clean.
    pub show_empty: bool,
    pub light_theme: bool,
    /// Index into the GUI's accent colour list.
    pub accent: usize,
    /// Unix time of the last finished clean.
    pub last_clean: Option<u64>,
    /// Bytes freed across all cleans.
    pub total_freed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            selection: None,
            exclusions: Vec::new(),
            reboot_locked: false,
            restore_point: true,
            show_empty: false,
            light_theme: false,
            accent: 0,
            last_clean: None,
            total_freed: 0,
        }
    }
}

pub fn data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("SysCleanX");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("syscleanx")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn backups_dir() -> PathBuf {
    data_dir().join("backups")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("settings.json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("settings.json")).map_err(|e| e.to_string())
    }
}
