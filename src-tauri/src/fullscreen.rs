use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, GWL_STYLE, WS_CAPTION,
};

use crate::events::{self, ev};
use crate::island_window;
use crate::state::AppState;

const GRACE: Duration = Duration::from_secs(6);

/// Set when the island was hidden by fullscreen; used to restore only what we hid.
static HIDDEN_BY_FS: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullscreenState {
    pub active: bool,
    pub class: String,
    pub title: String,
    pub pid: u32,
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut last_active = false;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            // Launch grace: the boot sequence can briefly look fullscreen.
            if start.elapsed() < GRACE {
                continue;
            }
            let st = probe();
            if st.active != last_active {
                last_active = st.active;
                handle_change(&app, &st);
            }
        }
    });
}

fn probe() -> FullscreenState {
    let mut out = FullscreenState {
        active: false,
        class: String::new(),
        title: String::new(),
        pid: 0,
    };
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return out;
        }
        out.pid = GetWindowThreadProcessId(fg, None);
        let mut buf = [0u16; 256];
        let n = GetClassNameW(fg, &mut buf);
        out.class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let n = GetWindowTextW(fg, &mut buf);
        out.title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

        // Skip the desktop itself.
        if out.class == "Progman" || out.class == "WorkerW" || out.class == "Shell_TrayWnd" {
            return out;
        }
        let mut r = RECT::default();
        // Some of these return BOOL and some Result in the projection; ignore
        // return values and judge success by the produced rects.
        let _ = GetWindowRect(fg, &mut r);
        let hmon: HMONITOR = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if hmon.is_invalid() {
            return out;
        }
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(hmon, &mut mi);
        if mi.rcMonitor.right > mi.rcMonitor.left {
            let covers = r.left <= mi.rcMonitor.left
                && r.top <= mi.rcMonitor.top
                && r.right >= mi.rcMonitor.right
                && r.bottom >= mi.rcMonitor.bottom;
            let style = GetWindowLongW(fg, GWL_STYLE);
            let borderless = ((style as u32) & WS_CAPTION.0) == 0;
            out.active = covers && borderless;
        }
    }
    out
}

fn handle_change(app: &AppHandle, st: &FullscreenState) {
    events::emit(app, ev::FULLSCREEN, st);
    let hide = st.active && {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        s.hide_fullscreen
    };
    if let Some(win) = island_window::island(app) {
        if hide {
            HIDDEN_BY_FS.store(true, Ordering::Relaxed);
            let _ = win.hide();
        } else if HIDDEN_BY_FS.swap(false, Ordering::Relaxed) {
            let _ = win.show();
        }
    }
}
