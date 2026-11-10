use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};

use crate::config::{Settings, DEFAULT_SETTINGS_JSON};
use crate::clipboard::ClipStore;
use crate::media::MediaState;
use crate::providers::tmdb::MediaInfo;
use crate::timers::Timer;

/// Shared application state managed by Tauri and read by services/commands.
pub struct AppState {
    pub settings: RwLock<Settings>,
    /// Latest SMTC media snapshot (None = no active session).
    pub media: Mutex<Option<MediaState>>,
    /// Countdown timers keyed by id.
    pub timers: Mutex<HashMap<u64, Timer>>,
    /// Clipboard history (in-memory, bounded, opt-in).
    pub clipboard: Mutex<ClipStore>,
    /// Mirrored notification feed (newest first).
    pub notifications: Mutex<Vec<crate::notify::NotifyItem>>,
    /// Bluetooth device connection state by endpoint id.
    pub bt_devices: Mutex<HashMap<String, crate::bluetooth::BtDevice>>,
    /// Whether the island UI currently wants metric samples.
    pub metrics_subscribed: AtomicBool,
    /// Looked-up TV/movie info keyed by media art key.
    pub media_info: Mutex<HashMap<String, MediaInfo>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            settings: RwLock::new(Settings::load_or_default()),
            media: Mutex::new(None),
            timers: Mutex::new(HashMap::new()),
            clipboard: Mutex::new(ClipStore::default()),
            notifications: Mutex::new(Vec::new()),
            bt_devices: Mutex::new(HashMap::new()),
            metrics_subscribed: AtomicBool::new(false),
            media_info: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Exposed for unit tests / external tools: the default settings document.
pub fn default_settings() -> serde_json::Value {
    serde_json::from_str(DEFAULT_SETTINGS_JSON).expect("default settings json is valid")
}
