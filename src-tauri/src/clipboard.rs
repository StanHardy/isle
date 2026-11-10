use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// CF_UNICODETEXT (13) — the projection exposes it in System::Ole with a
/// u16 backing type; using the literal here keeps the feature set small.
const CF_UNICODETEXT: u32 = 13;

use crate::events::{self, ev};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipEntry {
    pub id: u64,
    /// Masked when sensitive; full text is only served via clipboard_reveal/copy.
    pub text: String,
    pub ts_ms: u64,
    pub sensitive: bool,
}

#[derive(Debug, Default)]
pub struct ClipStore {
    pub entries: Vec<ClipEntry>,
    pub next_id: u64,
    pub last_seq: u32,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Poll the clipboard sequence number; opt-in capture of new text entries.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let seq = unsafe { GetClipboardSequenceNumber() };
            let (enabled, keep) = {
                let state: tauri::State<AppState> = app.state();
                let s = state.settings.read().expect("settings lock");
                (s.clipboard_enabled, s.clipboard_keep)
            };
            {
                let state: tauri::State<AppState> = app.state();
                let mut store = state.clipboard.lock().expect("clipboard lock");
                if !enabled {
                    store.last_seq = seq;
                    continue;
                }
                if seq == store.last_seq || seq == 0 {
                    continue;
                }
                store.last_seq = seq;
            }
            if let Some(text) = read_text() {
                if text.is_empty() {
                    continue;
                }
                push_and_emit(&app, text, keep);
            }
        }
    });
}

fn push_and_emit(app: &AppHandle, text: String, keep: u32) {
    let state: tauri::State<AppState> = app.state();
    let mut store = state.clipboard.lock().expect("clipboard lock");
    // Skip duplicates of the most recent entry (apps copy repeatedly).
    if store.entries.first().map(|e| e.text.clone()) == Some(text.clone()) {
        if let Some(first) = store.entries.first_mut() {
            first.ts_ms = now_ms();
        }
        return;
    }
    let id = store.next_id + 1;
    store.next_id = id;
    let sensitive = is_sensitive(&text);
    store.entries.insert(
        0,
        ClipEntry {
            id,
            text: if sensitive { mask_text(&text) } else { text },
            ts_ms: now_ms(),
            sensitive,
        },
    );
    let keep = keep.max(10) as usize;
    store.entries.truncate(keep);
    let list = masked_list(&store);
    drop(store);
    events::emit(app, ev::CLIPBOARD_SYNC, &list);
}

pub fn masked_list(store: &ClipStore) -> Vec<ClipEntry> {
    store.entries.clone()
}

/// Full text for reveal/copy (sensitive entries are masked in the UI).
pub fn full_text(app: &AppHandle, id: u64) -> Option<String> {
    let state: tauri::State<AppState> = app.state();
    let store = state.clipboard.lock().expect("clipboard lock");
    store.entries.iter().find(|e| e.id == id).map(|e| e.text.clone())
}

/// Read CF_UNICODETEXT with a short retry (the clipboard is often
/// momentarily busy; each attempt opens once and closes once).
pub fn read_text() -> Option<String> {
    unsafe {
        let mut opened = false;
        for _ in 0..4 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return None;
        }
        let result = (|| -> Option<String> {
            let _ = IsClipboardFormatAvailable(CF_UNICODETEXT);
            let handle: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return None;
            }
            // Wide string, NUL-terminated.
            let mut len = 0usize;
            while *ptr.cast::<u16>().add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr.cast::<u16>(), len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Write text to the clipboard (used by copy actions).
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;
    unsafe {
        let mut opened = false;
        for _ in 0..3 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !opened {
            return false;
        }
        let ok = (|| -> bool {
            let _ = EmptyClipboard();
            let hglobal = match GlobalAlloc(GMEM_MOVEABLE, byte_len) {
                Ok(h) => h,
                Err(_) => return false,
            };
            let dst = GlobalLock(hglobal);
            if dst.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst.cast::<u16>(), wide.len());
            let _ = GlobalUnlock(hglobal);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0))).is_ok()
        })();
        let _ = CloseClipboard();
        ok
    }
}

/// Public mask used for sensitive entries in events and UI payloads.
pub fn mask_text(text: &str) -> String {    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 4 {
        "•".repeat(chars.len().max(1))
    } else {
        let head: String = chars[..2].iter().collect();
        format!("{head}••••••")
    }
}

/// Heuristics for likely-sensitive clipboard content (cards, IBANs, tokens).
pub fn is_sensitive(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 8 || trimmed.contains('\n') {
        return false;
    }
    if has_card_number(trimmed) {
        return true;
    }
    if looks_like_iban(trimmed) {
        return true;
    }
    if looks_like_token(trimmed) {
        return true;
    }
    false
}

/// 13–19 digit groups (spaces/dashes allowed) that pass the Luhn checksum.
fn has_card_number(text: &str) -> bool {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    // Separators (spaces/dashes) are normal for card groups; other
    // non-digit content must stay a small minority of the text.
    let non_sep = text
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_whitespace() && *c != '-' && *c != '.')
        .count();
    if non_sep > digits.len() / 3 {
        return false;
    }
    luhn(&digits)
}

pub fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .chars()
        .rev()
        .filter_map(|c| c.to_digit(10))
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                let d2 = d * 2;
                d2 / 10 + d2 % 10
            } else {
                d
            }
        })
        .sum();
    // Require a plausible trailing check digit (sum divisible by 10).
    sum % 10 == 0 && digits.len() >= 13
}

fn looks_like_iban(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = compact.as_bytes();
    if bytes.len() < 15 || bytes.len() > 34 {
        return false;
    }
    let ok = bytes[0].is_ascii_uppercase()
        && bytes[1].is_ascii_uppercase()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4..].iter().all(|b| b.is_ascii_alphanumeric());
    ok && compact.chars().any(|c| c.is_ascii_digit())
}

fn looks_like_token(text: &str) -> bool {
    if text.contains("://") || text.contains(' ') {
        return false;
    }
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    let has_alpha = text.chars().any(|c| c.is_alphabetic());
    text.len() >= 20 && has_digit && has_alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luhn_accepts_valid_card() {
        // Classic test number.
        assert!(luhn("4111111111111111"));
        assert!(luhn("5500005555555559"));
        assert!(!luhn("4111111111111112"));
        assert!(!luhn("1234567890123"));
    }

    #[test]
    fn detects_card_with_separators() {
        assert!(is_sensitive("Visa 4111 1111 1111 1111"));
        assert!(!is_sensitive("meet me at 5pm today"));
    }

    #[test]
    fn detects_iban() {
        assert!(is_sensitive("GB29 NWBK 6016 1331 9268 19"));
        assert!(!is_sensitive("hello world"));
    }

    #[test]
    fn detects_tokens_but_not_urls() {
        assert!(is_sensitive("ghp_0123456789abcdefABCDEF"));
        assert!(!is_sensitive("https://github.com/StanHardy/isle"));
    }

    #[test]
    fn mask_preserves_head_only() {
        assert_eq!(mask_text("abcdefgh"), "ab••••••");
        assert_eq!(mask_text("abc"), "•••");
    }

    #[test]
    fn multiline_text_is_never_sensitive() {
        assert!(!is_sensitive("4111111111111111\nsecond line"));
    }
}
