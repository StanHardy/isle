use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};

use crate::config::Settings;
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
    /// Looked-up TV/movie info keyed by media art key (bounded, oldest first).
    pub media_info: Mutex<Vec<(String, MediaInfo)>>,
}

/// Bounded media-info cache helpers shared by commands and the auto-lookup.
pub const MEDIA_INFO_CAP: usize = 40;

pub fn media_info_get(cache: &[(String, MediaInfo)], key: &str) -> Option<MediaInfo> {
    cache
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, info)| info.clone())
}

pub fn media_info_insert(cache: &mut Vec<(String, MediaInfo)>, key: String, info: MediaInfo) {
    if let Some(slot) = cache.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = info;
        return;
    }
    if cache.len() >= MEDIA_INFO_CAP {
        cache.remove(0);
    }
    cache.push((key, info));
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
            media_info: Mutex::new(Vec::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
