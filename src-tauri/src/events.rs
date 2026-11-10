use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Event channel names shared with the frontend (mirrored in src/shared/events.ts).
pub mod ev {
    pub const MEDIA: &str = "media:update";
    pub const MEDIA_INFO: &str = "media:info";
    pub const SPECTRUM: &str = "audio:spectrum";
    pub const SETTINGS: &str = "settings:changed";
    pub const REMINDERS_SYNC: &str = "reminders:sync";
    pub const REMINDER_FIRED: &str = "reminder:fired";
    pub const VOLUME_OSD: &str = "volume:osd";
    pub const OSD_KEY: &str = "osd:key";
    pub const METRICS: &str = "metrics:sample";
    pub const TIMER_TICK: &str = "timer:tick";
    pub const TIMER_DONE: &str = "timer:done";
    pub const CLIPBOARD_SYNC: &str = "clipboard:sync";
    pub const NOTIFY_NEW: &str = "notify:new";
    pub const BT_DEVICE: &str = "bt:device";
    pub const WEATHER: &str = "weather:update";
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

/// Append a line to %APPDATA%/com.stanhardy.isle/debug.log — diagnostics for
/// problems that are invisible in a windowed app (no console attached).
/// Rotates at 256 KB.
pub fn debug_log(msg: &str) {
    use std::io::Write;
    let path = crate::config::data_dir().join("debug.log");
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > 256 * 1024 {
        let _ = std::fs::remove_file(&path);
        let Ok(f2) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else {
            return;
        };
        f = f2;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(f, "[{secs}] {msg}");
}
