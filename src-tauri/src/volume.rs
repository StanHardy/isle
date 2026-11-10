//! Volume OSD: registers a WASAPI endpoint-volume callback on the default
//! render device and emits `volume:osd` whenever hardware volume changes.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::implement;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, EDataFlow, ERole, IMMDeviceEnumerator,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

use crate::events::{self, ev};

fn now() -> Instant {
    Instant::now()
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCb {
    app: tauri::AppHandle,
    last: Mutex<Instant>,
}

impl windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolumeCallback_Impl for VolumeCb_Impl {
    fn OnNotify(
        &self,
        pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
    ) -> windows::core::Result<()> {
        if pnotify.is_null() {
            return Ok(());
        }
        // The struct is packed(1): copy it wholesale before reading fields.
        let data = unsafe { *pnotify };
        {
            let mut last = self.last.lock().expect("volume cb lock");
            if now().duration_since(*last) < Duration::from_millis(90) {
                return Ok(());
            }
            *last = now();
        }
        let volume = (data.fMasterVolume * 100.0).round().clamp(0.0, 100.0) as u32;
        let muted = data.bMuted.as_bool();
        events::emit(
            &self.app,
            ev::VOLUME_OSD,
            &serde_json::json!({ "volume": volume, "muted": muted }),
        );
        Ok(())
    }
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            let cb: IAudioEndpointVolumeCallback =
                VolumeCb { app: app.clone(), last: Mutex::new(now()) }.into();
            vol.RegisterControlChangeNotify(&cb)?;
            crate::events::debug_log("[volume] callback registered");
            // Keep BOTH objects alive: dropping the endpoint volume object
            // destroys the notification registration (notifications silently
            // stop firing — the bug that killed the volume OSD initially).
            std::mem::forget(vol);
            std::mem::forget(cb);
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("[volume] register failed: {err}");
            crate::events::debug_log(&format!("[volume] register failed: {err}"));
        }
    });
}
