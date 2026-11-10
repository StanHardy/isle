use tauri::{AppHandle, Emitter, Manager};

use crate::bluetooth;
use crate::clipboard::{self, ClipEntry};
use crate::config::{ExpandMode, Settings, Theme, Units};
use crate::credentials;
use crate::events::{self, ev};
use crate::island_window;
use crate::media::{self, MediaState};
use crate::notify::NotifyItem;
use crate::providers::{lrclib, open_meteo, tmdb};
use crate::state::AppState;
use crate::timers::{self, Timer};

// ---- island window ----

#[tauri::command]
pub fn island_set_frame(app: AppHandle, x: f64, y: f64, w: f64, h: f64) {
    if let Some(win) = island_window::island(&app) {
        let _ = win.set_size(tauri::LogicalSize::new(w.max(40.0), h.max(20.0)));
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
    }
}

#[tauri::command]
pub fn island_expand(app: AppHandle) {
    island_window::set_noactivate(&app, false);
}

#[tauri::command]
pub fn island_collapse(app: AppHandle) {
    island_window::set_noactivate(&app, true);
    if let Some(win) = island_window::island(&app) {
        let _ = win.set_focus();
        // Hand focus straight back to the desktop by clearing our own claim.
        let _ = win.hide();
        let _ = win.show();
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    if let Some(win) = island_window::settings_window(&app) {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
pub fn hide_island(app: AppHandle) {
    if let Some(win) = island_window::island(&app) {
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn show_island(app: AppHandle) {
    if let Some(win) = island_window::island(&app) {
        let _ = win.show();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn island_anchor(app: AppHandle) -> island_window::Anchor {
    island_window::anchor(&app)
}

// ---- settings ----

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Settings {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.clone()
}

#[tauri::command]
pub fn settings_update(app: AppHandle, patch: serde_json::Value) -> Settings {
    let state: tauri::State<AppState> = app.state();
    let mut next = {
        let current = state.settings.read().expect("settings lock");
        serde_json::to_value(&*current).expect("settings serialize")
    };
    merge_into(&mut next, patch);
    let mut settings: Settings = serde_json::from_value(next).unwrap_or_default();
    // Protected/derived fields.
    let (tmdb_cfg, bridge_token) = {
        let current = state.settings.read().expect("settings lock");
        (current.tmdb_configured, current.bridge_token.clone())
    };
    settings.tmdb_configured = tmdb_cfg;
    settings.bridge_token = bridge_token;
    settings.island_scale = settings.island_scale.clamp(0.6, 1.5);
    settings.clipboard_keep = settings.clipboard_keep.clamp(10, 500);
    *state.settings.write().expect("settings lock") = settings.clone();
    settings.save();
    let _ = app.emit(ev::DISPLAY, island_window::anchor(&app));
    settings
}

fn merge_into(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base.take(), patch) {
        (serde_json::Value::Object(mut a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(mut existing) => {
                        merge_into(&mut existing, v);
                        existing
                    }
                    None => v,
                };
                a.insert(k, merged);
            }
            *base = serde_json::Value::Object(a);
        }
        (_, patch) => *base = patch,
    }
}

// ---- media ----

#[tauri::command]
pub fn media_current(app: AppHandle) -> Option<MediaState> {
    let state: tauri::State<AppState> = app.state();
    let m = state.media.lock().expect("media state lock");
    m.clone()
}

#[tauri::command]
pub fn media_play_pause(_app: AppHandle) {
    media::play_pause();
}

#[tauri::command]
pub fn media_next(_app: AppHandle) {
    media::next();
}

#[tauri::command]
pub fn media_prev(_app: AppHandle) {
    media::prev();
}

// ---- timers ----

#[tauri::command]
pub fn timer_add(app: AppHandle, label: String, seconds: u64) -> u64 {
    timers::add(&app, &label, seconds.max(1))
}

#[tauri::command]
pub fn timer_cancel(app: AppHandle, id: u64) -> bool {
    let state: tauri::State<AppState> = app.state();
    let mut t = state.timers.lock().expect("timers lock");
    t.remove(&id).is_some()
}

#[tauri::command]
pub fn timer_pause(app: AppHandle, id: u64) -> bool {
    let state: tauri::State<AppState> = app.state();
    let now = timers::now_ms();
    let mut timers = state.timers.lock().expect("timers lock");
    match timers.get_mut(&id) {
        Some(t) if !t.paused => {
            t.remaining_ms = t.ends_at_ms.saturating_sub(now);
            t.paused = true;
            true
        }
        _ => false,
    }
}

#[tauri::command]
pub fn timer_resume(app: AppHandle, id: u64) -> bool {
    let state: tauri::State<AppState> = app.state();
    let mut timers = state.timers.lock().expect("timers lock");
    match timers.get_mut(&id) {
        Some(t) if t.paused => {
            t.ends_at_ms = timers::now_ms() + t.remaining_ms;
            t.paused = false;
            true
        }
        _ => false,
    }
}

#[tauri::command]
pub fn timers_list(app: AppHandle) -> Vec<Timer> {
    let state: tauri::State<AppState> = app.state();
    let mut timers = state.timers.lock().expect("timers lock");
    let now = timers::now_ms();
    let list: Vec<Timer> = timers
        .values_mut()
        .map(|t| {
            if !t.paused {
                t.remaining_ms = t.ends_at_ms.saturating_sub(now);
            }
            t.clone()
        })
        .collect();
    list
}

// ---- metrics ----

#[tauri::command]
pub fn metrics_subscribe(app: AppHandle, enable: bool) {
    use std::sync::atomic::Ordering;
    let state: tauri::State<AppState> = app.state();
    state
        .metrics_subscribed
        .store(enable, Ordering::Relaxed);
}

// ---- weather ----

#[tauri::command]
pub async fn weather_geocode(query: String) -> Result<Vec<open_meteo::GeoResult>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    open_meteo::geocode(query.trim()).await
}

#[tauri::command]
pub async fn weather_refresh(app: AppHandle) -> Result<serde_json::Value, String> {
    let (city, units) = {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        (s.weather_city.clone(), s.units)
    };
    let Some(city) = city else {
        return Ok(serde_json::json!({ "error": "no city selected" }));
    };
    match open_meteo::fetch(&city, &units).await {
        Ok(data) => {
            events::emit(&app, ev::WEATHER, &data);
            Ok(serde_json::to_value(&data).expect("weather serialize"))
        }
        Err(err) => Err(err),
    }
}

#[tauri::command]
pub fn weather_set_city(app: AppHandle, city: open_meteo::GeoResult) -> Settings {
    let state: tauri::State<AppState> = app.state();
    {
        let mut s = state.settings.write().expect("settings lock");
        s.weather_city = Some(crate::config::CityRef {
            name: city.name.clone(),
            lat: city.lat,
            lon: city.lon,
        });
        s.save();
        s.clone()
    }
}

// ---- lyrics ----

#[tauri::command]
pub async fn lyrics_fetch(
    artist: String,
    title: String,
    duration_s: u64,
    album: String,
) -> Result<Option<lrclib::LyricsPayload>, String> {
    if title.trim().is_empty() {
        return Ok(None);
    }
    lrclib::fetch(artist.trim(), title.trim(), duration_s, album.trim()).await
}

// ---- media info (TV/movie) ----

#[tauri::command]
pub async fn media_lookup(
    app: AppHandle,
    kind: String,
    query: Option<String>,
) -> Result<Option<tmdb::MediaInfo>, String> {
    let tmdb_key = credentials::get_secret("Isle:tmdb");
    let state: tauri::State<AppState> = app.state();
    match kind.as_str() {
        "auto" => {
            let snapshot = state.media.lock().expect("media state lock").clone();
            let Some(m) = snapshot else { return Ok(None) };
            let info = tmdb::lookup_auto(&m, tmdb_key.as_deref()).await;
            let key = m.art_key.clone();
            if let Some(info) = info.clone() {
                state
                    .media_info
                    .lock()
                    .expect("media info lock")
                    .insert(key, info);
            }
            Ok(info)
        }
        "tv" => {
            let q = query.unwrap_or_default();
            let Some((show, season, ep)) = tmdb::parse_episode_title(&q) else {
                return Ok(None);
            };
            tmdb::lookup_tv(&show, season, ep).await
        }
        "movie" => {
            let q = query.unwrap_or_default();
            let (title, year) = tmdb::split_year(&q);
            if let Some(key) = tmdb_key {
                tmdb::lookup_movie_tmdb(&title, year, &key).await
            } else {
                tmdb::lookup_movie_itunes(&title).await
            }
        }
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn media_search(
    kind: String,
    query: String,
) -> Result<Vec<tmdb::SearchHit>, String> {
    let key = credentials::get_secret("Isle:tmdb");
    tmdb::search(&kind, query.trim(), key.as_deref()).await
}

// ---- clipboard ----

#[tauri::command]
pub fn clipboard_list(app: AppHandle) -> Vec<ClipEntry> {
    let state: tauri::State<AppState> = app.state();
    let store = state.clipboard.lock().expect("clipboard lock");
    clipboard::masked_list(&store)
}

#[tauri::command]
pub fn clipboard_reveal(app: AppHandle, id: u64) -> Option<String> {
    clipboard::full_text(&app, id)
}

#[tauri::command]
pub fn clipboard_copy(app: AppHandle, id: u64) -> bool {
    let Some(text) = clipboard::full_text(&app, id) else {
        return false;
    };
    clipboard::set_text(&text)
}

#[tauri::command]
pub fn clipboard_delete(app: AppHandle, id: u64) -> bool {
    let state: tauri::State<AppState> = app.state();
    let mut store = state.clipboard.lock().expect("clipboard lock");
    let before = store.entries.len();
    store.entries.retain(|e| e.id != id);
    before != store.entries.len()
}

#[tauri::command]
pub fn clipboard_clear(app: AppHandle) {
    let state: tauri::State<AppState> = app.state();
    let mut store = state.clipboard.lock().expect("clipboard lock");
    store.entries.clear();
    let _ = app;
}

// ---- notifications ----

#[tauri::command]
pub fn notifications_list(app: AppHandle) -> Vec<NotifyItem> {
    let state: tauri::State<AppState> = app.state();
    let list = state.notifications.lock().expect("notifications lock");
    list.clone()
}

#[tauri::command]
pub fn notifications_clear(app: AppHandle) {
    let state: tauri::State<AppState> = app.state();
    state.notifications.lock().expect("notifications lock").clear();
}

// ---- bluetooth ----

#[tauri::command]
pub fn bt_devices(app: AppHandle) -> Vec<bluetooth::BtDevice> {
    bluetooth::devices(&app)
}

// ---- bridge ----

#[tauri::command]
pub fn bridge_status(app: AppHandle) -> serde_json::Value {
    crate::bridge::status(&app)
}

#[tauri::command]
pub fn bridge_set_enabled(app: AppHandle, enable: bool) -> serde_json::Value {
    let state: tauri::State<AppState> = app.state();
    {
        let mut s = state.settings.write().expect("settings lock");
        s.bridge_enabled = enable;
        s.save();
    }
    crate::bridge::status(&app)
}

#[tauri::command]
pub fn bridge_regenerate_token(app: AppHandle) -> serde_json::Value {
    let state: tauri::State<AppState> = app.state();
    {
        let mut s = state.settings.write().expect("settings lock");
        s.bridge_token = crate::config::generate_token();
        s.save();
    }
    crate::bridge::status(&app)
}

// ---- providers ----

#[tauri::command]
pub fn providers_set_tmdb(app: AppHandle, key: String) -> Result<(), String> {
    credentials::set_secret("Isle:tmdb", key.trim())?;
    let state: tauri::State<AppState> = app.state();
    let mut s = state.settings.write().expect("settings lock");
    s.tmdb_configured = true;
    s.save();
    Ok(())
}

#[tauri::command]
pub fn providers_clear_tmdb(app: AppHandle) -> Result<(), String> {
    credentials::delete_secret("Isle:tmdb")?;
    let state: tauri::State<AppState> = app.state();
    let mut s = state.settings.write().expect("settings lock");
    s.tmdb_configured = false;
    s.save();
    Ok(())
}

#[tauri::command]
pub fn providers_has_tmdb(app: AppHandle) -> bool {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.tmdb_configured
}

// theme helper for FE (system dark detection)
#[tauri::command]
pub fn theme_setting(app: AppHandle) -> Theme {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.theme
}

#[tauri::command]
pub fn units_setting(app: AppHandle) -> Units {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.units
}

#[tauri::command]
pub fn expand_mode_setting(app: AppHandle) -> ExpandMode {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.expand_mode
}
