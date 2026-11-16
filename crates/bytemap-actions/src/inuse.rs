//! Restart Manager probe: is a file in use? (brief: dry run lists the
//! in-use status). Pure FFI over windows-sys; failure is reported as an
//! error, never silently treated as "not in use" by the caller.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, RM_PROCESS_INFO,
};

/// Wide (UTF-16, NUL-terminated) copy of a path for the Win32 API.
fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Is `path` held open by any process (per Restart Manager)?
/// `Ok(false)` means verified not in use; `Err(reason)` means we could not
/// determine it — the caller must treat that as unknown, not as free.
pub fn file_in_use(path: &str) -> Result<bool, String> {
    let wpath = wide(path);
    let files: [*const u16; 1] = [wpath.as_ptr()];

    // Session key: any NUL-terminated unique string (docs: GUID-like).
    let key = format!("ByteMap-{:x}-{:x}", std::process::id(), now_ticks());
    let mut wkey = wide(&key);

    unsafe {
        let mut session: u32 = 0;
        let rc: WIN32_ERROR = RmStartSession(&mut session, 0, wkey.as_mut_ptr());
        if rc != ERROR_SUCCESS {
            return Err(format!("RmStartSession failed: win32 error {rc}"));
        }

        let rc: WIN32_ERROR =
            RmRegisterResources(session, 1, files.as_ptr(), 0, ptr::null(), 0, ptr::null());
        if rc != ERROR_SUCCESS {
            RmEndSession(session);
            return Err(format!("RmRegisterResources failed: win32 error {rc}"));
        }

        // First ask with a decent-size buffer; ERROR_MORE_DATA just means
        // more processes than we reserved — which is itself "in use".
        const CAP: usize = 64;
        let mut info: Vec<RM_PROCESS_INFO> = Vec::with_capacity(CAP);
        let mut needed: u32 = 0;
        let mut count: u32 = CAP as u32;
        let mut reboot_required: u32 = 0;
        let rc: WIN32_ERROR = RmGetList(
            session,
            &mut needed,
            &mut count,
            info.as_mut_ptr(),
            &mut reboot_required,
        );
        RmEndSession(session);

        match rc {
            ERROR_SUCCESS => Ok(count > 0),
            ERROR_MORE_DATA => Ok(true), // more holders than the buffer: in use
            other => Err(format!("RmGetList failed: win32 error {other}")),
        }
    }
}

fn now_ticks() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
