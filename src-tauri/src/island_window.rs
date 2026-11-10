use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW, GWL_EXSTYLE,
    HWND_TOPMOST, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

use crate::events::{self, ev};
use crate::state::AppState;

const ISLAND_LABEL: &str = "island";
const SETTINGS_LABEL: &str = "settings";

/// Compact pill size in logical units (before scale factor and user zoom).
pub const COMPACT_W: f64 = 344.0;
pub const COMPACT_H: f64 = 60.0;
/// Expanded panel size in logical units.
pub const EXPANDED_W: f64 = 396.0;
pub const EXPANDED_H: f64 = 560.0;
/// Gap between the top of the work area and the island, logical units.
pub const TOP_OFFSET: f64 = 10.0;

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
    let _ = win.hide();
    Ok(())
}

/// Place the island at top-center of the primary work area in compact size.
pub fn position_compact(app: &AppHandle) {
    if let Some(win) = island(app) {
        let a = anchor(app);
        let s = user_scale(app);
        let w = COMPACT_W * s;
        let h = COMPACT_H * s;
        let x = a.x as f64 + ((a.width as f64 - w) / 2.0);
        let y = a.y as f64 + TOP_OFFSET;
        let _ = win.set_size(LogicalSize::new(w, h));
        let _ = win.set_position(LogicalPosition::new(x, y));
    }
}

fn user_scale(app: &AppHandle) -> f64 {
    let state: tauri::State<AppState> = app.state();
    let settings = state.settings.read().expect("settings lock");
    (settings.island_scale as f64).max(0.5)
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
