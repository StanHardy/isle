use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Store a secret in Windows Credential Manager (per-user, DPAPI-backed).
pub fn set_secret(target: &str, secret: &str) -> Result<(), String> {
    let mut wide_target = wide(target);
    let blob = secret.as_bytes();
    let mut cred = CREDENTIALW::default();
    cred.Type = CRED_TYPE_GENERIC;
    cred.TargetName = PWSTR(wide_target.as_mut_ptr());
    cred.CredentialBlobSize = blob.len() as u32;
    cred.CredentialBlob = blob.as_ptr() as *mut u8;
    cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
    unsafe { CredWriteW(&cred, 0).map_err(|e| e.to_string()) }
}

pub fn get_secret(target: &str) -> Option<String> {
    let wide_target = wide(target);
    unsafe {
        let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(
            PCWSTR(wide_target.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut ptr,
        )
        .ok()?;
        if ptr.is_null() {
            return None;
        }
        let cred = &*ptr;
        let size = cred.CredentialBlobSize as usize;
        let out = if size == 0 || cred.CredentialBlob.is_null() {
            String::new()
        } else {
            let bytes = std::slice::from_raw_parts(cred.CredentialBlob, size);
            String::from_utf8_lossy(bytes).to_string()
        };
        CredFree(ptr.cast());
        Some(out)
    }
}

pub fn delete_secret(target: &str) -> Result<(), String> {
    let wide_target = wide(target);
    unsafe {
        CredDeleteW(PCWSTR(wide_target.as_ptr()), CRED_TYPE_GENERIC, None)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wide_appends_nul() {
        assert_eq!(super::wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
    }
}
