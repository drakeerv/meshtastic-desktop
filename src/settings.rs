//! Application settings: small, human-editable JSON stored in the platform
//! config directory. Per-device data lives in SQLite (`mt-persistence`);
//! this file only holds client preferences.

use std::collections::HashMap;
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
    /// Allow the IP-based location fallback when GeoClue and gpsd are absent.
    pub use_ip_location: bool,
    /// Device addresses whose host location should be shared automatically
    /// while connected. Keyed by `DeviceAddress::to_string` so the choice
    /// survives reconnects and restarts.
    pub share_location_devices: Vec<String>,
    /// Hide to the system tray instead of quitting when the window is closed.
    pub close_to_tray: bool,
    /// Boost contrast of borders and secondary text.
    pub high_contrast: bool,
    /// Interface scale factor applied to the whole window (1.0 is default).
    pub ui_scale: f32,
    /// Node number of the last connected device, used to show its message
    /// history offline and to key unread state.
    pub last_node_num: Option<u32>,
    /// Last-read message id per conversation, keyed `"<node>:<conversation>"`.
    pub read_marks: HashMap<String, i64>,
    /// Device node numbers whose history has already been seen, so the first
    /// load of a device's history is treated as read.
    pub known_history: Vec<u32>,
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
            use_ip_location: false,
            share_location_devices: Vec::new(),
            close_to_tray: true,
            high_contrast: false,
            ui_scale: 1.0,
            last_node_num: None,
            read_marks: HashMap::new(),
            known_history: Vec::new(),
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

    /// Whether the device at `address` shares the host's location.
    pub fn shares_location(&self, address: Option<&str>) -> bool {
        match address {
            Some(address) => self.share_location_devices.iter().any(|d| d == address),
            None => false,
        }
    }

    /// Enable or disable location sharing for `address`.
    pub fn set_shares_location(&mut self, address: &str, enabled: bool) {
        self.share_location_devices.retain(|d| d != address);
        if enabled {
            self.share_location_devices.push(address.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_sharing_is_per_device() {
        let mut settings = AppSettings::default();
        assert!(!settings.shares_location(Some("xaa")));
        assert!(!settings.shares_location(None));

        settings.set_shares_location("xaa", true);
        assert!(settings.shares_location(Some("xaa")));
        assert!(!settings.shares_location(Some("xbb")));

        settings.set_shares_location("xaa", false);
        assert!(!settings.shares_location(Some("xaa")));
        assert!(settings.share_location_devices.is_empty());
    }
}
