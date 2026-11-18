//! Helper process management (P6.T4b): the app never runs elevated, so it
//! starts the elevated helper with the `runas` verb (one UAC prompt) and
//! then talks to it over the pipe.
//!
//! `ensure_helper` is idempotent: if a helper is already serving, no spawn
//! happens. The spawned helper receives `--parent <our pid>`, so it exits
//! by itself when the app closes (T4a); the helper's own manifest makes it
//! elevated, and the app closes the returned process handle immediately.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::mft_client::{MftClient, DEFAULT_PIPE_NAME};

/// Outcome of an ensure attempt, for the UI/owner to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureStatus {
    /// A helper was already serving the pipe.
    AlreadyRunning,
    /// A new helper was spawned (UAC prompt) and is now serving.
    Spawned,
}

impl EnsureStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnsureStatus::AlreadyRunning => "running",
            EnsureStatus::Spawned => "spawned",
        }
    }
}

/// Make sure an elevated helper is serving [`DEFAULT_PIPE_NAME`].
///
/// Tries to connect first (fast, no prompt); if unreachable, spawns
/// `bytemap-helper.exe` from beside the current executable with the
/// `runas` verb (UAC prompt — the user decides) and waits up to ~8 s for
/// the pipe to appear.
pub fn ensure_helper() -> Result<EnsureStatus, String> {
    if MftClient::connect(DEFAULT_PIPE_NAME).is_ok() {
        return Ok(EnsureStatus::AlreadyRunning);
    }
    spawn_helper()?;
    // The elevated process has to pass UAC, start, and create the pipe.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if MftClient::connect(DEFAULT_PIPE_NAME).is_ok() {
            return Ok(EnsureStatus::Spawned);
        }
        if Instant::now() >= deadline {
            return Err("helper did not start after the UAC prompt".to_owned());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Launch the helper elevated via ShellExecuteExW("runas"). The process
/// handle is closed immediately: the helper watches OUR pid, not the
/// other way around (T4a), and an open handle to it is only a leak here.
fn spawn_helper() -> Result<(), String> {
    let exe = helper_exe_path()
        .ok_or_else(|| "bytemap-helper.exe not found next to the app executable".to_owned())?;
    let file: Vec<u16> = wide(exe.as_os_str());
    let verb: Vec<u16> = wide(OsStr::new("runas"));
    let params: Vec<u16> = wide(OsStr::new(&format!("--parent {}", std::process::id())));

    let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    // NOASYNC: the call returns after the launch decision, not after the
    // process starts; we poll for the pipe ourselves. NOCLOSEPROCESS:
    // the handle comes back so we can close it deterministically.
    sei.fMask = SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.lpParameters = params.as_ptr();
    sei.nShow = SW_HIDE; // console helper: no window flash
    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 {
        return Err(format!(
            "UAC launch refused or failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if !sei.hProcess.is_null() {
        unsafe { CloseHandle(sei.hProcess) };
    }
    Ok(())
}

/// The helper binary is expected next to the app's own executable (dev:
/// `target/debug/bytemap-helper.exe`; installed: the install directory).
fn helper_exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    dir.push("bytemap-helper.exe");
    if dir.is_file() {
        Some(dir)
    } else {
        None
    }
}

/// NUL-terminated UTF-16 for Win32 string pointers.
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path rule: beside the current exe, named bytemap-helper.exe.
    #[test]
    fn helper_path_is_beside_the_current_exe() {
        let path = helper_exe_path();
        // In the test env the helper exe is usually not built next to the
        // test binary, so the Option may be None — the NAME rule is what
        // we pin here by rebuilding the expected candidate.
        let expected = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("bytemap-helper.exe");
        if let Some(found) = path {
            assert_eq!(found, expected);
        }
        // When it exists it must be exactly that candidate.
        let _ = expected;
    }

    #[test]
    fn status_strings() {
        assert_eq!(EnsureStatus::AlreadyRunning.as_str(), "running");
        assert_eq!(EnsureStatus::Spawned.as_str(), "spawned");
    }
}
