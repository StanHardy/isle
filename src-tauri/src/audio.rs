//! Small audio helpers (system sound playback, microphone state).

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator,
    PlaySoundW, SND_ALIAS, SND_ASYNC, EDataFlow, ERole,
};
use windows::core::PCWSTR;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// IAudioEndpointVolume for the default capture (microphone) endpoint.
fn mic_volume() -> windows::core::Result<IAudioEndpointVolume> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        device.Activate(CLSCTX_ALL, None)
    }
}

pub fn mic_muted() -> bool {
    unsafe {
        mic_volume()
            .and_then(|v| v.GetMute())
            .map(|m| m.as_bool())
            .unwrap_or(false)
    }
}

pub fn mic_set_mute(muted: bool) -> Result<(), String> {
    unsafe {
        let vol = mic_volume().map_err(|e| e.to_string())?;
        vol.SetMute(muted, std::ptr::null()).map_err(|e| e.to_string())
    }
}

pub fn mic_volume_level() -> u32 {
    unsafe {
        mic_volume()
            .and_then(|v| v.GetMasterVolumeLevelScalar())
            .map(|v| (v * 100.0).round() as u32)
            .unwrap_or(0)
    }
}

/// Play a Windows system sound alias by name (e.g. "Notification.Default",
/// "SystemExclamation") asynchronously. Missing aliases fall back to the
/// default system sound.
pub fn play_system_sound(alias: &str) {
    let wide: Vec<u16> = alias.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        PlaySoundW(
            PCWSTR(wide.as_ptr()),
            None,
            SND_ALIAS | SND_ASYNC,
        );
    }
}
