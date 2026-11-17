//! bytemap-helper binary.
//!
//! Runs elevated (requireAdministrator manifest embedded by build.rs) and
//! serves exactly one pipe, [`DEFAULT_PIPE_NAME`], until killed. The app
//! starts it once per session (T4 wires the lifetime) and talks to it over
//! the pipe; the main app never runs as administrator.

use std::sync::atomic::AtomicBool;

fn main() {
    let shutdown = AtomicBool::new(false);
    if let Err(err) = bytemap_helper::serve(bytemap_helper::DEFAULT_PIPE_NAME, &shutdown) {
        eprintln!("bytemap-helper: serve failed: {err}");
        std::process::exit(1);
    }
}
