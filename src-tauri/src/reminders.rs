//! Repeat reminders: recurring reminders that fire a banner + chime on their
//! interval, persisted to reminders.json across restarts.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::events::{self, ev};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: u64,
    pub text: String,
    pub interval_min: u32,
    pub next_at_ms: u64,
}

fn path() -> std::path::PathBuf {
    crate::config::data_dir().join("reminders.json")
}

fn load() -> Vec<Reminder> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(app: &tauri::AppHandle) {
    let list = {
        let state: tauri::State<AppState> = app.state();
        let guard = state.reminders.lock().expect("reminders lock");
        guard.clone()
    };
    if let Ok(body) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(path(), body);
    }
    events::emit(app, ev::REMINDERS_SYNC, &list);
}

pub fn list(app: &tauri::AppHandle) -> Vec<Reminder> {
    let state: tauri::State<AppState> = app.state();
    let guard = state.reminders.lock().expect("reminders lock");
    guard.clone()
}

pub fn add(app: &tauri::AppHandle, text: &str, interval_min: u32) -> u64 {
    let state: tauri::State<AppState> = app.state();
    let mut list = state.reminders.lock().expect("reminders lock");
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    list.push(Reminder {
        id,
        text: text.chars().take(200).collect(),
        interval_min: interval_min.clamp(1, 1440),
        next_at_ms: now_ms() + interval_min as u64 * 60_000,
    });
    drop(list);
    save(app);
    id
}

pub fn delete(app: &tauri::AppHandle, id: u64) {
    let state: tauri::State<AppState> = app.state();
    state
        .reminders
        .lock()
        .expect("reminders lock")
        .retain(|r| r.id != id);
    save(app);
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Fire due reminders: banner + chime + reschedule. Runs every second.
pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(1000));
        let now = now_ms();
        let state: tauri::State<AppState> = app.state();
        let mut list = state.reminders.lock().expect("reminders lock");
        let due: Vec<Reminder> = list
            .iter()
            .filter(|r| r.next_at_ms <= now)
            .cloned()
            .collect();
        if due.is_empty() {
            continue;
        }
        for r in list.iter_mut() {
            if due.iter().any(|d| d.id == r.id) {
                r.next_at_ms = now + r.interval_min as u64 * 60_000;
            }
        }
        drop(list);
        for r in due {
            events::emit(
                &app,
                ev::ATTENTION,
                &serde_json::json!({ "kind": "reminder", "text": r.text }),
            );
            events::emit(
                &app,
                ev::REMINDER_FIRED,
                &serde_json::json!({ "id": r.id, "text": r.text }),
            );
            crate::audio::play_system_sound("Notification.Default");
            crate::events::debug_log(&format!("[reminders] fired: {}", r.text));
        }
    });
}

/// Mirror reminders into the app state at startup so the list survives
/// restarts before the first save.
pub fn init(app: &tauri::AppHandle) {
    let state: tauri::State<AppState> = app.state();
    let mut list = state.reminders.lock().expect("reminders lock");
    *list = load();
}
