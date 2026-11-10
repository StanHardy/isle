//! SMTC media integration.
//!
//! windows-rs COM interfaces are not `Send`, so every WinRT interaction runs
//! on a blocking worker thread (no `.await` across interface values). The
//! SMTC manager is a per-process singleton, re-acquired per work item, so no
//! interface state needs to be shared between threads.

use std::sync::{Mutex, OnceLock};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub id: String,
    pub app: String,
    pub title: String,
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

/// The manager is a per-process singleton; prefer the playing session.
fn pick_session(
    mgr: &GlobalSystemMediaTransportControlsSessionManager,
) -> windows::core::Result<Option<GlobalSystemMediaTransportControlsSession>> {
    let sessions = mgr.GetSessions()?;
    let count = sessions.Size()?;
    let mut playing = None;
    let mut first = None;
    for i in 0..count {
        let s = sessions.GetAt(i)?;
        let status = s
            .GetPlaybackInfo()
            .and_then(|p| p.PlaybackStatus())
            .unwrap_or(PlaybackStatus::Stopped);
        if first.is_none() {
            first = Some(s.clone());
        }
        if status == PlaybackStatus::Playing && playing.is_none() {
            playing = Some(s);
        }
    }
    Ok(playing.or(first))
}

pub fn start(app: AppHandle) {
    let _ = APP.set(app.clone());
    tauri::async_runtime::spawn_blocking(move || {
        ensure_com();
        if let Err(err) = subscribe(&app) {
            eprintln!("[media] subscribe failed: {err}");
        }
        if let Err(err) = refresh(&app) {
            eprintln!("[media] initial refresh failed: {err}");
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
            if let Err(err) = refresh(&app) {
                eprintln!("[media] refresh failed: {err}");
            }
        });
        Ok(())
    });
    mgr.SessionsChanged(&handler)?;
    Ok(())
}

/// Re-pick the current session and emit fresh state (blocking).
fn refresh(app: &AppHandle) -> windows::core::Result<()> {
    let mgr = request_manager()?;
    let session = pick_session(&mgr)?;
    match session {
        Some(s) => {
            attach_handlers(&s, app);
            let want_art = art_key_changed(&s)?;
            let state = build_state(&s, want_art)?;
            update_and_emit(app, Some(state));
        }
        None => {
            *LAST_ART_KEY.lock().expect("art key lock") = String::new();
            update_and_emit(app, None);
        }
    }
    Ok(())
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
            update_and_emit(app, Some(state));
        }
        None => {
            *LAST_ART_KEY.lock().expect("art key lock") = String::new();
            update_and_emit(app, None);
        }
    }
    Ok(())
}

fn build_state(
    s: &GlobalSystemMediaTransportControlsSession,
    want_art: bool,
) -> windows::core::Result<MediaState> {
    let props = wait(s.TryGetMediaPropertiesAsync()?)?;
    let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
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
    wait(reader.LoadAsync(len as u32)?)?;
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

fn update_and_emit(app: &AppHandle, state: Option<MediaState>) {
    {
        let state_ref: tauri::State<AppState> = app.state();
        *state_ref.media.lock().expect("media state lock") = state.clone();
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
}
