//! bytemap-helper binary.
//!
//! Runs elevated (requireAdministrator manifest embedded by build.rs) and
//! serves exactly one pipe, [`DEFAULT_PIPE_NAME`], until killed. The app
//! starts it once per session (T4 wires the lifetime) and talks to it over
//! the pipe; the main app never runs as administrator.
//!
//! `--parent <pid>`: exit when that process is gone (checked between
//! clients). The app always passes its own pid at spawn time, so closing
//! the app leaves no stray elevated helper. Omitted when the owner starts
//! the helper by hand — it then serves until killed or Ctrl+C.

use std::sync::atomic::AtomicBool;

fn main() {
    let shutdown = AtomicBool::new(false);
    let watch_parent = parse_parent_arg();
    if let Err(err) =
        bytemap_helper::serve(bytemap_helper::DEFAULT_PIPE_NAME, &shutdown, watch_parent)
    {
        eprintln!("bytemap-helper: serve failed: {err}");
        std::process::exit(1);
    }
}

/// `--parent <pid>` anywhere in the args; anything else is ignored so the
/// command line stays extensible.
fn parse_parent_arg() -> Option<u32> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--parent" {
            return args.next().and_then(|value| value.parse().ok());
        }
    }
    None
}
