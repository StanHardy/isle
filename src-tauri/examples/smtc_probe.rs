//! Ground-truth probe: prints every SMTC session with status and position.
//! Run: cargo run --example smtc_probe --manifest-path src-tauri/Cargo.toml

use std::sync::{Mutex, OnceLock};

use windows::Foundation::TimeSpan;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};

static APP: OnceLock<()> = OnceLock::new();

fn ensure_com() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
}

fn wait<T>(op: windows_future::IAsyncOperation<T>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType,
{
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<()>();
    let handler =
        windows_future::AsyncOperationCompletedHandler::<T>::new(move |_, _| {
            let _ = tx.send(());
            Ok(())
        });
    op.SetCompleted(&handler)?;
    rx.recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| windows::core::Error::from_hresult(windows::core::HRESULT(1)))?;
    op.GetResults()
}

fn wait_p<T, P>(op: windows_future::IAsyncOperationWithProgress<T, P>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType,
    P: windows::core::RuntimeType,
{
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<()>();
    let handler = windows_future::AsyncOperationWithProgressCompletedHandler::<T, P>::new(
        move |_, _| {
            let _ = tx.send(());
            Ok(())
        },
    );
    op.SetCompleted(&handler)?;
    rx.recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| windows::core::Error::from_hresult(windows::core::HRESULT(1)))?;
    op.GetResults()
}

fn main() {
    APP.set(()).ok();
    ensure_com();

    // Optional control mode: smtc_probe play <app-substring> / pause <substr>
    // Diagnostic mode:       smtc_probe art <app-substring>
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "art" {
        let mgr = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
            Ok(op) => match wait(op) {
                Ok(m) => m,
                Err(e) => {
                    println!("request failed: {e}");
                    return;
                }
            },
            Err(e) => {
                println!("request failed: {e}");
                return;
            }
        };
        let sessions = mgr.GetSessions().expect("sessions");
        let count = sessions.Size().unwrap_or(0);
        for i in 0..count {
            let s = sessions.GetAt(i).expect("session");
            let app = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
            if !app.to_lowercase().contains(&args[2].to_lowercase()) {
                continue;
            }
            let props = match s.TryGetMediaPropertiesAsync() {
                Ok(op) => match wait(op) {
                    Ok(p) => p,
                    Err(e) => {
                        println!("props failed: {e}");
                        return;
                    }
                },
                Err(e) => {
                    println!("props failed: {e}");
                    return;
                }
            };
            match props.Thumbnail() {
                Err(e) => println!("Thumbnail() failed: {e}"),
                Ok(thumb) => {
                    match thumb.OpenReadAsync() {
                        Err(e) => println!("OpenReadAsync failed: {e}"),
                        Ok(op) => match wait(op) {
                            Err(e) => println!("open await failed: {e}"),
                            Ok(stream) => {
                                let size = stream.Size().unwrap_or(0);
                                let ct = stream.ContentType().map(|c| c.to_string()).unwrap_or_default();
                                println!("stream size={size} content_type=[{ct}]");
                                let sz = (size as u32).min(4_000_000);
                                match windows::Storage::Streams::Buffer::Create(sz) {
                                    Err(e) => println!("buffer failed: {e}"),
                                    Ok(buf) => match stream.ReadAsync(
                                        &buf,
                                        sz,
                                        windows::Storage::Streams::InputStreamOptions::ReadAhead,
                                    ) {
                                        Err(e) => println!("ReadAsync failed: {e}"),
                                        Ok(op2) => match wait_p(op2) {
                                            Err(e) => println!("read await failed: {e}"),
                                            Ok(read) => {
                                                let len = read.Length().unwrap_or(0);
                                                match windows::Storage::Streams::DataReader::FromBuffer(&read) {
                                                    Err(e) => println!("FromBuffer failed: {e}"),
                                                    Ok(reader) => {
                                                        let mut bytes = vec![0u8; len as usize];
                                                        match reader.ReadBytes(&mut bytes) {
                                                            Err(e) => println!("ReadBytes failed: {e}"),
                                                            Ok(_) => {
                                                                println!("read {len} bytes, first: {:02x?}", &bytes[..8.min(bytes.len())]);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                    },
                                }
                            }
                        },
                    }
                }
            }
            return;
        }
        println!("no session matching {}", args[2]);
        return;
    }
    if args.len() >= 3 && (args[1] == "play" || args[1] == "pause") {
        let mgr = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
            Ok(op) => match wait(op) {
                Ok(m) => m,
                Err(e) => {
                    println!("request failed: {e}");
                    return;
                }
            },
            Err(e) => {
                println!("request failed: {e}");
                return;
            }
        };
        let sessions = mgr.GetSessions().expect("sessions");
        let count = sessions.Size().unwrap_or(0);
        for i in 0..count {
            let s = sessions.GetAt(i).expect("session");
            let app = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
            if app.to_lowercase().contains(&args[2].to_lowercase()) {
                let op = if args[1] == "play" {
                    s.TryPlayAsync().expect("op")
                } else {
                    s.TryPauseAsync().expect("op")
                };
                match wait(op) {
                    Ok(_) => println!("{} on {} → ok", args[1], app),
                    Err(e) => println!("failed: {e}"),
                }
                return;
            }
        }
        println!("no session matching {}", args[2]);
        return;
    }
    let mgr = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
        Ok(op) => match wait(op) {
            Ok(m) => m,
            Err(e) => {
                println!("request failed: {e}");
                return;
            }
        },
        Err(e) => {
            println!("request failed: {e}");
            return;
        }
    };
    let sessions = match mgr.GetSessions() {
        Ok(s) => s,
        Err(e) => {
            println!("sessions failed: {e}");
            return;
        }
    };
    let count = sessions.Size().unwrap_or(0);
    println!("session count: {count}");
    for i in 0..count {
        let Ok(s) = sessions.GetAt(i) else { continue };
        let app = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
        let status = s
            .GetPlaybackInfo()
            .and_then(|p| p.PlaybackStatus())
            .map(|x| format!("{x:?}"))
            .unwrap_or_else(|_| "?".into());
        let (title, artist) = match s.TryGetMediaPropertiesAsync() {
            Ok(op) => match wait(op) {
                Ok(p) => (
                    p.Title().map(|t| t.to_string()).unwrap_or_default(),
                    p.Artist().map(|t| t.to_string()).unwrap_or_default(),
                ),
                Err(_) => (String::new(), String::new()),
            },
            Err(_) => (String::new(), String::new()),
        };
        let tl = s.GetTimelineProperties();
        let (pos, end) = match tl {
            Ok(t) => (
                t.Position().map(|p| p.Duration).unwrap_or(0),
                t.EndTime().map(|e| e.Duration).unwrap_or(0),
            ),
            Err(_) => (0, 0),
        };
        println!(
            "#{i} app=[{app}]\n    status={status} title=[{title}] artist=[{artist}] pos={}s/{}s",
            pos / 10_000_000,
            end / 10_000_000
        );
        let _ = TimeSpan::default();
    }
}
