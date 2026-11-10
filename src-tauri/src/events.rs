use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Event channel names shared with the frontend (mirrored in src/shared/events.ts).
pub mod ev {
    pub const MEDIA: &str = "media:update";
    pub const MEDIA_INFO: &str = "media:info";
    pub const SPECTRUM: &str = "audio:spectrum";
    pub const METRICS: &str = "metrics:sample";
    pub const TIMER_TICK: &str = "timer:tick";
    pub const TIMER_DONE: &str = "timer:done";
    pub const CLIPBOARD_SYNC: &str = "clipboard:sync";
    pub const NOTIFY_NEW: &str = "notify:new";
    pub const BT_DEVICE: &str = "bt:device";
    pub const WEATHER: &str = "weather:update";
    pub const LYRICS: &str = "lyrics:loaded";
    pub const FULLSCREEN: &str = "fullscreen:changed";
    pub const DISPLAY: &str = "display:changed";
    pub const ATTENTION: &str = "island:attention";
}

/// Emit an event; failures are logged, never fatal (services run fire-and-forget).
pub fn emit<T: Serialize>(app: &AppHandle, event: &str, payload: &T) {
    if let Err(err) = app.emit(event, payload) {
        eprintln!("[isle] emit {event} failed: {err}");
    }
}
