//! bytemap-helper — ByteMap's elevated scan helper (Phase 6).
//!
//! The main ByteMap app NEVER runs as administrator (brief rule). Instead,
//! this tiny helper process runs elevated (requireAdministrator manifest)
//! and serves a local named pipe, [`DEFAULT_PIPE_NAME`], over which the app
//! requests MFT scans. P6.T1 is the skeleton: pipe server + handshake.
//! P6.T2 formalizes the frame protocol and streams MFT records.
//!
//! Serving rules (fail-closed):
//! - one pipe instance at a time with `nMaxInstances = 1` and
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`: while a client is served, a second
//!   client gets `ERROR_PIPE_BUSY`, and a second server process cannot
//!   create the pipe at all (refuses to serve two masters);
//! - every client must present the [`PROTOCOL_MAGIC`] handshake first;
//! - a rejected or malformed client is disconnected and never takes the
//!   server down.
//!
//! Documented limitation, in scope for P6.T2/T3: the `shutdown` flag is
//! observed between clients; while the helper is blocked waiting for the
//! next client it stays blocked. The binary is expected to be terminated
//! when the app exits (T4 wires the helper lifetime).

use std::io;
use std::sync::atomic::AtomicBool;

/// The pipe the helper serves. Local machine only; no network exposure.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\bytemap-scan";

/// Frame magic for every protocol message ("BYM1").
pub const PROTOCOL_MAGIC: [u8; 4] = *b"BYM1";
/// Protocol version. Bumped on any wire-format change.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(windows)]
pub mod proto;

/// Serve the named pipe until `shutdown` is observed (between clients).
///
/// Blocks. Errors from pipe creation or connect are returned; client
/// protocol errors are reported to stderr and the server keeps serving.
pub fn serve(pipe_name: &str, shutdown: &AtomicBool) -> io::Result<()> {
    #[cfg(windows)]
    {
        pipe::serve(pipe_name, shutdown)
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe_name, shutdown);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bytemap-helper only serves on Windows",
        ))
    }
}

#[cfg(windows)]
mod pipe {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use crate::proto::{self, Frame, HandshakeError};

    /// Serve clients one at a time until `shutdown` is seen between clients.
    pub(super) fn serve(pipe_name: &str, shutdown: &AtomicBool) -> io::Result<()> {
        while !shutdown.load(Ordering::SeqCst) {
            let handle = create_pipe_instance(pipe_name)?;
            if let Err(err) = accept(handle) {
                unsafe { CloseHandle(handle) };
                return Err(err);
            }
            // The File owns the pipe handle from here on: dropping it closes
            // the instance. Serve this client to completion before ever
            // accepting another.
            let mut stream = unsafe { File::from_raw_handle(handle) };
            let outcome = run_session(&mut stream);
            drop(stream);
            if let Err(err) = outcome {
                // A rejected or malformed client is already disconnected and
                // must not take the server down.
                eprintln!("bytemap-helper: client session ended: {err}");
            }
        }
        Ok(())
    }

    /// One client session: handshake, one scan request, frames back.
    fn run_session(stream: &mut File) -> Result<(), SessionError> {
        proto::handshake_server(stream).map_err(SessionError::Handshake)?;
        let request = proto::read_frame(stream)?;
        match request {
            Frame::ScanRequest { .. } => {
                // P6.T3 streams the real MFT records here. Fail visibly,
                // on the wire, until then.
                proto::write_frame(
                    stream,
                    &Frame::Error {
                        message: "mft streaming not wired yet (P6.T3)".to_owned(),
                    },
                )?;
                Ok(())
            }
            other => Err(SessionError::Protocol(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected a scan request, got {other:?}"),
            ))),
        }
    }

    enum SessionError {
        Handshake(HandshakeError),
        Protocol(io::Error),
    }

    impl From<io::Error> for SessionError {
        fn from(err: io::Error) -> Self {
            SessionError::Protocol(err)
        }
    }

    impl core::fmt::Display for SessionError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                SessionError::Handshake(err) => write!(f, "{err}"),
                SessionError::Protocol(err) => write!(f, "{err}"),
            }
        }
    }

    /// Wait for one client. Success or `ERROR_PIPE_CONNECTED` (the client
    /// connected between instance creation and the connect call) both count.
    fn accept(handle: HANDLE) -> io::Result<()> {
        let ok = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if ok != 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
            return Ok(());
        }
        Err(err)
    }

    /// Create one pipe instance. `nMaxInstances = 1` plus
    /// `FILE_FLAG_FIRST_PIPE_INSTANCE`: a busy server makes extra clients
    /// fail with `ERROR_PIPE_BUSY`, and a second server process cannot
    /// create the pipe at all.
    fn create_pipe_instance(name: &str) -> io::Result<HANDLE> {
        let wide: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::ffi::OsStr;
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};

    use super::*;

    /// Every test gets its own pipe name so tests never collide.
    fn unique_pipe(tag: &str) -> String {
        format!(r"\\.\pipe\bytemap-test-{tag}-{}", std::process::id())
    }

    /// Connect as a client, retrying while the server thread spins up.
    fn connect(name: &str) -> std::fs::File {
        let wide: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        for attempt in 0..200 {
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
                return unsafe { std::fs::File::from_raw_handle(handle) };
            }
            assert!(
                attempt < 199,
                "connect to {name} failed: {}",
                std::io::Error::last_os_error()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        unreachable!()
    }

    #[test]
    fn serve_handshakes_a_client_and_stops_on_shutdown() {
        let name = unique_pipe("ok");
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, shutdown) = (name.clone(), Arc::clone(&shutdown));
            std::thread::spawn(move || serve(&name, &shutdown))
        };
        let mut client = connect(&name);
        client.write_all(&PROTOCOL_MAGIC).unwrap();
        client.write_all(&PROTOCOL_VERSION.to_le_bytes()).unwrap();
        let mut reply = [0u8; 7];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(&reply[0..4], &PROTOCOL_MAGIC);
        assert_eq!(u16::from_le_bytes([reply[4], reply[5]]), PROTOCOL_VERSION);
        assert_eq!(reply[6], 0, "handshake must be accepted");
        // Ask the server to stop after this client, then disconnect. The
        // server finishes the session, sees the flag, and exits Ok.
        shutdown.store(true, Ordering::SeqCst);
        drop(client);
        server.join().unwrap().expect("serve failed");
    }

    #[test]
    fn serve_rejects_a_bad_magic_client_and_keeps_serving() {
        let name = unique_pipe("badmagic");
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, shutdown) = (name.clone(), Arc::clone(&shutdown));
            std::thread::spawn(move || serve(&name, &shutdown))
        };
        let mut client = connect(&name);
        client.write_all(b"NOPE").unwrap();
        client.write_all(&PROTOCOL_VERSION.to_le_bytes()).unwrap();
        let mut reply = [0u8; 7];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[6], 1, "bad magic must be rejected with status 1");
        // The rejected client must not take the server down: the next
        // honest client handshakes fine.
        let mut client2 = connect(&name);
        client2.write_all(&PROTOCOL_MAGIC).unwrap();
        client2.write_all(&PROTOCOL_VERSION.to_le_bytes()).unwrap();
        let mut reply2 = [0u8; 7];
        client2.read_exact(&mut reply2).unwrap();
        assert_eq!(reply2[6], 0);
        shutdown.store(true, Ordering::SeqCst);
        drop(client2);
        server.join().unwrap().expect("serve failed");
    }

    #[test]
    fn serve_refuses_a_second_master_while_busy() {
        let name = unique_pipe("busy");
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, shutdown) = (name.clone(), Arc::clone(&shutdown));
            std::thread::spawn(move || serve(&name, &shutdown))
        };
        let mut client = connect(&name);
        client.write_all(&PROTOCOL_MAGIC).unwrap();
        client.write_all(&PROTOCOL_VERSION.to_le_bytes()).unwrap();
        let mut reply = [0u8; 7];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[6], 0);
        // While client 1 is being served, a second client must fail with
        // ERROR_PIPE_BUSY (single-instance pipe, one master at a time).
        let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
        let second = unsafe {
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
        assert_eq!(
            second, INVALID_HANDLE_VALUE,
            "second master must be refused"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(ERROR_PIPE_BUSY as i32),
            "refusal must be ERROR_PIPE_BUSY"
        );
        shutdown.store(true, Ordering::SeqCst);
        drop(client);
        server.join().unwrap().expect("serve failed");
    }
}
