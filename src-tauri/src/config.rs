use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Tauri app data dir for the unpackaged app: %APPDATA%\<identifier>.
/// Computed directly so services can load config before any window exists.
pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("com.stanhardy.isle")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpandMode {
    Hover,
    Click,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TempUnit {
    Metric,
    Imperial,
}

/// Wind/speed unit is independent of temperature (UK: °C + mph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeedUnit {
    Kmh,
    Mph,
}

/// Visual style of the spectrum bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpectrumStyle {
    Accent,
    White,
    Gradient,
    Tint,
    Mono,
}

/// Overall island look: floating glass capsule (pill), screen-edge notch
/// (opaque, hangs from the top, Notchy-style), Droppy-style material
/// variants of the capsule (opaque black / clear glass), or a full-width
/// top strip (Peeka-style) that covers the entire screen edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IslandDesign {
    Pill,
    Notch,
    Black,
    Clear,
    Strip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CityRef {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    /// User zoom multiplier applied on top of the monitor scale factor.
    pub island_scale: f32,
    pub island_design: IslandDesign,
    pub expand_mode: ExpandMode,
    pub theme: Theme,
    /// Hide the island while a fullscreen app is in the foreground.
    pub hide_fullscreen: bool,
    /// Module ids in display order for the expanded panel.
    pub module_order: Vec<String>,
    pub modules_enabled: std::collections::HashMap<String, bool>,
    pub media_enabled: bool,
    pub metrics_enabled: bool,
    pub weather_enabled: bool,
    pub weather_city: Option<CityRef>,
    pub temp_unit: TempUnit,
    pub wind_unit: SpeedUnit,
    pub lyrics_enabled: bool,
    /// Auto-lookup TV/movie info for video-looking media (episode/year titles).
    pub media_info_auto: bool,
    /// Spectrum bar color theme.
    pub spectrum_style: SpectrumStyle,
    /// Play a chime when a timer completes.
    pub timer_sound: bool,
    /// Opt-in clipboard history (privacy-sensitive).
    pub clipboard_enabled: bool,
    /// Bounded local retention for clipboard history.
    pub clipboard_keep: u32,
    /// Opt-in notification mirroring (privacy-sensitive).
    pub notifications_enabled: bool,
    pub bluetooth_enabled: bool,
    /// Opt-in phone companion bridge (LAN, token-gated).
    pub bridge_enabled: bool,
    pub bridge_token: String,
    /// Marker only — the actual key lives in Windows Credential Manager.
    pub tmdb_configured: bool,
}

impl Default for Settings {
    /// Explicit defaults — NOT serde-based, because container-level
    /// `#[serde(default)]` calls this during parsing and would recurse.
    fn default() -> Self {
        Self {
            version: 1,
            island_scale: 1.0,
            island_design: IslandDesign::Pill,
            expand_mode: ExpandMode::Hover,
            theme: Theme::System,
            hide_fullscreen: true,
            module_order: [
                "media", "info", "timer", "note", "weather", "metrics", "lyrics", "bluetooth",
                "notifications", "clipboard",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            modules_enabled: [
                ("media", true),
                ("note", true),
                ("timer", true),
                ("weather", true),
                ("metrics", true),
                ("info", true),
                ("lyrics", true),
                ("bluetooth", true),
                ("notifications", false),
                ("clipboard", false),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
            media_enabled: true,
            metrics_enabled: true,
            weather_enabled: true,
            weather_city: None,
            temp_unit: TempUnit::Metric,
            wind_unit: SpeedUnit::Mph,
            lyrics_enabled: true,
            media_info_auto: true,
            spectrum_style: SpectrumStyle::Accent,
            timer_sound: true,
            clipboard_enabled: false,
            clipboard_keep: 100,
            notifications_enabled: false,
            bluetooth_enabled: true,
            bridge_enabled: false,
            bridge_token: String::new(),
            tmdb_configured: false,
        }
    }
}

impl Settings {
    /// Load settings from disk; on missing/corrupt file fall back to defaults,
    /// merging the file over defaults so new keys survive upgrades.
    pub fn load_or_default() -> Self {
        let defaults: serde_json::Value = serde_json::from_str(DEFAULT_SETTINGS_JSON)
            .expect("built-in defaults are valid");
        let merged = match fs::read_to_string(settings_path()) {
            Ok(text) => {
                let stored: serde_json::Value = serde_json::from_str(&text).unwrap_or(defaults.clone());
                merge(defaults, stored)
            }
            Err(_) => defaults,
        };
        let mut settings: Settings = serde_json::from_value(merged).unwrap_or_default();
        if settings.bridge_token.is_empty() {
            settings.bridge_token = generate_token();
            settings.save();
        }
        settings
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(body) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, body).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }
}

/// Recursive merge: `stored` wins for scalars/arrays; objects merge key-wise.
fn merge(mut base: serde_json::Value, stored: serde_json::Value) -> serde_json::Value {
    match (base.take(), stored) {
        (serde_json::Value::Object(mut a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(existing) => {
                        let mut e = existing;
                        merge(e.take(), v)
                    }
                    None => v,
                };
                a.insert(k, merged);
            }
            serde_json::Value::Object(a)
        }
        (_, stored) => stored,
    }
}

/// Generate a 64-bit token as 16 hex chars — adequate for LAN pairing.
pub fn generate_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    h.write_u128(nanos);
    h.write_u128(std::process::id() as u128);
    format!("{:016x}", h.finish())
}

pub const DEFAULT_SETTINGS_JSON: &str = r#"{
  "version": 1,
  "islandScale": 1.0,
  "islandDesign": "pill",
  "expandMode": "hover",
  "theme": "system",
  "hideFullscreen": true,
  "moduleOrder": ["media", "info", "timer", "note", "weather", "metrics", "lyrics", "bluetooth", "notifications", "clipboard"],
  "modulesEnabled": {
    "media": true,
    "timer": true,
    "weather": true,
    "metrics": true,
    "info": true,
    "lyrics": true,
    "bluetooth": true,
    "notifications": false,
    "clipboard": false
  },
  "mediaEnabled": true,
  "metricsEnabled": true,
  "weatherEnabled": true,
  "weatherCity": null,
  "tempUnit": "metric",
  "windUnit": "mph",
  "lyricsEnabled": true,
  "mediaInfoAuto": true,
  "spectrumStyle": "accent",
  "timerSound": true,
  "clipboardEnabled": false,
  "clipboardKeep": 100,
  "notificationsEnabled": false,
  "bluetoothEnabled": true,
  "bridgeEnabled": false,
  "bridgeToken": "",
  "tmdbConfigured": false
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let s: Settings = serde_json::from_str(DEFAULT_SETTINGS_JSON).unwrap();
        assert_eq!(s.expand_mode, ExpandMode::Hover);
        assert!(!s.clipboard_enabled);
        assert!(s.clipboard_keep > 0);
    }

    #[test]
    fn merge_prefers_stored_scalars_and_keeps_new_keys() {
        let base: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"b":{"x":1,"y":2},"new":true}"#).unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(r#"{"a":9,"b":{"x":5}}"#).unwrap();
        let out = merge(base, stored);
        assert_eq!(out["a"], 9);
        assert_eq!(out["b"]["x"], 5);
        assert_eq!(out["b"]["y"], 2);
        assert_eq!(out["new"], true);
    }

    #[test]
    fn token_is_hex16() {
        let t = generate_token();
        assert_eq!(t.len(), 16);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, generate_token());
    }
}
