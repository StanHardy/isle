//! Keep-awake: hold ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
//! on a dedicated worker thread so the machine (and its display) stay awake
//! until the requested deadline passes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};

use crate::events::{self, ev};

/// Epoch-ms deadline while active, None when off.
static DEADLINE: Mutex<Option<u64>> = Mutex::new(None);
static APPLIED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwakeState {
    pub active: bool,
    pub until_ms: u64,
    pub remaining_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn emit_state(app: &tauri::AppHandle, active: bool, until_ms: u64) {
    let now = now_ms();
    events::emit(
        app,
        "keepawake:state",
        &AwakeState {
            active,
            until_ms,
            remaining_ms: if active { until_ms.saturating_sub(now) } else { 0 },
        },
    );
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last_active = false;
        loop {
            let deadline = DEADLINE.lock().expect("keepawake lock").unwrap_or(0);
            let now = now_ms();
            let active = deadline > now;
            if active {
                if !APPLIED.swap(true, Ordering::Relaxed) {
                    unsafe {
                        SetThreadExecutionState(
                            ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
                        );
                    }
                }
                if !last_active {
                    emit_state(&app, true, deadline);
                }
                last_active = true;
                // Sleep until expiry (or a short slice so live edits apply).
                let remain = deadline - now;
                std::thread::sleep(Duration::from_millis(remain.min(1000)));
            } else {
                if APPLIED.swap(false, Ordering::Relaxed) {
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                }
                if last_active {
                    emit_state(&app, false, 0);
                }
                last_active = false;
                std::thread::sleep(Duration::from_millis(800));
            }
        }
    });
}

pub fn set(app: &tauri::AppHandle, minutes: u64) -> u64 {
    let deadline = now_ms() + minutes * 60_000;
    *DEADLINE.lock().expect("keepawake lock") = Some(deadline);
    emit_state(app, true, deadline);
    deadline
}

pub fn off(app: &tauri::AppHandle) {
    *DEADLINE.lock().expect("keepawake lock") = None;
    emit_state(app, false, 0);
}
