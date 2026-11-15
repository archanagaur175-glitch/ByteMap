//! Walker benchmark (P1.T5).
//!
//! Builds a synthetic 100k-file fixture ONCE into %TEMP%\bytemap-bench-100k
//! and reuses it (marker file). Run it twice in one process: first run is
//! cold-ish (metadata not cached), second is warm. For a true cold run,
//! invoke the binary twice and read the first line of each.
//!
//! Usage: cargo run --release -p bytemap-scan --bin bench-walk

use std::path::{Path, PathBuf};
use std::time::Instant;

use bytemap_core::Tree;
use bytemap_scan::{scan_folder, ScanOptions};

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join("bytemap-bench-100k")
}

fn build_if_missing(root: &Path) -> std::io::Result<()> {
    let marker = root.join(".bytemap-bench-marker");
    if marker.exists() {
        return Ok(());
    }
    println!("building fixture (one time) at {} ...", root.display());
    let start = Instant::now();
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root)?;
    // 10 * 10 * 10 leaf dirs with 100 empty files each = 100_000 files.
    for a in 0..10u32 {
        for b in 0..10u32 {
            for c in 0..10u32 {
                let dir = root
                    .join(format!("d{a:02}"))
                    .join(format!("s{b:02}"))
                    .join(format!("t{c:02}"));
                std::fs::create_dir_all(&dir)?;
                for f in 0..100u32 {
                    std::fs::File::create(dir.join(format!("f{f:03}.dat")))?;
                }
            }
        }
    }
    std::fs::write(
        &marker,
        b"100k files, 1000 leaf dirs, synthetic benchmark fixture",
    )?;
    println!("fixture built in {:?}", start.elapsed());
    Ok(())
}

fn run_once(root: &Path, label: &str, dedup: bool) -> Result<(), bytemap_scan::ScanError> {
    let mut tree = Tree::new();
    let stats = scan_folder(
        root,
        &mut tree,
        &ScanOptions {
            hardlink_dedup: dedup,
        },
    )?;
    let entries = tree.len() as u64;
    let per_sec = if stats.elapsed_ms > 0 {
        entries * 1000 / stats.elapsed_ms as u64
    } else {
        0
    };
    println!(
        "[{label}] entries={} (files={} dirs={}) denied={} elapsed_ms={} entries/s={per_sec}",
        entries, stats.files, stats.dirs, stats.denied, stats.elapsed_ms
    );
    Ok(())
}

fn main() {
    let root = fixture_root();
    if let Err(e) = build_if_missing(&root) {
        eprintln!("fixture build failed: {e}");
        std::process::exit(1);
    }
    if let Err(e) = run_once(&root, "cold ", false) {
        eprintln!("scan failed: {e}");
        std::process::exit(1);
    }
    if let Err(e) = run_once(&root, "warm ", false) {
        eprintln!("scan failed: {e}");
        std::process::exit(1);
    }
    if let Err(e) = run_once(&root, "dedup", true) {
        eprintln!("scan failed: {e}");
        std::process::exit(1);
    }
}
