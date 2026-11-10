//! SMTC media integration.
//!
//! windows-rs COM interfaces are not `Send`, so every WinRT interaction runs
//! on a blocking worker thread (no `.await` across interface values). The
//! SMTC manager is a per-process singleton, re-acquired per work item, so no
//! interface state needs to be shared between threads.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    SessionsChangedEventArgs,
};
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};

use crate::events::{self, ev};
use crate::state::AppState;

static APP: OnceLock<AppHandle> = OnceLock::new();
/// Art is only re-sent when the media key changes; the frontend caches by key.
static LAST_ART_KEY: Mutex<String> = Mutex::new(String::new());
/// Retry bookkeeping for artwork that failed to download: (key, attempts).
/// A failed fetch must NOT lock the key in (Spotify occasionally serves a
/// stream that fails once) — we reset the key so the 2 s poll retries, up
/// to three attempts per track.
static ART_RETRY: Mutex<(String, u32)> = Mutex::new((String::new(), 0));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub id: String,
    pub app: String,
    /// Display title (prettified for episode-style filenames).
    pub title: String,
    /// Untouched SMTC title — used for provider lookups, never displayed.
    pub raw_title: String,
    pub artist: String,
    pub album: String,
    pub status: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub art_key: String,
    pub art_b64: Option<String>,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub shuffle: Option<bool>,
    pub repeat: Option<String>,
}

fn ensure_com() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
}

/// Block until a WinRT async operation completes (windows-future 0.3 has no
/// blocking `get`; this mirrors it with a completion handler + channel wait).
fn wait<T>(op: windows_future::IAsyncOperation<T>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType,
{
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx, rx) = mpsc::channel::<()>();
    let handler = windows_future::AsyncOperationCompletedHandler::<T>::new(move |_, _| {
        let _ = tx.send(());
        Ok(())
    });
    op.SetCompleted(&handler)?;
    if rx.recv_timeout(Duration::from_secs(30)).is_err() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004005u32 as i32, // E_FAIL
        )));
    }
    op.GetResults()
}

/// Same as [`wait`] for progress-carrying operations.
fn wait_p<T, P>(op: windows_future::IAsyncOperationWithProgress<T, P>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType,
    P: windows::core::RuntimeType,
{
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx, rx) = mpsc::channel::<()>();
    let handler = windows_future::AsyncOperationWithProgressCompletedHandler::<T, P>::new(
        move |_, _| {
            let _ = tx.send(());
            Ok(())
        },
    );
    op.SetCompleted(&handler)?;
    if rx.recv_timeout(Duration::from_secs(30)).is_err() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004005u32 as i32,
        )));
    }
    op.GetResults()
}

fn request_manager() -> windows::core::Result<GlobalSystemMediaTransportControlsSessionManager> {
    wait(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?)
}

/// Per-session playback activity: last observed position and when it was
/// last seen ADVANCING while Playing. Some players (mpc-hc) keep status
/// "Playing" at end-of-file with a frozen position — position movement is
/// the only reliable signal of a genuinely active session.
static SESSION_ACTIVITY: OnceLock<Mutex<std::collections::HashMap<String, (u64, Instant)>>> =
    OnceLock::new();
const ACTIVITY_WINDOW: Duration = Duration::from_secs(3);

fn session_activity() -> &'static Mutex<std::collections::HashMap<String, (u64, Instant)>> {
    SESSION_ACTIVITY.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Pick the current session: prefer one whose position is actually
/// advancing, then any Playing session, then the first session.
fn pick_session(
    mgr: &GlobalSystemMediaTransportControlsSessionManager,
) -> windows::core::Result<Option<GlobalSystemMediaTransportControlsSession>> {
    let sessions = mgr.GetSessions()?;
    let count = sessions.Size()?;
    let now = Instant::now();
    let mut best: Option<(u8, GlobalSystemMediaTransportControlsSession, Instant)> = None;
    {
        let mut activity = session_activity().lock().expect("session activity lock");
        for i in 0..count {
            let s = sessions.GetAt(i)?;
            let id = s.SourceAppUserModelId().map(|x| x.to_string()).unwrap_or_default();
            let status = s
                .GetPlaybackInfo()
                .and_then(|p| p.PlaybackStatus())
                .unwrap_or(PlaybackStatus::Stopped);
            let (pos_ms, end_ms) = match s.GetTimelineProperties() {
                Ok(t) => (
                    t.Position().map(|p| (p.Duration / 10_000).max(0) as u64).unwrap_or(0),
                    t.EndTime().map(|e| (e.Duration / 10_000).max(0) as u64).unwrap_or(0),
                ),
                Err(_) => (0, 0),
            };
            let entry = activity.entry(id).or_insert((u64::MAX, now));
            let moved = pos_ms != entry.0;
            if moved && status == PlaybackStatus::Playing {
                entry.0 = pos_ms;
                entry.1 = now;
            } else if moved {
                // Position changed while not Playing (seek while paused) —
                // track it so a stale record doesn't linger.
                entry.0 = pos_ms;
            }
            // Score: actively advancing > playing > paused > ended-at-end.
            // Some players (mpc-hc) hold status Playing at end-of-file with a
            // frozen position; that must not beat a paused session.
            let ended = end_ms > 0 && pos_ms + 1000 >= end_ms;
            let score: u8 = if entry.1.elapsed() < ACTIVITY_WINDOW {
                3
            } else if status == PlaybackStatus::Playing && !ended {
                2
            } else if status == PlaybackStatus::Playing {
                0
            } else {
                1
            };
            let replace = match &best {
                None => true,
                Some((best_score, _, best_time)) => {
                    score > *best_score
                        || (score == *best_score && entry.1 > *best_time)
                }
            };
            if replace {
                best = Some((score, s, entry.1));
            }
        }
        if activity.len() > 16 {
            activity.clear();
        }
    }
    Ok(best.map(|(_, s, _)| Some(s)).unwrap_or(None))
}

pub fn start(app: AppHandle) {
    let _ = APP.set(app.clone());
    tauri::async_runtime::spawn_blocking(move || {
        ensure_com();
        if let Err(err) = subscribe(&app) {
            eprintln!("[media] subscribe failed: {err}");
        }
        attach_all(&app);
        if let Err(err) = refresh(&app) {
            eprintln!("[media] initial refresh failed: {err}");
        }
        // Poll safety net: a session whose status flips (e.g. Spotify goes
        // from paused to playing) does NOT fire SessionsChanged — it already
        // existed — and if we never picked it, it has no per-session
        // handlers. Polling re-picks the Playing session within 2 s no
        // matter what.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            ensure_com();
            if let Err(err) = refresh(&app) {
                eprintln!("[media] poll refresh failed: {err}");
            }
        }
    });
}

fn subscribe(app: &AppHandle) -> windows::core::Result<()> {
    let mgr = request_manager()?;
    let handler_app = app.clone();
    let handler = TypedEventHandler::<
        GlobalSystemMediaTransportControlsSessionManager,
        SessionsChangedEventArgs,
    >::new(move |_, _| {
        let app = handler_app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            ensure_com();
            // The session set changed: re-attach handlers to every session
            // (any of them may become the picked one later) and re-pick.
            attach_all(&app);
            if let Err(err) = refresh(&app) {
                eprintln!("[media] refresh failed: {err}");
            }
        });
        Ok(())
    });
    mgr.SessionsChanged(&handler)?;
    Ok(())
}

/// Attach change handlers to EVERY session so that any session's metadata /
/// playback / timeline change re-picks the current one — including sessions
/// that are paused now but start playing later (the Spotify case).
fn attach_all(app: &AppHandle) {
    let mgr = match request_manager() {
        Ok(m) => m,
        Err(err) => {
            eprintln!("[media] attach_all failed: {err}");
            return;
        }
    };
    let sessions = match mgr.GetSessions() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[media] attach_all sessions failed: {err}");
            return;
        }
    };
    let count = sessions.Size().unwrap_or(0);
    for i in 0..count {
        if let Ok(s) = sessions.GetAt(i) {
            attach_handlers(&s, app);
        }
    }
}

/// Track art-fetch success/failure; a failed fetch releases the key so the
/// 2 s poll retries (max 3 attempts per track).
fn note_art_result(key: &str, want_art: bool, got_art: bool) {
    if !want_art {
        return;
    }
    if got_art {
        *ART_RETRY.lock().expect("art retry lock") = (key.to_string(), 0);
        return;
    }
    let mut retry = ART_RETRY.lock().expect("art retry lock");
    let attempts = if retry.0 == key { retry.1 + 1 } else { 1 };
    *retry = (key.to_string(), attempts);
    if attempts < 3 {
        *LAST_ART_KEY.lock().expect("art key lock") = String::new();
        crate::events::debug_log(&format!(
            "[media] art fetch failed (attempt {attempts}) — retrying: {key}"
        ));
    } else {
        crate::events::debug_log(&format!(
            "[media] art fetch failed {attempts}x — giving up: {key}"
        ));
    }
}

/// Re-pick the current session and emit fresh state (blocking).
fn refresh(app: &AppHandle) -> windows::core::Result<()> {
    let mgr = request_manager()?;
    let session = pick_session(&mgr)?;
    match session {
        Some(s) => {
            let want_art = art_key_changed(&s)?;
            let state = build_state(&s, want_art)?;
            let key = state.art_key.clone();
            let got_art = state.art_b64.is_some();
            update_and_emit(app, Some(state));
            note_art_result(&key, want_art, got_art);
            if want_art {
                schedule_info_lookup(app, key);
            }
        }
        None => {
            *LAST_ART_KEY.lock().expect("art key lock") = String::new();
            update_and_emit(app, None);
        }
    }
    Ok(())
}

/// Background TV/movie lookup for video-looking media (episode or year
/// titles). Debounced and re-validated so fast track skips don't stack
/// stale fetches; result is cached and pushed via media:info.
fn schedule_info_lookup(app: &AppHandle, key: String) {
    let enabled = {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        s.media_info_auto
    };
    if !enabled {
        return;
    }
    let raw_title = {
        let state: tauri::State<AppState> = app.state();
        let m = state.media.lock().expect("media state lock");
        match m.as_ref() {
            Some(m) => m.raw_title.clone(),
            None => return,
        }
    };
    if !crate::providers::tmdb::looks_like_video(&raw_title) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        ensure_com();
        let state: tauri::State<AppState> = app.state();
        let current_key = {
            let m = state.media.lock().expect("media state lock");
            match m.as_ref() {
                Some(m) => m.art_key.clone(),
                None => return,
            }
        };
        if current_key != key {
            return; // track changed again while we waited
        }
        let snapshot = {
            let m = state.media.lock().expect("media state lock");
            m.clone()
        };
        let Some(m) = snapshot else { return };
        let tmdb_key = crate::credentials::get_secret("Isle:tmdb");
        let info =
            tauri::async_runtime::block_on(crate::providers::tmdb::lookup_auto(&m, tmdb_key.as_deref()));
        if let Some(info) = info {
            {
                let mut cache = state.media_info.lock().expect("media info lock");
                crate::state::media_info_insert(&mut cache, key.clone(), info.clone());
            }
            events::emit(
                &app,
                ev::MEDIA_INFO,
                &serde_json::json!({ "key": key, "info": info }),
            );
        }
    });
}

/// Update the cached art key; true when artwork should be re-fetched.
fn art_key_changed(
    s: &GlobalSystemMediaTransportControlsSession,
) -> windows::core::Result<bool> {
    let props = wait(s.TryGetMediaPropertiesAsync()?)?;
    let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|t| t.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|t| t.to_string()).unwrap_or_default();
    let duration = session_duration_ms(s).unwrap_or(0);
    let key = format!("{title}|{artist}|{album}|{duration}");
    let mut last = LAST_ART_KEY.lock().expect("art key lock");
    let changed = *last != key;
    if changed {
        *last = key;
    }
    Ok(changed)
}

/// Per-session event handlers; duplicate handlers are harmless because
/// update_and_emit dedupes identical non-art payloads.
fn attach_handlers(s: &GlobalSystemMediaTransportControlsSession, app: &AppHandle) {
    let h1 = TypedEventHandler::<
        GlobalSystemMediaTransportControlsSession,
        windows::Media::Control::MediaPropertiesChangedEventArgs,
    >::new({
        let app = app.clone();
        move |_, _| {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                ensure_com();
                if let Err(err) = emit_current(&app, false) {
                    eprintln!("[media] emit failed: {err}");
                }
            });
            Ok(())
        }
    });
    let h2 = TypedEventHandler::<
        GlobalSystemMediaTransportControlsSession,
        windows::Media::Control::PlaybackInfoChangedEventArgs,
    >::new({
        let app = app.clone();
        move |_, _| {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                ensure_com();
                if let Err(err) = emit_current(&app, false) {
                    eprintln!("[media] emit failed: {err}");
                }
            });
            Ok(())
        }
    });
    let h3 = TypedEventHandler::<
        GlobalSystemMediaTransportControlsSession,
        windows::Media::Control::TimelinePropertiesChangedEventArgs,
    >::new({
        let app = app.clone();
        move |_, _| {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                ensure_com();
                if let Err(err) = emit_current(&app, false) {
                    eprintln!("[media] emit failed: {err}");
                }
            });
            Ok(())
        }
    });
    let _ = s.MediaPropertiesChanged(&h1);
    let _ = s.PlaybackInfoChanged(&h2);
    let _ = s.TimelinePropertiesChanged(&h3);
}

/// Re-emit the current session state (with art when the key changed).
fn emit_current(app: &AppHandle, force_art: bool) -> windows::core::Result<()> {
    let mgr = request_manager()?;
    let session = pick_session(&mgr)?;
    match session {
        Some(s) => {
            let want_art = if force_art {
                true
            } else {
                art_key_changed(&s)?
            };
            let state = build_state(&s, want_art)?;
            let key = state.art_key.clone();
            let got_art = state.art_b64.is_some();
            update_and_emit(app, Some(state));
            note_art_result(&key, want_art, got_art);
            if want_art {
                schedule_info_lookup(app, key);
            }
        }
        None => {
            *LAST_ART_KEY.lock().expect("art key lock") = String::new();
            update_and_emit(app, None);
        }
    }
    Ok(())
}

/// "Suits S05E02.Compensation.mkv" → "Suits · S05E02 — Compensation".
/// Local players (mpc-hc) expose the raw filename via SMTC; this prettifies
/// episode-style names while leaving normal titles untouched.
fn display_title(raw: &str) -> String {
    let base = raw.trim();
    const EXTS: [&str; 9] = ["mkv", "mp4", "avi", "mov", "webm", "ts", "m2ts", "wmv", "flv"];
    let lower = base.to_lowercase();
    let mut no_ext = base;
    for ext in EXTS {
        if lower.ends_with(&format!(".{ext}")) {
            no_ext = &base[..base.len() - ext.len() - 1];
            break;
        }
    }
    let bytes = no_ext.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let is_s = bytes[i] == b's' || bytes[i] == b'S';
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if is_s && boundary && bytes[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() && j - i <= 2 {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'e' || bytes[j] == b'E') {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() && k - j <= 3 {
                    k += 1;
                }
                if k > j + 1 {
                    if let (Ok(season), Ok(ep)) = (
                        no_ext[i + 1..j].parse::<u32>(),
                        no_ext[j + 1..k].parse::<u32>(),
                    ) {
                        if season > 0 && ep > 0 {
                            let show = crate::providers::tmdb::clean_show(&no_ext[..i]);
                            let rest = crate::providers::tmdb::clean_show(&no_ext[k..]);
                            let mut out = format!("{show} · S{season:02}E{ep:02}");
                            if !rest.is_empty() {
                                out.push_str(&format!(" — {rest}"));
                            }
                            return out;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    base.to_string()
}

fn build_state(
    s: &GlobalSystemMediaTransportControlsSession,
    want_art: bool,
) -> windows::core::Result<MediaState> {
    let props = wait(s.TryGetMediaPropertiesAsync()?)?;
    let raw_title = props.Title().map(|t| t.to_string()).unwrap_or_default();
    let title = display_title(&raw_title);
    let artist = props.Artist().map(|t| t.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|t| t.to_string()).unwrap_or_default();
    let app_id = s.SourceAppUserModelId()?.to_string();

    let playback = s.GetPlaybackInfo()?;
    let status = playback.PlaybackStatus()?;
    let ctl = playback.Controls()?;
    let can_play = ctl.IsPlayEnabled().unwrap_or(false);
    let can_pause = ctl.IsPauseEnabled().unwrap_or(false);
    let can_next = ctl.IsNextEnabled().unwrap_or(false);
    let can_prev = ctl.IsPreviousEnabled().unwrap_or(false);
    let shuffle = playback.IsShuffleActive().ok().and_then(|r| r.Value().ok());
    let repeat = playback
        .AutoRepeatMode()
        .ok()
        .and_then(|r| r.Value().ok())
        .map(|m| match m.0 {
            1 => "track".to_string(),
            2 => "list".to_string(),
            _ => "none".to_string(),
        });

    let duration_ms = session_duration_ms(s).unwrap_or(0);
    let position_ms = s
        .GetTimelineProperties()
        .and_then(|t| t.Position())
        .map(|p| (p.Duration / 10_000).max(0) as u64)
        .unwrap_or(0);

    let art_key = format!("{title}|{artist}|{album}|{duration_ms}");
    let mut art_b64 = None;
    if want_art {
        if let Ok(thumb) = props.Thumbnail() {
            art_b64 = read_art(thumb).ok().map(|(mime, bytes)| {
                format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            });
        }
    }

    Ok(MediaState {
        id: app_id.clone(),
        app: pretty_app(&app_id),
        title,
        raw_title,
        artist,
        album,
        status: status_str(status).to_string(),
        position_ms,
        duration_ms,
        art_key,
        art_b64,
        can_play,
        can_pause,
        can_next,
        can_prev,
        shuffle,
        repeat,
    })
}

fn session_duration_ms(s: &GlobalSystemMediaTransportControlsSession) -> Option<u64> {
    s.GetTimelineProperties()
        .and_then(|t| t.EndTime())
        .ok()
        .map(|t| (t.Duration / 10_000).max(0) as u64)
}

fn status_str(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "playing",
        PlaybackStatus::Paused => "paused",
        PlaybackStatus::Stopped => "stopped",
        PlaybackStatus::Closed => "closed",
        PlaybackStatus::Opened => "opened",
        PlaybackStatus::Changing => "changing",
        other => match other.0 {
            4 => "playing",
            5 => "paused",
            _ => "unknown",
        },
    }
}

/// "Microsoft.ZuneMusic_8wekyb3d8bbwe!..." → "ZuneMusic"
fn pretty_app(id: &str) -> String {
    let name = id.split('!').next().unwrap_or(id);
    let name = name.strip_prefix("Microsoft.").unwrap_or(name);
    match name.rsplit_once('_') {
        Some((base, _)) => base.to_string(),
        None => name.to_string(),
    }
}

/// Download the thumbnail stream into (mime, bytes); runs on a blocking thread.
/// NOTE: DataReader::FromBuffer needs no LoadAsync — the buffer is already
/// fully populated, and calling LoadAsync on it fails outright for some
/// providers (Spotify: 0x800710DD "operation identifier is not valid").
fn read_art(
    stream_ref: windows::Storage::Streams::IRandomAccessStreamReference,
) -> windows::core::Result<(String, Vec<u8>)> {
    let stream = wait(stream_ref.OpenReadAsync()?)?;
    let mime_declared = stream.ContentType().map(|c| c.to_string()).unwrap_or_default();
    let size = (stream.Size()? as u32).min(4_000_000);
    let buf = Buffer::Create(size)?;
    let read = wait_p(stream.ReadAsync(&buf, size, InputStreamOptions::ReadAhead)?)?;
    let len = read.Length()? as usize;
    let reader = DataReader::FromBuffer(&read)?;
    let mut bytes = vec![0u8; len];
    reader.ReadBytes(&mut bytes)?;
    let mime = if !mime_declared.is_empty() {
        mime_declared
    } else {
        detect_mime(&bytes).to_string()
    };
    Ok((mime, bytes))
}

fn detect_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

/// Identity of a media snapshot ignoring artwork bytes (art arrives on key
/// change only). Used to suppress duplicate emits — duplicate per-session
/// handlers and the poll can both produce identical states.
fn fingerprint(s: &MediaState) -> serde_json::Value {
    let mut v = serde_json::to_value(s).expect("media serialize");
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "artB64".into(),
            serde_json::Value::Bool(s.art_b64.is_some()),
        );
    }
    v
}

fn update_and_emit(app: &AppHandle, state: Option<MediaState>) {
    let changed;
    {
        let state_ref: tauri::State<AppState> = app.state();
        let mut guard = state_ref.media.lock().expect("media state lock");
        changed = match (&*guard, &state) {
            (Some(prev), Some(next)) => fingerprint(prev) != fingerprint(next),
            (None, None) => false,
            _ => true,
        };
        if !changed {
            return;
        }
        *guard = state.clone();
    }
    events::emit(app, ev::MEDIA, &state);
}

/// Shared AppHandle accessor for other services.
pub fn app_handle() -> Option<&'static AppHandle> {
    APP.get()
}

// ---- Transport controls ----
// Note: SMTC in the windows-rs 0.62 projection exposes no timeline-change
// API, so seeking is not offered; transport + skip are supported.

fn control<F>(f: F)
where
    F: FnOnce(&GlobalSystemMediaTransportControlsSession) -> windows::core::Result<()>
        + Send
        + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        ensure_com();
        let Ok(mgr) = request_manager() else { return };
        let Ok(Some(s)) = pick_session(&mgr) else { return };
        if let Err(err) = f(&s) {
            eprintln!("[media] control failed: {err}");
        }
    });
}

pub fn play_pause() {
    control(|s| {
        let playing = s
            .GetPlaybackInfo()
            .and_then(|p| p.PlaybackStatus())
            .map(|st| st == PlaybackStatus::Playing)
            .unwrap_or(false);
        let op = if playing {
            s.TryPauseAsync()?
        } else {
            s.TryPlayAsync()?
        };
        wait(op).map(|_| ())
    });
}

pub fn next() {
    control(|s| wait(s.TrySkipNextAsync()?).map(|_| ()));
}

pub fn prev() {
    control(|s| wait(s.TrySkipPreviousAsync()?).map(|_| ()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_app_strips_prefix_and_suffix() {
        assert_eq!(pretty_app("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.MediaPlayer"), "ZuneMusic");
        assert_eq!(pretty_app("spotify.exe!wow"), "spotify.exe");
        assert_eq!(pretty_app("plain"), "plain");
    }

    #[test]
    fn detect_mime_matches_signatures() {
        assert_eq!(detect_mime(&[0x89, b'P', b'N', b'G', 0, 0]), "image/png");
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(detect_mime(b"RIFF\x00\x00\x00\x00WEBPVP8 "), "image/webp");
        assert_eq!(detect_mime(&[1, 2, 3]), "image/png");
    }

    #[test]
    fn display_title_prettifies_episode_filenames() {
        assert_eq!(
            display_title("Suits S05E02.Compensation.mkv"),
            "Suits · S05E02 — Compensation"
        );
        assert_eq!(
            display_title("show.name.S02E05.720p.WEB-DL.x264"),
            "show name · S02E05"
        );
        assert_eq!(display_title("Blade Runner 2049"), "Blade Runner 2049");
        assert_eq!(display_title("ASAP S1E1"), "ASAP · S01E01");
        // No extension stripping without a known video extension.
        assert_eq!(display_title("Regular Song Name"), "Regular Song Name");
    }

    fn sample_state() -> MediaState {
        MediaState {
            id: "spotify".into(),
            app: "Spotify".into(),
            title: "You Know".into(),
            raw_title: "You Know".into(),
            artist: "Wake N' Bake, Merkules".into(),
            album: "You Know".into(),
            status: "playing".into(),
            position_ms: 18_000,
            duration_ms: 193_000,
            art_key: "You Know|Wake N' Bake, Merkules|You Know|193000".into(),
            art_b64: None,
            can_play: true,
            can_pause: true,
            can_next: true,
            can_prev: true,
            shuffle: None,
            repeat: None,
        }
    }

    #[test]
    fn fingerprint_ignores_art_bytes_but_not_metadata() {
        let a = sample_state();
        let mut b = a.clone();
        b.art_b64 = Some("data:image/png;base64,xxx".into());
        // Art byte presence alone (same key) must not re-trigger emits.
        assert_ne!(fingerprint(&a), fingerprint(&b));
        let mut c = a.clone();
        c.art_b64 = None;
        assert_eq!(fingerprint(&a), fingerprint(&c));
        let mut d = a.clone();
        d.position_ms += 500;
        assert_ne!(fingerprint(&a), fingerprint(&d));
        let mut e = a.clone();
        e.status = "paused".into();
        assert_ne!(fingerprint(&a), fingerprint(&e));
    }
}
