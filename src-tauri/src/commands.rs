use base64::Engine as _;
use tauri::{AppHandle, Emitter, Manager};

use crate::bluetooth;
use crate::clipboard::{self, ClipEntry};
use crate::config::{ExpandMode, Settings, Theme};
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
pub fn island_set_frame(app: AppHandle, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    if let Some(win) = island_window::island(&app) {
        let _ = win.set_size(tauri::LogicalSize::new(w.max(40.0), h.max(20.0)));
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
        island_window::apply_round_region(&win, radius);
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

/// Frontend error reporting → debug.log (windowed app has no console).
#[tauri::command]
pub fn frontend_log(app: AppHandle, message: String) {
    crate::events::debug_log(&format!("[fe] {}", message.chars().take(2000).collect::<String>()));
    let _ = app;
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
pub fn settings_update(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    let state: tauri::State<AppState> = app.state();
    let mut next = {
        let current = state.settings.read().expect("settings lock");
        serde_json::to_value(&*current).expect("settings serialize")
    };
    merge_into(&mut next, patch);
    // A malformed patch must be REJECTED, not silently reset to defaults —
    // unwrap_or_default here used to wipe the user's whole configuration.
    let mut settings: Settings = serde_json::from_value(next).map_err(|err| {
        crate::events::debug_log(&format!("[settings] rejected invalid patch: {err}"));
        format!("invalid settings patch: {err}")
    })?;
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
    // Broadcast to every window: the island only fetched settings once at
    // startup, so changes made in the settings window must be pushed to it.
    let _ = app.emit(ev::SETTINGS, &settings);
    let _ = app.emit(ev::DISPLAY, island_window::anchor(&app));
    Ok(settings)
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
    let (enabled, city, temp_unit, wind_unit) = {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        (s.weather_enabled, s.weather_city.clone(), s.temp_unit, s.wind_unit)
    };
    if !enabled {
        return Ok(serde_json::json!({ "error": "weather disabled in settings" }));
    }
    let Some(city) = city else {
        return Ok(serde_json::json!({ "error": "no city selected" }));
    };
    match open_meteo::fetch(&city, &temp_unit, &wind_unit).await {
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
            // Serve from cache first — the background auto-lookup may have
            // already fetched this entry.
            {
                let cache = state.media_info.lock().expect("media info lock");
                if let Some(info) = crate::state::media_info_get(&cache, &m.art_key) {
                    return Ok(Some(info));
                }
            }
            let info = tmdb::lookup_auto(&m, tmdb_key.as_deref()).await;
            if let Some(info) = info.clone() {
                let mut cache = state.media_info.lock().expect("media info lock");
                crate::state::media_info_insert(&mut cache, m.art_key.clone(), info.clone());
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

/// Cached info for the currently playing media, if any (no network).
#[tauri::command]
pub fn media_info_cached(app: AppHandle) -> Option<tmdb::MediaInfo> {
    let state: tauri::State<AppState> = app.state();
    let m = state.media.lock().expect("media state lock").clone()?;
    let cache = state.media_info.lock().expect("media info lock");
    crate::state::media_info_get(&cache, &m.art_key)
}

/// Shared HTTP client for ad-hoc fetches — reqwest::get has NO default
/// timeout and could hang the async command indefinitely.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client builds")
    })
}

/// Fetch a remote image into a data URL so the frontend can canvas-sample
/// it without CORS taint (used for album/poster tint). Capped at 3 MB and
/// domain-allowlisted to the providers Isle actually uses — this command is
/// IPC-exposed, so it must not become a general-purpose proxy.
const IMAGE_HOST_ALLOWLIST: &[&str] = &[
    "static.tvmaze.com",
    "image.tmdb.org",
    "is1-ssl.mzstatic.com",
    "is2-ssl.mzstatic.com",
    "is3-ssl.mzstatic.com",
    "is4-ssl.mzstatic.com",
    "is5-ssl.mzstatic.com",
];

fn image_host_allowed(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    IMAGE_HOST_ALLOWLIST
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Fetch a remote image into a data URL so the frontend can canvas-sample
/// it without CORS taint (used for album/poster tint). Capped at 3 MB.
#[tauri::command]
pub async fn fetch_data_url(url: String) -> Result<String, String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("only http(s) urls allowed".into());
    }
    if !image_host_allowed(&url) {
        return Err("image host not allowlisted".into());
    }
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 3 * 1024 * 1024 {
        return Err("image too large".into());
    }
    if !mime.starts_with("image/") {
        return Err("not an image".into());
    }
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Copy "title — artist" of the playing media to the clipboard.
#[tauri::command]
pub fn copy_now_playing(app: AppHandle) -> Result<bool, String> {
    let state: tauri::State<AppState> = app.state();
    let m = state.media.lock().expect("media state lock").clone();
    let Some(m) = m else { return Ok(false) };
    if m.title.is_empty() {
        return Ok(false);
    }
    let text = format!("{} — {}", m.title, m.artist);
    Ok(crate::clipboard::set_text(&text))
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

// ---- updater ----

/// Check the update endpoint. Fails with a readable error when the release
/// feed doesn't exist yet (the repo isn't published) — the UI surfaces that.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    Ok(match update {
        Some(u) => serde_json::json!({
            "available": true,
            "version": u.version,
            "notes": u.body.clone().unwrap_or_default(),
        }),
        None => serde_json::json!({ "available": false }),
    })
}

/// Download + install the pending update, then restart the app.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no update available")?;
    update
        .download_and_install(
            |downloaded, total| {
                crate::events::debug_log(&format!(
                    "[updater] downloaded {downloaded}/{}",
                    total.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
                ));
            },
            || {
                crate::events::debug_log("[updater] download finished");
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    crate::events::debug_log("[updater] installed — restarting");
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

// theme helper for FE (system dark detection)
#[tauri::command]
pub fn theme_setting(app: AppHandle) -> Theme {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.theme
}

#[tauri::command]
pub fn expand_mode_setting(app: AppHandle) -> ExpandMode {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    s.expand_mode
}
