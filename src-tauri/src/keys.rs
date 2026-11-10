//! Global media-key / volume-key feedback: a low-level keyboard hook that
//! emits `osd:key` events so the island can show what each key did.

use std::sync::OnceLock;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    KBDLLHOOKSTRUCT,
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, HOOKPROC, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, MSG,
};

use crate::events::{self, ev};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

const LABELS: &[(u32, &str)] = &[
    (0xB3, "⏯ Play / Pause"),
    (0xB0, "⏭ Next track"),
    (0xB1, "⏮ Previous track"),
    (0xB2, "⏹ Stop"),
    (0xAD, "🔇 Mute"),
    (0xAF, "🔊 Volume up"),
    (0xAE, "🔉 Volume down"),
];

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Fire on key-up so held volume keys don't spam the OSD.
    if code >= 0 && wparam.0 == 0x101 {
        let st = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if let Some((_, label)) = LABELS.iter().find(|(vk, _)| *vk == st.vkCode) {
            if let Some(app) = APP.get() {
                events::emit(app, ev::OSD_KEY, &serde_json::json!({ "label": label }));
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

pub fn start(app: tauri::AppHandle) {
    let _ = APP.set(app);
    std::thread::spawn(move || unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0);
        if let Err(err) = hook {
            eprintln!("[keys] hook failed: {err}");
            return;
        }
        let hook = hook.expect("hook");
        let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    });
}
