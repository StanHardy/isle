use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    SetWinEventHook, CUIAutomation, HWINEVENTHOOK, IUIAutomation, IUIAutomationElementArray, TreeScope_Descendants, WINEVENTPROC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetClassNameW, GetWindowTextW, TranslateMessage,
    EVENT_OBJECT_SHOW, MSG, WINEVENT_OUTOFCONTEXT,
};

use crate::events::{self, ev};
use crate::state::AppState;

static APP: OnceLock<AppHandle> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
/// Toast re-render dedupe: Win11 re-renders banners during their entrance.
static LAST_KEY: Mutex<(String, Option<Instant>)> = Mutex::new((String::new(), None));

/// The toast banner's CoreWindow title is the localized "New notification"
/// string — match the common ones so the mirror works beyond English
/// installs. Unknown titles are logged once each (debug.log) so a missed
/// locale is self-diagnosing.
const TOAST_TITLES: &[&str] = &[
    "New notification",      // en
    "Neue Benachrichtigung", // de
    "Nouvelle notification", // fr
    "Nueva notificación",    // es
    "Nuova notifica",        // it
    "Nova notificação",      // pt-BR
    "Новое уведомление",     // ru
    "Нове повідомлення",     // uk
    "Nowe powiadomienie",    // pl
    "Nieuwe melding",        // nl
    "Yeni bildirim",         // tr
    "新しい通知",              // ja
    "새 알림",                // ko
    "新通知",                 // zh-CN / zh-TW
    "إعلام جديد",             // ar
    "नई सूचना",               // hi
    "Nytt meddelande",       // sv
    "Ny meddelelse",         // da
    "Uusi ilmoitus",         // fi
    "Nytt varsel",           // nb
    "Nové oznámení",         // cs
    "Νέα ειδοποίηση",        // el
    "Új értesítés",          // hu
    "Notificare nouă",       // ro
    "הודעה חדשה",             // he
    "การแจ้งเตือนใหม่",        // th
    "Thông báo mới",         // vi
    "Notifikasi baru",       // id
];

static SEEN_TITLES: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn seen_titles() -> &'static Mutex<std::collections::HashSet<String>> {
    SEEN_TITLES.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn is_toast_banner(title: &str) -> bool {
    if TOAST_TITLES.iter().any(|t| *t == title) {
        return true;
    }
    // Diagnose future locales: log each unknown CoreWindow title once.
    let mut seen = seen_titles().lock().expect("seen titles lock");
    if seen.insert(title.to_string()) {
        crate::events::debug_log(&format!(
            "[notify] unknown CoreWindow title {title:?} — report this to add your locale"
        ));
    }
    false
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyItem {
    pub id: u64,
    pub source: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub ts_ms: u64,
}

/// UI Automation toast mirror (UserNotificationListener requires package
/// identity, which unpackaged apps do not have). Hooks EVENT_OBJECT_SHOW on a
/// pumped thread and reads new toast windows.
pub fn start(app: AppHandle) {
    let _ = APP.set(app);
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let proc: WINEVENTPROC = Some(winevent_proc);
        let hook = SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_SHOW,
            None,
            proc,
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            eprintln!("[notify] SetWinEventHook failed");
            return;
        }
        let mut msg = MSG::default();
        // Message pump: required for out-of-context WinEvent hooks.
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

unsafe extern "system" fn winevent_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    idobject: i32,
    _idchild: i32,
    _thread: u32,
    _time: u32,
) {
    if idobject != 0 {
        // OBJID_WINDOW only.
        return;
    }
    if hwnd.0.is_null() {
        return;
    }
    let mut buf = [0u16; 128];
    let n = GetClassNameW(hwnd, &mut buf);
    let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    if class != "Windows.UI.Core.CoreWindow" {
        return;
    }
    let mut buf = [0u16; 128];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    if !is_toast_banner(&title) {
        return;
    }
    if let Some(item) = read_toast(hwnd) {
        publish(item);
    }
}

/// Read the toast's text elements via UI Automation.
fn read_toast(hwnd: HWND) -> Option<NotifyItem> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let element = automation.ElementFromHandle(hwnd).ok()?;
        let cond = automation.CreateTrueCondition().ok()?;
        let all: IUIAutomationElementArray = element.FindAll(TreeScope_Descendants, &cond).ok()?;
        let n = all.Length().ok()?;
        let mut names: Vec<String> = Vec::new();
        for i in 0..n {
            if let Ok(el) = all.GetElement(i) {
                if let Ok(name) = el.CurrentName() {
                    let s = name.to_string();
                    let s = s.trim();
                    if !s.is_empty() && !names.iter().any(|x| x == s) {
                        names.push(s.to_string());
                    }
                }
            }
        }
        build_item(&names)
    }
}

/// Heuristic layout: [app, title, ...body lines], dropping action labels.
fn build_item(names: &[String]) -> Option<NotifyItem> {
    const NOISE: [&str; 4] = ["Close", "Dismiss", "Clear all", "Settings"];
    let kept: Vec<&String> = names
        .iter()
        .filter(|n| !NOISE.iter().any(|x| n.eq_ignore_ascii_case(x)))
        .collect();
    if kept.is_empty() {
        return None;
    }
    let (app, title, body) = match kept.len() {
        1 => (String::new(), kept[0].clone(), String::new()),
        2 => (String::new(), kept[0].clone(), kept[1].clone()),
        _ => (
            kept[0].clone(),
            kept[1].clone(),
            kept[2..]
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>()
                .join(" · "),
        ),
    };
    Some(NotifyItem {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        source: "windows".into(),
        app,
        title,
        body,
        ts_ms: crate::clipboard::now_ms(),
    })
}

fn publish(item: NotifyItem) {
    let key = format!("{}|{}", item.title, item.body);
    {
        let mut last = LAST_KEY.lock().expect("notify key lock");
        if last.0 == key && last.1.map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false) {
            return;
        }
        *last = (key, Some(Instant::now()));
    }
    let app = match APP.get() {
        Some(a) => a,
        None => return,
    };
    let enabled = {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        s.notifications_enabled
    };
    if !enabled {
        return;
    }
    let state: tauri::State<AppState> = app.state();
    {
        let mut list = state.notifications.lock().expect("notifications lock");
        list.insert(0, item.clone());
        list.truncate(50);
    }
    events::emit(app, ev::NOTIFY_NEW, &item);
}
