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
//! Lifetime (P6.T4a): when started with `--parent <pid>` (the app does
//! this at spawn time), the helper exits as soon as that process is gone,
//! checked between clients — an elevated process cannot be killed by the
//! unelevated app, so the helper notices its parent's death itself. When
//! the owner starts it by hand (no `--parent`), it serves until killed or
//! Ctrl+C. The check happens between clients: the helper finishes the
//! current scan session first.
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
/// Protocol version. Bumped on any wire-format change. 2: Record gained
/// `mtime` (P6.T3a). 3: USN refresh frames (P8.T3).
pub const PROTOCOL_VERSION: u16 = 3;

#[cfg(windows)]
pub mod mft;
#[cfg(windows)]
pub mod proto;
#[cfg(windows)]
pub mod usn;

/// Serve the named pipe until `shutdown` is observed (between clients), or
/// — when a parent pid is being watched — until that process is gone.
///
/// Blocks. Errors from pipe creation or connect are returned; client
/// protocol errors are reported to stderr and the server keeps serving.
pub fn serve(pipe_name: &str, shutdown: &AtomicBool, watch_parent: Option<u32>) -> io::Result<()> {
    #[cfg(windows)]
    {
        pipe::serve(pipe_name, shutdown, watch_parent)
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe_name, shutdown, watch_parent);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bytemap-helper only serves on Windows",
        ))
    }
}

/// Whether `pid` names a running process. `OpenProcess` with
/// PROCESS_QUERY_LIMITED_INFORMATION fails for a dead pid; access denied
/// would still mean it exists, but an elevated helper can always query a
/// lower-integrity process this way. Pid reuse (dead parent's id picked up
/// by another process) keeps the helper alive — a documented, harmless
/// limitation.
#[cfg(windows)]
fn parent_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

#[cfg(windows)]
mod pipe {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use crate::mft;
    use crate::parent_alive;
    use crate::proto::{self, Frame, HandshakeError};

    /// Serve clients one at a time until `shutdown` is seen between
    /// clients, or — when a parent pid is being watched — until that
    /// process is gone (P6.T4a). A watcher thread detects the death and
    /// unblocks the accept loop with a dummy client, because the helper
    /// spends its idle life blocked in `ConnectNamedPipe` and would never
    /// reach a between-clients check on its own.
    pub fn serve(
        pipe_name: &str,
        shutdown: &AtomicBool,
        watch_parent: Option<u32>,
    ) -> io::Result<()> {
        std::thread::scope(|scope| {
            if let Some(pid) = watch_parent {
                scope.spawn(move || watch_parent_process(pipe_name, pid, shutdown));
            }
            let result = (|| {
                while !shutdown.load(Ordering::SeqCst) {
                    let handle = create_pipe_instance(pipe_name)?;
                    if let Err(err) = accept(handle) {
                        unsafe { CloseHandle(handle) };
                        return Err(err);
                    }
                    // The File owns the pipe handle from here on: dropping
                    // it closes the instance. Serve this client to
                    // completion before ever accepting another.
                    let mut stream = unsafe { File::from_raw_handle(handle) };
                    let outcome = run_session(&mut stream);
                    drop(stream);
                    if let Err(err) = outcome {
                        // A rejected or malformed client is already
                        // disconnected and must not take the server down.
                        eprintln!("bytemap-helper: client session ended: {err}");
                    }
                }
                Ok(())
            })();
            // Release the watcher on every exit path; it exits on its
            // next poll.
            shutdown.store(true, Ordering::SeqCst);
            result
        })
    }

    /// Parent watch (P6.T4a): when the launching process dies, set the
    /// shutdown flag and force one dummy connection so the blocked
    /// `ConnectNamedPipe` returns and the serve loop can observe the
    /// flag. A session already in progress finishes first (documented).
    fn watch_parent_process(pipe_name: &str, pid: u32, shutdown: &AtomicBool) {
        loop {
            if shutdown.load(Ordering::SeqCst) {
                return; // serve ended for another reason
            }
            if !parent_alive(pid) {
                // The app that launched us is gone: no stray elevated
                // helper left behind.
                shutdown.store(true, Ordering::SeqCst);
                // The serve loop is usually blocked in accept. A dummy
                // client ends that wait; the handshake fails fast and
                // the loop top sees the flag. Retry a few times in case
                // a session (or instance churn) is in progress.
                for _ in 0..20 {
                    match dummy_connect(pipe_name) {
                        Ok(client) => {
                            drop(client);
                            return;
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(250)),
                    }
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// One raw client connection for unblocking the accept loop; no
    /// handshake is sent, so the server session ends at the first read.
    fn dummy_connect(pipe_name: &str) -> io::Result<File> {
        let wide: Vec<u16> = OsStr::new(pipe_name).encode_wide().chain(Some(0)).collect();
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
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    /// One client session: handshake, one scan request, frames back.
    /// Visible to tests so a session can run over in-memory buffers.
    pub(super) fn run_session<S: Read + Write>(stream: &mut S) -> Result<(), SessionError> {
        proto::handshake_server(stream).map_err(SessionError::Handshake)?;
        match proto::read_frame(stream)? {
            Frame::ScanRequest { path } => {
                // Streams the real MFT records (P6.T3a). Every failure is
                // reported on the wire as an Error frame, which ends the
                // session normally.
                mft::handle_scan_request(stream, &path);
                Ok(())
            }
            Frame::UsnRequest {
                path,
                journal_id,
                since_usn,
            } => {
                // Streams the journal delta (P8.T3). Same fail-closed rule:
                // a journal gap or a directory rename becomes an Error frame.
                crate::usn::handle_usn_request(stream, &path, journal_id, since_usn);
                Ok(())
            }
            other => Err(SessionError::Protocol(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected a scan or usn request, got {other:?}"),
            ))),
        }
    }

    #[derive(Debug)]
    pub(super) enum SessionError {
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
            std::thread::spawn(move || serve(&name, &shutdown, None))
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
            std::thread::spawn(move || serve(&name, &shutdown, None))
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
            std::thread::spawn(move || serve(&name, &shutdown, None))
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

    #[test]
    fn parent_alive_sees_itself_but_not_a_bogus_pid() {
        assert!(parent_alive(std::process::id()));
        assert!(!parent_alive(u32::MAX), "u32::MAX is never a valid pid");
    }

    /// The app's lifetime contract: a helper watching a parent that dies
    /// mid-session exits by itself after that client session ends.
    #[test]
    fn serve_exits_when_the_watched_parent_is_gone() {
        use std::os::windows::process::CommandExt;
        // A real, long-enough child process (no console window flash);
        // its pid is the watched "parent". Killed after the handshake.
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping", "-n", "30", "127.0.0.1", ">nul"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .expect("spawn a helper-parent stand-in");
        let name = unique_pipe("orphan");
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, shutdown) = (name.clone(), Arc::clone(&shutdown));
            let pid = child.id();
            std::thread::spawn(move || serve(&name, &shutdown, Some(pid)))
        };
        let mut client = connect(&name);
        eprintln!("[dbg-client] connected");
        client.write_all(&PROTOCOL_MAGIC).unwrap();
        client.write_all(&PROTOCOL_VERSION.to_le_bytes()).unwrap();
        let mut reply = [0u8; 7];
        client.read_exact(&mut reply).unwrap();
        eprintln!("[dbg-client] handshake done");
        assert_eq!(reply[6], 0);

        // The watched "parent" dies while the helper idles between
        // clients: the watcher thread must notice, set the shutdown flag,
        // and unblock the accept loop — serve then exits with Ok without
        // ever being asked via the flag directly.
        eprintln!("[dbg-client] killing stand-in");
        child.kill().expect("kill the stand-in parent");
        child.wait().expect("reap the stand-in parent");
        eprintln!("[dbg-client] killed+reaped");
        // Drop the Child LAST: its open process handle keeps the
        // terminated process object alive, which makes the watched pid
        // look alive. The real app closes its runas handle right after
        // spawn, so production never hits this.
        drop(child);
        // Ending the client session is what lets the server reach the
        // loop-top parent check: it cannot exit while a session is open.
        drop(client);
        server
            .join()
            .unwrap()
            .expect("serve must exit when parent is gone");
    }
}
