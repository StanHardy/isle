mod audio;
mod bluetooth;
mod bridge;
mod clipboard;
mod commands;
mod config;
mod credentials;
mod events;
mod focus;
mod fullscreen;
mod island_window;
mod keys;
mod keepawake;
mod media;
mod metrics;
mod notify;
mod providers;
mod reminders;
mod spectrum;
mod state;
mod timers;
mod volume;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

use state::AppState;

/// Single-instance guard. Returns false when another Isle already owns the
/// mutex (we then poke its show-event and exit).
fn ensure_single_instance() -> bool {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::{CreateMutexW, SetEvent};
    use windows::Win32::System::Threading::{OpenEventW, EVENT_MODIFY_STATE};
    use windows::core::w;

    unsafe {
        let mutex = CreateMutexW(None, true, w!("Local\\IsleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Poke the running instance so it flashes the island, then leave.
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\IsleShow")) {
                let _ = SetEvent(event);
            }
            return false;
        }
        // Deliberately leak the HANDLE wrapper: the OS-level mutex stays held
        // until process exit, which is what enforces single-instance.
        let _ = mutex;
        true
    }
}

/// Wait for "show me" pokes from second launches.
fn start_show_watcher(app: tauri::AppHandle) {
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    use windows::core::w;

    std::thread::spawn(move || unsafe {
        let event = match CreateEventW(None, false, false, w!("Local\\IsleShow")) {
            Ok(h) => h,
            Err(_) => return,
        };
        loop {
            WaitForSingleObject(event, INFINITE);
            if let Some(win) = island_window::island(&app) {
                let _ = win.show();
                let _ = win.set_focus();
            }
            events::emit(
                &app,
                events::ev::ATTENTION,
                &serde_json::json!({ "kind": "welcome", "text": "Isle is already running" }),
            );
        }
    });
}

fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "open-settings", "Settings", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle-island", "Show / Hide Island", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Isle", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &toggle, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    TrayIconBuilder::with_id("isle-tray")
        .icon(icon)
        .tooltip("Isle")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open-settings" => show_settings(app),
            "toggle-island" => toggle_island(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Startup diagnostics: log every connected monitor (name, geometry, DPI)
/// so multi-monitor placement issues are diagnosable from debug.log.
fn log_monitors(app: &tauri::AppHandle) {
    match app.available_monitors() {
        Ok(monitors) => {
            let primary = app.primary_monitor().ok().flatten();
            for m in monitors {
                let is_primary = primary
                    .as_ref()
                    .map(|p| p.name() == m.name())
                    .unwrap_or(false);
                crate::events::debug_log(&format!(
                    "[startup] monitor {:?} pos({}, {}) {}x{} scale={} primary={}",
                    m.name(),
                    m.position().x,
                    m.position().y,
                    m.size().width,
                    m.size().height,
                    m.scale_factor(),
                    is_primary
                ));
            }
        }
        Err(err) => crate::events::debug_log(&format!("[startup] monitors: {err}")),
    }
}

fn show_settings(app: &tauri::AppHandle) {
    if let Some(win) = island_window::settings_window(app) {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn toggle_island(app: &tauri::AppHandle) {
    if let Some(win) = island_window::island(app) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
        }
    }
}

pub fn run() {
    if !ensure_single_instance() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::island_set_frame,
            commands::island_expand,
            commands::island_collapse,
            commands::open_settings,
            commands::hide_island,
            commands::show_island,
            commands::quit_app,
            commands::copy_now_playing,
            commands::volume_get,
            commands::volume_set,
            commands::mic_state,
            commands::mic_set_mute_state,
            commands::keep_awake_set,
            commands::keep_awake_off,
            commands::note_get,
            commands::note_save,
            commands::frontend_log,
            commands::update_check,
            commands::update_install,
            commands::island_anchor,
            commands::settings_get,
            commands::settings_update,
            commands::media_current,
            commands::media_play_pause,
            commands::media_next,
            commands::media_prev,
            commands::timer_add,
            commands::timer_cancel,
            commands::timer_pause,
            commands::timer_resume,
            commands::timers_list,
            commands::metrics_subscribe,
            commands::weather_geocode,
            commands::weather_refresh,
            commands::weather_set_city,
            commands::lyrics_fetch,
            commands::media_lookup,
            commands::media_search,
            commands::media_info_cached,
            commands::fetch_data_url,
            commands::clipboard_list,
            commands::clipboard_reveal,
            commands::clipboard_copy,
            commands::clipboard_delete,
            commands::clipboard_clear,
            commands::notifications_list,
            commands::notifications_clear,
            commands::bt_devices,
            commands::bridge_status,
            commands::bridge_set_enabled,
            commands::bridge_regenerate_token,
            commands::providers_set_tmdb,
            commands::providers_clear_tmdb,
            commands::providers_has_tmdb,
            commands::theme_setting,
            commands::expand_mode_setting,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            crate::events::debug_log("[startup] isle launched");
            log_monitors(&handle);
            island_window::create(&handle)?;
            island_window::create_settings(&handle)?;
            create_tray(&handle)?;
            island_window::start_watch(handle.clone());
            start_show_watcher(handle.clone());
            media::start(handle.clone());
            metrics::start(handle.clone());
            timers::start(handle.clone());
            clipboard::start(handle.clone());
            notify::start(handle.clone());
            bluetooth::start(handle.clone());
            fullscreen::start(handle.clone());
            reminders::init(&handle);
            reminders::start(handle.clone());
            spectrum::start(handle.clone());
            keys::start(handle.clone());
            volume::start(handle.clone());
            keepawake::start(handle.clone());
            providers::open_meteo::start_service(handle.clone());
            bridge::start(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Isle failed to start");
}
