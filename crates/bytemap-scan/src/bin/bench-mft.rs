//! MFT fast-path benchmark + gates (P6.T4c, adjusted by P6.T4d).
//!
//! Two modes:
//!
//! 1. **Parity (default)** — scans `%TEMP%\bytemap-bench-100k` through BOTH
//!    tiers and demands identical results: file/dir/denied counts and the
//!    root's logical bytes. The fixture is a folder, so the helper uses its
//!    two-pass subtree path (the only place that path is still exercised).
//!
//!        cargo run --release -p bytemap-scan --bin bench-mft
//!
//! 2. **Speed (`--whole-volume [drive]`)** — streams a whole volume
//!    (default: `%SystemDrive%\`) through the helper's single-pass fast
//!    path and requires >= 333k entries/s, the gate number from the P1.T7
//!    spike. The walker scans the same volume for comparison; because a
//!    live volume changes between the two scans, only the raw counts are
//!    compared (<= 1% drift), never exact parity.
//!
//!        cargo run --release -p bytemap-scan --bin bench-mft -- --whole-volume
//!
//! Both modes need the elevated helper; when none is serving, this bin
//! starts one with `runas` (UAC prompt). RUN BY THE OWNER.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bytemap_core::Tree;
use bytemap_scan::mft_client::{MftClient, WireFrame, DEFAULT_PIPE_NAME};
use bytemap_scan::mft_tree::build_tree_from_records;
use bytemap_scan::spawn::ensure_helper;
use bytemap_scan::{scan_folder, ScanOptions, ScanStats};

/// The parity fixture: 101,112 entries, deterministic.
fn fixture_root() -> PathBuf {
    std::env::temp_dir().join("bytemap-bench-100k")
}

/// `%SystemDrive%\` (e.g. `C:\`).
fn system_drive() -> String {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    format!("{drive}\\")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let whole_volume = args
        .iter()
        .position(|arg| arg == "--whole-volume")
        .map(|at| args.get(at + 1).cloned());
    let ok = match whole_volume {
        Some(drive) => {
            let drive = drive.unwrap_or_else(system_drive);
            let root = if drive.ends_with(['\\', '/']) {
                drive
            } else {
                format!("{drive}\\")
            };
            run_whole_volume(&root)
        }
        None => run_fixture(),
    };
    if !ok {
        std::process::exit(1);
    }
}

/// Tier B: reference scan.
fn walker_scan(root: &Path) -> Result<(Tree, ScanStats), String> {
    let mut tree = Tree::new();
    let stats =
        scan_folder(root, &mut tree, &ScanOptions::default(), None).map_err(|e| e.to_string())?;
    Ok((tree, stats))
}

/// The elevated helper must be up; a missing helper is a loud skip.
fn helper_or_skip() -> bool {
    match ensure_helper() {
        Ok(status) => {
            println!("[helper] {status:?}");
            true
        }
        Err(e) => {
            eprintln!(
                "SKIPPED LOUDLY: helper not available: {e}\n\
                 Build and start it (elevated) with:\n  \
                 cargo build --release -p bytemap-helper\n  \
                 cargo run --release -p bytemap-helper\n\
                 ...or accept the UAC prompt this bin triggers when the exe exists."
            );
            false
        }
    }
}

/// Tier A over a folder: stream the helper's records, build the tree, and
/// time the whole stream (the MFT metric; the tree build is part of it).
/// Returns (tree, stats, stream_ms).
fn mft_scan_folder(root: &Path) -> Result<(Tree, ScanStats, u128), String> {
    let mut client = MftClient::connect(DEFAULT_PIPE_NAME).map_err(|e| e.to_string())?;
    client
        .request_scan(&root.to_string_lossy())
        .map_err(|e| e.to_string())?;

    let stream_start = Instant::now();
    let mut failure: Option<String> = None;
    let stream = std::iter::from_fn(|| {
        if failure.is_some() {
            return None;
        }
        match client.next_frame() {
            Ok(WireFrame::Record(record)) => Some(record),
            Ok(WireFrame::Done { .. }) => None,
            Ok(WireFrame::Error { message }) => {
                failure = Some(message);
                None
            }
            Ok(other) => {
                failure = Some(format!("unexpected frame from helper: {other:?}"));
                None
            }
            Err(e) => {
                failure = Some(e.to_string());
                None
            }
        }
    });
    let (tree, stats) =
        build_tree_from_records(root, stream).map_err(|e| format!("tree build failed: {e}"))?;
    if let Some(message) = failure {
        return Err(format!("helper scan failed: {message}"));
    }
    Ok((tree, stats, stream_start.elapsed().as_millis()))
}

/// Tier A over a whole volume: stream frames and count them, no tree. The
/// speed gate must not pay for a 1.7M-node tree the UI would build anyway.
/// Returns (wire entries, wire logical bytes, stream_ms).
fn mft_stream_volume(root: &Path) -> Result<(u64, u64, u128), String> {
    let mut client = MftClient::connect(DEFAULT_PIPE_NAME).map_err(|e| e.to_string())?;
    client
        .request_scan(&root.to_string_lossy())
        .map_err(|e| e.to_string())?;

    let stream_start = Instant::now();
    let mut entries = 0u64;
    let mut logical = 0u64;
    loop {
        match client.next_frame().map_err(|e| e.to_string())? {
            WireFrame::Record(record) => {
                entries += 1;
                if record.attributes & 0x10 == 0 {
                    logical += record.logical_size;
                }
            }
            WireFrame::Done {
                entries: done_entries,
                logical_bytes: done_logical,
            } => {
                // The helper's own tally must match what arrived.
                if done_entries != entries || done_logical != logical {
                    return Err(format!(
                        "helper tally mismatch: done=({done_entries}, {done_logical}) wire=({entries}, {logical})"
                    ));
                }
                return Ok((entries, logical, stream_start.elapsed().as_millis()));
            }
            WireFrame::Error { message } => return Err(format!("helper scan failed: {message}")),
            other => return Err(format!("unexpected frame from helper: {other:?}")),
        }
    }
}

/// Mode 1: exact parity on the deterministic 100k fixture. Speed is
/// reported for information only — a 100k subtree is not what the gate
/// measures any more (P6.T4d).
fn run_fixture() -> bool {
    let root = fixture_root();
    if !root.is_dir() {
        eprintln!(
            "SKIPPED LOUDLY: {} does not exist. Run `cargo run --release -p bytemap-scan --bin bench-walk` first.",
            root.display()
        );
        std::process::exit(2);
    }

    // --- Tier B: walker ---
    let (walker_tree, walker_stats) = match walker_scan(&root) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("walker scan failed: {e}");
            return false;
        }
    };
    let walker_entries = walker_tree.len() as u64;
    let walker_per_sec = u128::from(walker_entries) * 1000 / walker_stats.elapsed_ms.max(1);
    println!(
        "[walker] entries={walker_entries} (files={} dirs={} denied={}) elapsed_ms={} entries/s={walker_per_sec}",
        walker_stats.files, walker_stats.dirs, walker_stats.denied, walker_stats.elapsed_ms
    );

    // --- Tier A: helper up (UAC when absent), MFT scan ---
    if !helper_or_skip() {
        std::process::exit(2);
    }
    let (mft_tree, mft_stats, stream_ms) = match mft_scan_folder(&root) {
        Ok(triple) => triple,
        Err(e) => {
            eprintln!("mft scan failed: {e}");
            return false;
        }
    };
    let mft_entries = mft_tree.len() as u64;
    let mft_per_sec = u128::from(mft_entries) * 1000 / stream_ms.max(1);
    println!(
        "[mft] entries={mft_entries} (files={} dirs={} denied={}) stream_ms={stream_ms} entries/s={mft_per_sec}",
        mft_stats.files, mft_stats.dirs, mft_stats.denied
    );

    // --- Parity gate ---
    let walker_logical = walker_tree
        .root()
        .map(|r| walker_tree.node(r).agg_logical());
    let mft_logical = mft_tree.root().map(|r| mft_tree.node(r).agg_logical());
    let parity_ok = walker_stats.files == mft_stats.files
        && walker_stats.dirs == mft_stats.dirs
        && walker_stats.denied == mft_stats.denied
        && walker_logical == mft_logical;
    if !parity_ok {
        eprintln!(
            "parity failed: walker files={} dirs={} denied={} logical={walker_logical:?}; \
             mft files={} dirs={} denied={} logical={mft_logical:?}",
            walker_stats.files,
            walker_stats.dirs,
            walker_stats.denied,
            mft_stats.files,
            mft_stats.dirs,
            mft_stats.denied
        );
    }
    println!(
        "BENCH-MFT mode=parity walker_entries={walker_entries} mft_entries={mft_entries} \
         mft_stream_ms={stream_ms} mft_per_sec={mft_per_sec} (informational; the speed gate \
         runs in --whole-volume mode) parity={} ",
        if parity_ok { "ok" } else { "FAILED" }
    );
    parity_ok
}

/// Mode 2: the speed gate on a whole volume.
fn run_whole_volume(root: &str) -> bool {
    let path = Path::new(root).to_path_buf();
    if !path.is_dir() {
        eprintln!(
            "SKIPPED LOUDLY: {} is not a readable volume root.",
            path.display()
        );
        std::process::exit(2);
    }

    // --- Tier A: helper up (UAC when absent), single-pass stream ---
    if !helper_or_skip() {
        std::process::exit(2);
    }
    let (mft_entries, mft_logical, stream_ms) = match mft_stream_volume(&path) {
        Ok(triple) => triple,
        Err(e) => {
            eprintln!("mft whole-volume scan failed: {e}");
            return false;
        }
    };
    let mft_per_sec = u128::from(mft_entries) * 1000 / stream_ms.max(1);
    println!(
        "[mft] volume={} entries={mft_entries} logical={mft_logical} stream_ms={stream_ms} entries/s={mft_per_sec}",
        path.display()
    );

    // --- Tier B: walker on the same volume, for comparison only ---
    let (walker_tree, walker_stats) = match walker_scan(&path) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("walker whole-volume scan failed: {e}");
            return false;
        }
    };
    let walker_entries = walker_tree.len() as u64;
    let walker_per_sec = u128::from(walker_entries) * 1000 / walker_stats.elapsed_ms.max(1);
    println!(
        "[walker] volume={} entries={walker_entries} (files={} dirs={} denied={}) elapsed_ms={} entries/s={walker_per_sec}",
        path.display(),
        walker_stats.files,
        walker_stats.dirs,
        walker_stats.denied,
        walker_stats.elapsed_ms
    );

    // Live-volume sanity: the two scans ran minutes apart and the volume
    // changed, so exact parity is impossible. Only flags a wild divergence.
    let drift = walker_entries.abs_diff(mft_entries) as f64 / walker_entries.max(1) as f64;
    let sanity_ok = drift <= 0.01;
    let speed_ok = mft_per_sec >= 333_000;
    println!(
        "BENCH-MFT mode=whole-volume volume={} walker_entries={walker_entries} walker_per_sec={walker_per_sec} \
         mft_entries={mft_entries} mft_stream_ms={stream_ms} mft_per_sec={mft_per_sec} \
         count_drift={:.3}% sanity={} speed_gate={} (target >= 333k entries/s)",
        path.display(),
        drift * 100.0,
        if sanity_ok { "ok" } else { "FAILED" },
        if speed_ok { "passed" } else { "FAILED" }
    );
    sanity_ok && speed_ok
}
