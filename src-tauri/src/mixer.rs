//! Per-app volume mixer: enumerates the audio sessions on the default render
//! endpoint and exposes per-session volume (ISimpleAudioVolume).

use std::collections::HashMap;
use std::mem::size_of;

use serde::Serialize;
use windows::core::Interface as _;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionState, IAudioSessionControl2, IAudioSessionEnumerator,
    IAudioSessionManager2, ISimpleAudioVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerApp {
    pub pid: u32,
    pub name: String,
    pub volume: u32,
}

fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return format!("pid {pid}");
        };
        if snap.is_invalid() {
            return format!("pid {pid}");
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut name: Option<String> = None;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID == pid {
                    let raw = String::from_utf16_lossy(
                        &entry.szExeFile[..entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(0)],
                    );
                    name = Some(raw);
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        name.unwrap_or_else(|| format!("pid {pid}"))
    }
}

/// Enumerate active per-app sessions on the default render endpoint.
pub fn list() -> Result<Vec<MixerApp>, String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
        let count = sessions.GetCount().map_err(|e| e.to_string())?;

        let mut out = Vec::new();
        for i in 0..count {
            let Ok(control) = sessions.GetSession(i) else { continue };
            // Skip inactive/expired sessions — only live audio.
            if matches!(
                control.GetState(),
                Ok(state) if state.0 != 1
            ) {
                continue;
            }
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let pid = control2.GetProcessId().unwrap_or(0);
            if pid == 0 || pid == std::process::id() {
                continue;
            }
            let Ok(vol) = control.cast::<ISimpleAudioVolume>() else { continue };
            let volume = vol
                .GetMasterVolume()
                .map(|v| (v * 100.0).round() as u32)
                .unwrap_or(0);
            out.push(MixerApp {
                pid,
                name: process_name(pid),
                volume,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

/// Set one app's session volume (all sessions of that process).
pub fn set(pid: u32, pct: u32) -> Result<(), String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
        let count = sessions.GetCount().map_err(|e| e.to_string())?;
        for i in 0..count {
            let Ok(control) = sessions.GetSession(i) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            if control2.GetProcessId().unwrap_or(0) != pid {
                continue;
            }
            let Ok(vol) = control.cast::<ISimpleAudioVolume>() else { continue };
            let _ = vol.SetMasterVolume(pct.clamp(0, 100) as f32 / 100.0, std::ptr::null());
        }
        Ok(())
    }
}

/// Friendly process names for a batch of pids (single snapshot).
fn process_names(pids: &[u32]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return map;
        };
        if snap.is_invalid() {
            return map;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pids.contains(&pid) {
                    let raw = String::from_utf16_lossy(
                        &entry.szExeFile[..entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(0)],
                    );
                    map.insert(pid, raw);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    map
}

/// Batch variant used by the poller (one snapshot for all sessions).
pub fn list_batch() -> Result<Vec<MixerApp>, String> {
    let mut apps = list()?;
    let pids: Vec<u32> = apps.iter().map(|a| a.pid).collect();
    let names = process_names(&pids);
    for a in &mut apps {
        if let Some(n) = names.get(&a.pid) {
            a.name = n.clone();
        }
    }
    Ok(apps)
}
