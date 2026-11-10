use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::events::{self, ev};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub id: u64,
    pub label: String,
    pub total_ms: u64,
    pub remaining_ms: u64,
    pub paused: bool,
    /// Epoch ms when an unpaused timer will finish.
    pub ends_at_ms: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Countdown loop: emits ticks and completion; state lives in AppState.timers.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_tick_ms = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let state: tauri::State<AppState> = app.state();
            let mut timers = state.timers.lock().expect("timers lock");
            let now = now_ms();
            let mut done = Vec::new();
            for t in timers.values_mut() {
                if !t.paused {
                    let left = t.ends_at_ms.saturating_sub(now);
                    if left == 0 {
                        done.push((t.id, t.label.clone(), t.total_ms));
                    }
                    t.remaining_ms = left;
                }
            }
            for (id, _label, _total) in &done {
                timers.remove(id);
            }
            drop(timers);
            for (id, label, total) in done {
                events::emit(&app, ev::TIMER_DONE, &serde_json::json!({ "id": id, "label": label }));
                events::emit(
                    &app,
                    ev::ATTENTION,
                    &serde_json::json!({ "kind": "timer-done", "id": id, "label": label }),
                );
                if timer_sound(&app) {
                    crate::audio::play_system_sound("Notification.Default");
                }
                crate::focus::record(((total / 60_000).max(1)) as u32);
            }
            if now - last_tick_ms >= 500 {
                last_tick_ms = now;
                let state: tauri::State<AppState> = app.state();
                let timers = state.timers.lock().expect("timers lock");
                let list: Vec<&Timer> = timers.values().collect();
                events::emit(&app, ev::TIMER_TICK, &list);
            }
        }
    });
}

fn timer_sound(app: &AppHandle) -> bool {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.timer_sound
}

/// Create a timer; returns the assigned id.
pub fn add(app: &AppHandle, label: &str, seconds: u64) -> u64 {
    let state: tauri::State<AppState> = app.state();
    let mut timers = state.timers.lock().expect("timers lock");
    let id = timers.keys().copied().max().unwrap_or(0) + 1;
    let total = seconds * 1000;
    timers.insert(
        id,
        Timer {
            id,
            label: label.to_string(),
            total_ms: total,
            remaining_ms: total,
            paused: false,
            ends_at_ms: now_ms() + total,
        },
    );
    id
}
