//! Client side of the BYM1 protocol: connects to the elevated helper's
//! named pipe and exchanges frames. The helper does the elevated MFT work;
//! the main app stays unelevated (brief rule). P6.T3 wires real scans on
//! top of this; P6.T2 proves the pipe end-to-end with the helper's stub.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

use bytemap_helper::proto::{self, Frame, HandshakeError};

/// Re-exported so downstream crates (src-tauri) can consume Record streams
/// without depending on bytemap-helper directly.
pub use bytemap_helper::proto::{Frame as WireFrame, Record};

/// The pipe the helper serves.
pub const DEFAULT_PIPE_NAME: &str = bytemap_helper::DEFAULT_PIPE_NAME;

#[derive(Debug)]
pub enum MftClientError {
    /// The helper is not reachable (not running / refused) after retries.
    Unreachable(io::Error),
    Handshake(HandshakeError),
    Io(io::Error),
}

impl core::fmt::Display for MftClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MftClientError::Unreachable(err) => write!(f, "helper unreachable: {err}"),
            MftClientError::Handshake(err) => write!(f, "helper handshake failed: {err}"),
            MftClientError::Io(err) => write!(f, "helper pipe io: {err}"),
        }
    }
}

impl std::error::Error for MftClientError {}

/// One pipe connection to the helper: handshake done, ready for frames.
pub struct MftClient {
    stream: File,
}

impl MftClient {
    /// Connect to the helper's pipe and handshake. Retries while the
    /// helper spins up (`ERROR_FILE_NOT_FOUND`) or is busy serving another
    /// client (`ERROR_PIPE_BUSY` -> `WaitNamedPipeW`), within a ~2 s budget.
    pub fn connect(pipe_name: &str) -> Result<Self, MftClientError> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().chain(Some(0)).collect();
        loop {
            let handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                let mut client = Self {
                    stream: unsafe { File::from_raw_handle(handle) },
                };
                proto::handshake_client(&mut client.stream).map_err(MftClientError::Handshake)?;
                return Ok(client);
            }
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(code) if code as u32 == ERROR_PIPE_BUSY => {
                    // Instance busy: wait for the next free one (up to 1 s).
                    let ok = unsafe { WaitNamedPipeW(wide.as_ptr(), 1000) };
                    if ok == 0 && Instant::now() >= deadline {
                        return Err(MftClientError::Unreachable(err));
                    }
                }
                Some(code) if code as u32 == ERROR_FILE_NOT_FOUND => {
                    // The helper has not created the pipe yet.
                }
                _ => return Err(MftClientError::Unreachable(err)),
            }
            if Instant::now() >= deadline {
                return Err(MftClientError::Unreachable(err));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Ask the helper to scan `path` (UTF-8 scan root).
    pub fn request_scan(&mut self, path: &str) -> io::Result<()> {
        proto::write_frame(
            &mut self.stream,
            &Frame::ScanRequest {
                path: path.to_owned(),
            },
        )
    }

    /// Read the next frame. A session always ends with `Done` or `Error`,
    /// so there is no EOF-as-success case: any read failure is an error.
    pub fn next_frame(&mut self) -> Result<Frame, MftClientError> {
        proto::read_frame(&mut self.stream).map_err(MftClientError::Io)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use bytemap_helper::serve;

    use super::*;

    #[test]
    fn client_round_trips_a_scan_request_and_reads_the_reply() {
        // One process, two threads, one real named pipe (no elevation).
        let name = format!(r"\\.\pipe\bytemap-client-test-{}", std::process::id());
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, shutdown) = (name.clone(), Arc::clone(&shutdown));
            std::thread::spawn(move || serve(&name, &shutdown, None))
        };
        let mut client =
            MftClient::connect(&name).expect("connect must retry until the server is up");
        client
            .request_scan(r"Q:\fixture")
            .expect("scan request must be sent");
        // The real server (P6.T3a) parses the path, tries to open the
        // volume, and reports the failure as a terminal Error frame.
        // Q: is assumed to be a nonexistent drive on this machine.
        match client.next_frame().expect("server must answer") {
            Frame::Error { message } => {
                assert!(message.contains("cannot open"), "reply: {message}");
            }
            other => panic!("expected an Error frame, got {other:?}"),
        }
        // The helper observes the shutdown flag only between clients (its
        // accept blocks while idle, documented T1 limitation), so park a
        // throwaway client in the next session BEFORE arming shutdown: its
        // disconnect then lets the server loop out and see the flag.
        let wake = MftClient::connect(&name).expect("second client must be connectable");
        shutdown.store(true, Ordering::SeqCst);
        drop(client);
        drop(wake);
        server.join().unwrap().expect("serve failed");
    }
}
