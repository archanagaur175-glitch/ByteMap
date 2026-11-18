//! MFT fast-path benchmark + parity gate (P6.T4c).
//!
//! Scans the SAME fixture — %TEMP%\bytemap-bench-100k — through BOTH tiers
//! and compares:
//! 1. Walker (Tier B): jwalk + rayon, no elevation needed.
//! 2. MFT (Tier A): the elevated helper over the BYM1 pipe. Needs the
//!    helper running; this bin starts one with `runas` (UAC prompt) when
//!    none is up. RUN BY THE OWNER (the UAC prompt must be accepted):
//!
//!     cargo run --release -p bytemap-scan --bin bench-mft
//!
//! Parity gate: identical file/dir/denied counts and identical total
//! logical bytes on the root aggregate. Speed gate: MFT >= 333k entries/s
//! (the brief's >= 10x walker target, calibrated by the P1.T7 spike).

use std::time::Instant;

use bytemap_core::Tree;
use bytemap_scan::mft_client::{MftClient, WireFrame, DEFAULT_PIPE_NAME};
use bytemap_scan::mft_tree::build_tree_from_records;
use bytemap_scan::spawn::ensure_helper;
use bytemap_scan::{scan_folder, ScanOptions, ScanStats};

fn fixture_root() -> std::path::PathBuf {
    std::env::temp_dir().join("bytemap-bench-100k")
}

/// Tier B: reference scan.
fn walker_scan(root: &std::path::Path) -> Result<(Tree, ScanStats), String> {
    let mut tree = Tree::new();
    let stats =
        scan_folder(root, &mut tree, &ScanOptions::default(), None).map_err(|e| e.to_string())?;
    Ok((tree, stats))
}

/// Tier A: stream the helper's records and build the tree, timing the
/// stream phase (the MFT gate metric) separately from the tree build.
/// Returns (tree, stats, stream_ms, wire logical bytes of files).
fn mft_scan(root: &std::path::Path) -> Result<(Tree, ScanStats, u128, u64), String> {
    let stream_start = Instant::now();
    let mut client = MftClient::connect(DEFAULT_PIPE_NAME).map_err(|e| e.to_string())?;
    client
        .request_scan(&root.to_string_lossy())
        .map_err(|e| e.to_string())?;

    let mut records: u64 = 0;
    let mut logical: u64 = 0;
    let mut failure: Option<String> = None;
    let stream = std::iter::from_fn(|| {
        if failure.is_some() {
            return None;
        }
        match client.next_frame() {
            Ok(WireFrame::Record(record)) => {
                records += 1;
                if record.attributes & 0x10 == 0 {
                    logical += record.logical_size;
                }
                Some(record)
            }
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
    let stream_ms = stream_start.elapsed().as_millis();
    Ok((tree, stats, stream_ms, logical))
}

fn main() {
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
            std::process::exit(1);
        }
    };
    let walker_entries = walker_tree.len() as u64;
    let walker_per_sec = u128::from(walker_entries) * 1000 / walker_stats.elapsed_ms.max(1);
    println!(
        "[walker] entries={walker_entries} (files={} dirs={} denied={}) elapsed_ms={} entries/s={walker_per_sec}",
        walker_stats.files, walker_stats.dirs, walker_stats.denied, walker_stats.elapsed_ms
    );

    // --- Tier A: helper up (spawn via UAC when absent), MFT scan ---
    match ensure_helper() {
        Ok(status) => println!("[helper] {status:?}"),
        Err(e) => {
            eprintln!(
                "SKIPPED LOUDLY: helper not available: {e}\n\
                 Build and start it (elevated) with:\n  \
                 cargo build --release -p bytemap-helper\n  \
                 cargo run --release -p bytemap-helper\n\
                 ...or accept the UAC prompt this bin triggers when the exe exists."
            );
            std::process::exit(2);
        }
    }
    let (mft_tree, mft_stats, stream_ms, _logical_wire) = match mft_scan(&root) {
        Ok(tuple) => tuple,
        Err(e) => {
            eprintln!("mft scan failed: {e}");
            std::process::exit(1);
        }
    };
    let mft_entries = mft_tree.len() as u64;
    let mft_per_sec = u128::from(mft_entries) * 1000 / stream_ms.max(1);
    println!(
        "[mft] entries={mft_entries} (files={} dirs={} denied={}) stream_ms={stream_ms} entries/s={mft_per_sec}",
        mft_stats.files, mft_stats.dirs, mft_stats.denied
    );

    // --- Parity gate ---
    let mut verdict = String::from("ok");
    if walker_stats.files != mft_stats.files
        || walker_stats.dirs != mft_stats.dirs
        || walker_stats.denied != mft_stats.denied
        || walker_tree
            .root()
            .map(|r| walker_tree.node(r).agg_logical())
            != mft_tree.root().map(|r| mft_tree.node(r).agg_logical())
    {
        verdict = "PARITY FAILED".to_owned();
        eprintln!(
            "parity failed: walker files={} dirs={} denied={} logical={:?}; mft files={} dirs={} denied={} logical={:?}",
            walker_stats.files,
            walker_stats.dirs,
            walker_stats.denied,
            walker_tree.root().map(|r| walker_tree.node(r).agg_logical()),
            mft_stats.files,
            mft_stats.dirs,
            mft_stats.denied,
            mft_tree.root().map(|r| mft_tree.node(r).agg_logical())
        );
    }
    let speed_gate = mft_per_sec >= 333_000;
    println!(
        "BENCH-MFT walker_entries={walker_entries} walker_per_sec={walker_per_sec} mft_entries={mft_entries} mft_stream_ms={stream_ms} mft_per_sec={mft_per_sec} parity={verdict} speed_gate={} (target >= 333k entries/s)",
        if speed_gate { "passed" } else { "FAILED" }
    );
    if verdict != "ok" || !speed_gate {
        std::process::exit(1);
    }
}
