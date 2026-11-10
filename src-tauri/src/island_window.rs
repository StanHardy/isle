use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW,
    GWL_EXSTYLE, HWND_TOPMOST, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

use crate::events::{self, ev};
use crate::state::AppState;

const ISLAND_LABEL: &str = "island";
const SETTINGS_LABEL: &str = "settings";

/// Compact pill size in logical units (before scale factor and user zoom).
pub const COMPACT_W: f64 = 348.0;
pub const COMPACT_H: f64 = 56.0;
/// Corner radius in logical units (compact pill / expanded panel).
pub const COMPACT_RADIUS: f64 = 28.0;
pub const EXPANDED_RADIUS: f64 = 24.0;
/// The island sits flush against the top of the work area — any gap lets
/// maximized apps' title bars peek through above it.
pub const TOP_OFFSET: f64 = 0.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    /// Work-area top-left in physical (virtual-screen) pixels.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

pub fn island<'a>(app: &'a AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window(ISLAND_LABEL)
}

pub fn settings_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window(SETTINGS_LABEL)
}

/// Primary-monitor work area in physical pixels plus the island window's scale.
pub fn anchor(app: &AppHandle) -> Anchor {
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut _ as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let scale = island(app)
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    Anchor {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        scale,
    }
}

/// Create the island overlay window (hidden until first page load).
pub fn create(app: &AppHandle) -> Result<(), tauri::Error> {
    let handle = app.clone();
    let win = WebviewWindowBuilder::new(app, ISLAND_LABEL, WebviewUrl::App("index.html".into()))
        .title("Isle")
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .visible(false)
        .focused(false)
        .on_page_load(move |_webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                if let Some(w) = handle.get_webview_window(ISLAND_LABEL) {
                    let _ = w.show();
                }
            }
        })
        .build()?;
    apply_ex_style(&win, true);
    apply_round_region(&win, COMPACT_RADIUS);
    position_compact(app);
    Ok(())
}

pub fn create_settings(app: &AppHandle) -> Result<(), tauri::Error> {
    let win = WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("settings.html".into()),
    )
    .title("Isle Settings")
    .decorations(true)
    .resizable(true)
    .visible(false)
    .inner_size(880.0, 640.0)
    .min_inner_size(720.0, 520.0)
    .build()?;
    // Closing the window must HIDE it, not destroy it — the tray and the
    // island's gear button reopen the same window; a destroyed one would
    // make "open settings" silently do nothing.
    let win_for_close = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_for_close.hide();
        }
    });
    let _ = win.hide();
    Ok(())
}

/// Clip the window to a rounded rect so the island can never show square
/// corners, regardless of webview rendering state. The system owns the
/// region after SetWindowRgn; re-apply whenever the size changes.
/// radius < 3 clears the region (rectangular strip design).
pub fn apply_round_region(win: &tauri::WebviewWindow, radius: f64) {
    let Ok(hwnd) = win.hwnd() else { return };
    if radius < 3.0 {
        unsafe {
            SetWindowRgn(hwnd, None, true);
        }
        return;
    }
    let Ok(scale) = win.scale_factor() else { return };
    let Ok(size) = win.outer_size() else { return };
    let r = ((radius * scale).round() as i32).max(2);
    unsafe {
        let hrgn = CreateRoundRectRgn(0, 0, size.width as i32 + 1, size.height as i32 + 1, r * 2, r * 2);
        if !hrgn.is_invalid() {
            // SetWindowRgn takes ownership of the region (system deletes it).
            SetWindowRgn(hwnd, Some(hrgn), true);
        }
    }
}

/// Place the island in compact state. Design-aware:
/// - strip: full work-area width, 48 px tall, flush at the top
/// - notch: hangs FROM the screen edge (window top pushed above the screen
///   so the uniform rounded region shows only bottom corners)
/// NOTE: all settings access happens in ONE scoped read — std RwLock is not
/// reentrant; a queued writer while holding the guard would deadlock.
pub fn position_compact(app: &AppHandle) {
    if let Some(win) = island(app) {
        let a = anchor(app);
        let (w, h, off, rad, centered) = {
            let state: tauri::State<AppState> = app.state();
            let s = state.settings.read().expect("settings lock");
            let scale = (s.island_scale as f64).max(0.5);
            match s.island_design {
                crate::config::IslandDesign::Strip => {
                    (a.width as f64 / scale, 48.0 * scale, 0.0, 0.0, false)
                }
                crate::config::IslandDesign::Notch => {
                    (360.0 * scale, 80.0 * scale, -24.0, 24.0, true)
                }
                _ => (COMPACT_W * scale, COMPACT_H * scale, TOP_OFFSET, COMPACT_RADIUS, true),
            }
        };
        let x = if centered {
            a.x as f64 / a.scale + ((a.width as f64 / a.scale - w) / 2.0)
        } else {
            a.x as f64 / a.scale
        };
        let y = a.y as f64 / a.scale + off;
        let _ = win.set_size(LogicalSize::new(w, h));
        let _ = win.set_position(LogicalPosition::new(x, y));
        apply_round_region(&win, rad);
    }
}

/// Toggle mouse-activation. The island runs with WS_EX_NOACTIVATE so clicking
/// the pill never steals focus from the app underneath; expansion re-enables
/// activation so the panel can take keyboard input.
pub fn set_noactivate(app: &AppHandle, noactivate: bool) {
    if let Some(win) = island(app) {
        apply_ex_style(&win, noactivate);
        if !noactivate {
            let _ = win.set_focus();
        }
    }
}

fn apply_ex_style(win: &tauri::WebviewWindow, noactivate: bool) {
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            let base = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let mut style = WINDOW_EX_STYLE(base as u32) | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
            if noactivate {
                style |= WS_EX_NOACTIVATE;
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style.0 as isize);
        }
    }
}

/// Reassert topmost and follow work-area changes (resolution, taskbar moves).
pub fn start_watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last = (0i32, 0i32, 0i32, 0i32, 0.0f64);
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let a = anchor(&app);
            let key = (a.x, a.y, a.width, a.height, a.scale);
            if key != last {
                let changed = last.0 != 0 || last.1 != 0 || last.2 != 0;
                last = key;
                if changed {
                    position_compact(&app);
                    events::emit(&app, ev::DISPLAY, &a);
                }
            }
            // Some topmost games cover overlays; reassert topmost when visible.
            if let Some(win) = island(&app) {
                if win.is_visible().unwrap_or(false) {
                    if let Ok(hwnd) = win.hwnd() {
                        unsafe {
                            let _ = SetWindowPos(
                                hwnd,
                                Some(HWND_TOPMOST),
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                            );
                        }
                    }
                }
            }
        }
    });
}
