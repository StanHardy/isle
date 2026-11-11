//! Bluetooth: audio-endpoint connect/disconnect events (KSCATEGORY_RENDER
//! interfaces) + Bluetooth battery levels (AEP watcher on
//! System.Devices.AepBattery_Level).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::core::{HSTRING, Interface as _};
use windows::Devices::Enumeration::{
    DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher,
};
use windows::Foundation::{IReference, TypedEventHandler};

use crate::events::{self, ev};
use crate::state::AppState;

/// KSCATEGORY_RENDER device interfaces = audio render endpoints (BT/USB/…).
const RENDER_AQS: &str =
    "(System.Devices.InterfaceClassGuid:=\"{65E8773D-8F56-11D0-A3B2-00A0C9223196}\")";
const PROP_ENABLED: &str = "System.Devices.InterfaceEnabled";

const AEP_AQS: &str =
    "(System.Devices.Aep.IsConnected:=System.StructuredQueryType.Boolean#True)";
const PROP_BATTERY: &str = "System.Devices.AepBattery_Level";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtBattery {
    pub id: String,
    pub name: String,
    pub level: u32,
}

static FIRST_PASS: AtomicBool = AtomicBool::new(true);

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let watcher = match DeviceInformation::CreateWatcherAqsFilter(&HSTRING::from(RENDER_AQS)) {
            Ok(w) => w,
            Err(err) => {
                crate::events::debug_log(&format!("[bt] watcher failed: {err}"));
                return;
            }
        };

        let added = TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(|_, info| {
            let info = info.ok()?;
            let id = info.Id().map(|i| i.to_string()).unwrap_or_default();
            let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
            let connected = interface_enabled(info).unwrap_or(true);
            on_state(&id, &name, connected);
            Ok(())
        });
        let updated = TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(|_, upd| {
            let upd = upd.ok()?;
            let id = upd.Id().map(|i| i.to_string()).unwrap_or_default();
            if let Some(enabled) = update_enabled(upd) {
                let name = current_name(&id).unwrap_or_else(|| id.clone());
                on_state(&id, &name, enabled);
            }
            Ok(())
        });
        let completed = TypedEventHandler::<DeviceWatcher, windows::core::IInspectable>::new(|_, _| {
            FIRST_PASS.store(false, Ordering::Relaxed);
            Ok(())
        });
        if let Err(err) = watcher.Added(&added) {
            crate::events::debug_log(&format!("[bt] added hook failed: {err}"));
            return;
        }
        if let Err(err) = watcher.Updated(&updated) {
            crate::events::debug_log(&format!("[bt] updated hook failed: {err}"));
            return;
        }
        let _ = watcher.EnumerationCompleted(&completed);
        if let Err(err) = watcher.Start() {
            crate::events::debug_log(&format!("[bt] watcher start failed: {err}"));
        }
        // Keep the watcher alive for the process lifetime.
        std::mem::forget(watcher);
    });
}

fn interface_enabled(info: &DeviceInformation) -> Option<bool> {
    let props = info.Properties().ok()?;
    let value = props.Lookup(&HSTRING::from(PROP_ENABLED)).ok()?;
    value.cast::<IReference<bool>>().ok()?.Value().ok()
}

fn update_enabled(upd: &DeviceInformationUpdate) -> Option<bool> {
    let props = upd.Properties().ok()?;
    let value = props.Lookup(&HSTRING::from(PROP_ENABLED)).ok()?;
    value.cast::<IReference<bool>>().ok()?.Value().ok()
}

fn current_name(id: &str) -> Option<String> {
    let app = crate::media::app_handle()?;
    let state: tauri::State<AppState> = app.state();
    let devices = state.bt_devices.lock().expect("bt lock");
    devices.get(id).map(|d| d.name.clone())
}

fn on_state(id: &str, name: &str, connected: bool) {
    let app = match crate::media::app_handle() {
        Some(a) => a.clone(),
        None => return,
    };
    let animate = !FIRST_PASS.load(Ordering::Relaxed);
    {
        let state: tauri::State<AppState> = app.state();
        let mut devices = state.bt_devices.lock().expect("bt lock");
        let entry = devices
            .entry(id.to_string())
            .or_insert_with(|| BtDevice {
                id: id.to_string(),
                name: name.to_string(),
                connected,
            });
        entry.name = name.to_string();
        entry.connected = connected;
    }
    let enabled = {
        let state: tauri::State<AppState> = app.state();
        let s = state.settings.read().expect("settings lock");
        s.bluetooth_enabled
    };
    if animate && enabled && !name.trim().is_empty() {
        let payload = serde_json::json!({
            "id": id, "name": name, "connected": connected,
            "kind": "audio-render",
        });
        events::emit(&app, ev::BT_DEVICE, &payload);
        events::emit(
            &app,
            ev::ATTENTION,
            &serde_json::json!({
                "kind": if connected { "bt-connected" } else { "bt-disconnected" },
                "name": name,
            }),
        );
    }
}

pub fn devices(app: &AppHandle) -> Vec<BtDevice> {
    let state: tauri::State<AppState> = app.state();
    let devices: HashMap<String, BtDevice> = state.bt_devices.lock().expect("bt lock").clone();
    let mut list: Vec<BtDevice> = devices.into_values().collect();
    list.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
    list
}

// ---- battery ----

static BT_APP: OnceLock<AppHandle> = OnceLock::new();
static BATTERY_FIRST_PASS: AtomicBool = AtomicBool::new(true);

/// Bluetooth battery levels for connected AEPs (Win11 exposes
/// System.Devices.AepBattery_Level). Emits bt:battery on change.
pub fn start_battery_watcher(app: AppHandle) {
    let _ = BT_APP.set(app.clone());
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let props = vec![
            HSTRING::from("System.ItemNameDisplay"),
            HSTRING::from(PROP_BATTERY),
        ];
        let Ok(props_iter) =
            windows::Foundation::PropertyValue::CreateStringArray(&props).and_then(|v| {
                v.cast::<windows_collections::IIterable<HSTRING>>()
            })
        else {
            crate::events::debug_log("[bt] battery props failed");
            return;
        };
        let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
            &HSTRING::from(AEP_AQS),
            &props_iter,
            DeviceInformationKind::AssociationEndpoint,
        );
        let Ok(watcher) = watcher else {
            crate::events::debug_log("[bt] battery watcher create failed");
            return;
        };

        let levels: Arc<Mutex<HashMap<String, (String, u32)>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let added_app = app.clone();
        let added_levels = levels.clone();
        let added = TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
            move |_, info| {
                let info = info.ok()?;
                let id = info.Id().map(|i| i.to_string()).unwrap_or_default();
                let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
                let level = battery_level(&info).unwrap_or(0);
                if level == 0 {
                    return Ok(());
                }
                {
                    let mut map = added_levels.lock().expect("bt battery lock");
                    let prev = map.get(&id).map(|(_, l)| *l).unwrap_or(u32::MAX);
                    map.insert(id.clone(), (name.clone(), level));
                    if level == prev {
                        return Ok(());
                    }
                }
                if !BATTERY_FIRST_PASS.load(Ordering::Relaxed) {
                    events::emit(
                        &added_app,
                        "bt:battery",
                        &serde_json::json!({ "id": id, "name": name, "level": level }),
                    );
                }
                Ok(())
            },
        );
        let upd_app = app.clone();
        let upd_levels = levels.clone();
        let updated = TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(
            move |_, upd| {
                let upd = upd.ok()?;
                let id = upd.Id().map(|i| i.to_string()).unwrap_or_default();
                let (name, prev) = {
                    let map = upd_levels.lock().expect("bt battery lock");
                    match map.get(&id) {
                        Some((n, l)) => (n.clone(), *l),
                        None => return Ok(()),
                    }
                };
                let props = upd.Properties();
                let Ok(props) = props else { return Ok(()) };
                let Ok(v) = props.Lookup(&HSTRING::from(PROP_BATTERY)) else { return Ok(()) };
                let Ok(bat) = v.cast::<windows::Foundation::IReference<u32>>() else {
                    return Ok(());
                };
                let Ok(level) = bat.Value() else { return Ok(()) };
                if level > 0 && level <= 100 && level != prev {
                    {
                        let mut map = upd_levels.lock().expect("bt battery lock");
                        map.insert(id.clone(), (name.clone(), level));
                    }
                    if !BATTERY_FIRST_PASS.load(Ordering::Relaxed) {
                        events::emit(
                            &upd_app,
                            "bt:battery",
                            &serde_json::json!({ "id": id, "name": name, "level": level }),
                        );
                    }
                }
                Ok(())
            },
        );
        let completed = TypedEventHandler::<DeviceWatcher, windows::core::IInspectable>::new(
            |_, _| {
                BATTERY_FIRST_PASS.store(false, Ordering::Relaxed);
                crate::events::debug_log("[bt] battery watcher enumerated");
                Ok(())
            },
        );
        if let Err(err) = watcher.Added(&added) {
            crate::events::debug_log(&format!("[bt] battery added hook failed: {err}"));
            return;
        }
        if let Err(err) = watcher.Updated(&updated) {
            crate::events::debug_log(&format!("[bt] battery updated hook failed: {err}"));
            return;
        }
        let _ = watcher.EnumerationCompleted(&completed);
        if let Err(err) = watcher.Start() {
            crate::events::debug_log(&format!("[bt] battery watcher start failed: {err}"));
            return;
        }
        crate::events::debug_log("[bt] battery watcher running");
        std::mem::forget(watcher);
    });
}

fn battery_level(info: &DeviceInformation) -> Option<u32> {
    let props = info.Properties().ok()?;
    let value = props.Lookup(&HSTRING::from(PROP_BATTERY)).ok()?;
    value
        .cast::<windows::Foundation::IReference<u32>>()
        .ok()?
        .Value()
        .ok()
}
