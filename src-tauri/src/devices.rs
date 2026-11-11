//! Audio device switching via the undocumented IPolicyConfig COM interface,
//! plus device listing with friendly names.

use std::mem::size_of;
use std::sync::Mutex;

use serde::Serialize;
use windows::core::{IUnknown, IUnknownImpl, Interface, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    eConsole, eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CLSCTX_ALL, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

// The known IPolicyConfig CLSID/IID pair (documented by SoundSwitch et al).
const CLSID_POLICY_CONFIG_CLIENT: windows::core::GUID =
    windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IID_POLICY_CONFIG: windows::core::GUID =
    windows::core::GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c4);

/// Vtable layout mirrors IPolicyConfig on Win10/11 (SoundSwitch ordering):
/// IUnknown (0-2), then 11 methods, SetDefaultEndpoint at index 13.
#[repr(C)]
struct IPolicyConfigVtbl {
    QueryInterface: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        iid: *const windows::core::GUID,
        out: *mut *mut core::ffi::c_void,
    ) -> windows::core::HRESULT,
    AddRef: unsafe extern "system" fn(this: *mut core::ffi::c_void) -> u32,
    Release: unsafe extern "system" fn(this: *mut core::ffi::c_void) -> u32,
    GetMixFormat: usize,
    GetDeviceFormat: usize,
    ResetDeviceFormat: usize,
    SetDeviceFormat: usize,
    GetProcessingPeriod: usize,
    SetProcessingPeriod: usize,
    GetShareMode: usize,
    SetShareMode: usize,
    GetPropertyValue: usize,
    SetPropertyValue: usize,
    SetDefaultEndpoint:
        unsafe extern "system" fn(this: *mut core::ffi::c_void, wide_id: *const u16, role: i32) -> windows::core::HRESULT,
    SetEndpointVisibility: usize,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Set the default render endpoint for both console and multimedia roles.
pub fn set_default_device(device_id: &str) -> Result<(), String> {
    unsafe {
        let unknown: IUnknown = CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
            .map_err(|e| e.to_string())?;
        let this = unknown.as_raw() as *mut core::ffi::c_void;
        if this.is_null() {
            return Err("policy config null".into());
        }
        let vtbl = *(this as *mut *mut IPolicyConfigVtbl);
        let id = wide(device_id);
        let set_default = unsafe { (*vtbl).SetDefaultEndpoint };
        for role in [eConsole.0, eMultimedia.0] {
            let hr = unsafe { set_default(this, id.as_ptr(), role) };
            if hr.is_err() {
                return Err(format!("SetDefaultEndpoint failed: {hr:?}"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn device_friendly_name(device: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
            return String::new();
        };
        let key = PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };
        let Ok(pv) = store.GetValue(&key) else {
            return String::new();
        };
        // VT_LPWSTR
        if pv.Anonymous.Anonymous.vt == windows::Win32::System::Variant::VARENUM(31) {
            let ptr = pv.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|i| *ptr.0.add(*i) != 0).count();
                return String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, len));
            }
        }
        String::new()
    }
}

/// Enumerate active render endpoints with names + default flag.
pub fn audio_devices() -> Result<Vec<AudioDevice>, String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let default = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let default_id = match default.GetId() {
            Ok(id) => {
                let len = (0..).take_while(|i| unsafe { *id.0.add(*i) } != 0).count();
                unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(id.0, len)) }
            }
            Err(_) => String::new(),
        };

        let count = collection.GetCount().map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for i in 0..count {
            let Ok(device) = collection.Item(i) else { continue };
            let id = match device.GetId() {
                Ok(id) => {
                    let len = (0..).take_while(|i| unsafe { *id.0.add(*i) } != 0).count();
                    unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(id.0, len)) }
                }
                Err(_) => String::new(),
            };
            let name = device_friendly_name(&device);
            let is_default = id == default_id;
            out.push(AudioDevice {
                id,
                name,
                is_default,
            });
        }
        out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
        Ok(out)
    }
}

