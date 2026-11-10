//! Small audio helpers (system sound playback, microphone state).

use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
use windows::core::PCWSTR;

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
