//! Application settings: small, human-editable JSON stored in the platform
//! config directory. Per-device data lives in SQLite (`mt-persistence`);
//! this file only holds client preferences.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which theme the user prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    Dark,
    Light,
    System,
}

impl std::fmt::Display for ThemePref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::Dark, ThemePref::Light, ThemePref::System];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::Dark => "Dark",
            ThemePref::Light => "Light",
            ThemePref::System => "System",
        }
    }
}

/// Client preferences persisted between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: ThemePref,
    /// Address of the last successfully used device.
    pub last_address: Option<String>,
    /// Reconnect to `last_address` on startup.
    pub auto_connect: bool,
    /// Show desktop notifications for incoming messages.
    pub notifications: bool,
    /// Send a message with Enter rather than Ctrl+Enter.
    pub send_on_enter: bool,
    /// Whether to begin a BLE scan when the Connect view opens.
    pub scan_ble_on_start: bool,
    /// Preferred position unit.
    pub imperial: bool,
    /// Draw the optional online OpenStreetMap tile layer on the map.
    pub online_tiles: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemePref::Dark,
            last_address: None,
            auto_connect: false,
            notifications: true,
            send_on_enter: true,
            scan_ble_on_start: false,
            imperial: false,
            online_tiles: false,
        }
    }
}

impl AppSettings {
    /// Location of the settings file.
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("meshtastic").join("settings.json"))
    }

    /// Load settings, falling back to defaults on any error.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist settings, ignoring errors (best effort; they are not vital).
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}
