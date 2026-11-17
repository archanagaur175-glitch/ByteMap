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
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use crate::{PROTOCOL_MAGIC, PROTOCOL_VERSION};

    /// Serve clients one at a time until `shutdown` is seen between clients.
    pub(super) fn serve(pipe_name: &str, shutdown: &AtomicBool) -> io::Result<()> {
        while !shutdown.load(Ordering::SeqCst) {
            let handle = create_pipe_instance(pipe_name)?;
            if let Err(err) = accept(handle) {
                unsafe { CloseHandle(handle) };
                return Err(err);
            }
            // Serve this client to completion (handshake, then whatever the
            // session does) before ever accepting another.
            let outcome = handshake(handle).and_then(|()| drain_client(handle));
            unsafe { CloseHandle(handle) };
            if let Err(err) = outcome {
                // A rejected or malformed client is already disconnected and
                // must not take the server down.
                eprintln!("bytemap-helper: client rejected: {err}");
            }
        }
        Ok(())
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

    /// Handshake. Client sends magic (4 bytes) + protocol version (u16 LE);
    /// server answers magic + its version + status byte (0 ok, 1 bad magic,
    /// 2 bad version). A non-zero status ends the connection.
    fn handshake(handle: HANDLE) -> io::Result<()> {
        let mut request = [0u8; 6];
        read_exact(handle, &mut request)?;
        let version = u16::from_le_bytes([request[4], request[5]]);
        let (status, verdict) = if request[0..4] != PROTOCOL_MAGIC {
            (1u8, Some("bad magic"))
        } else if version != PROTOCOL_VERSION {
            (2, Some("bad version"))
        } else {
            (0, None)
        };
        let mut reply = Vec::with_capacity(7);
        reply.extend_from_slice(&PROTOCOL_MAGIC);
        reply.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        reply.push(status);
        write_all(handle, &reply)?;
        match verdict {
            Some(reason) => Err(io::Error::new(io::ErrorKind::InvalidData, reason)),
            None => Ok(()),
        }
    }

    /// Consume the client's post-handshake bytes (none in this skeleton) and
    /// wait for the disconnect. EOF, `ERROR_BROKEN_PIPE` and `ERROR_NO_DATA`
    /// are the normal end of a client session.
    fn drain_client(handle: HANDLE) -> io::Result<()> {
        let mut byte = [0u8; 1];
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                byte.as_mut_ptr(),
                1,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            // Stray bytes are ignored in this skeleton (P6.T2 streams frames).
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code as u32 == ERROR_BROKEN_PIPE || code as u32 == ERROR_NO_DATA => {
                Ok(())
            }
            _ => Err(err),
        }
    }

    fn read_exact(handle: HANDLE, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let mut read = 0u32;
            let ok = unsafe {
                ReadFile(
                    handle,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client disconnected mid-handshake",
                ));
            }
            buf = &mut buf[read as usize..];
        }
        Ok(())
    }

    fn write_all(handle: HANDLE, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle,
                    buf.as_ptr(),
                    buf.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "client stopped reading mid-handshake",
                ));
            }
            buf = &buf[written as usize..];
        }
        Ok(())
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
