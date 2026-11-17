//! P1.T7 MFT spike — READ-ONLY feasibility probe.
//!
//! RUN BY THE OWNER in an elevated shell (reading the raw volume needs
//! administrator; the main ByteMap app never runs elevated — this is a
//! separate throwaway probe):
//!
//!     cargo run --release -p bytemap-helper --example mft_probe
//!     cargo run --release -p bytemap-helper --example mft_probe -- \\.\D:
//!
//! What it does: opens the volume READ-ONLY via `Volume::new`, streams every
//! live MFT record chunk-by-chunk (`MftScan`, like the planned helper), and
//! counts entries + logical bytes with timings. It writes nothing to the
//! volume. Report the printed numbers back (entries, bytes, entries/s).

use std::time::Instant;

use ntfs_reader::{DefaultPathCache, FileInfo, MftScan, Volume};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Win32 device path exactly as documented: no trailing backslash.
    let volume = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"\\.\C:".to_string());

    eprintln!("opening {volume} (read-only; needs an elevated shell)...");
    let open_start = Instant::now();
    let mut scan = MftScan::new(Volume::new(&volume)?)?;
    let open_elapsed = open_start.elapsed();
    eprintln!("volume opened in {open_elapsed:.1?}");

    let mut cache = DefaultPathCache::new();
    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut total_bytes = 0u128;
    let scan_start = Instant::now();
    while let Some(chunk) = scan.next_chunk()? {
        for file in chunk.files() {
            let info = FileInfo::with_cache(&file, &mut cache);
            if file.is_directory() {
                dirs += 1;
            } else {
                files += 1;
            }
            total_bytes += info.size as u128;
        }
    }
    let scan_elapsed = scan_start.elapsed();
    let entries = files + dirs;
    let speed = entries as f64 / scan_elapsed.as_secs_f64().max(f64::EPSILON);

    println!("volume:        {volume}");
    println!("entries:       {entries} (files {files}, dirs {dirs})");
    println!("logical bytes: {total_bytes}");
    println!("open:          {open_elapsed:.2?}");
    println!("scan:          {scan_elapsed:.2?}");
    println!("speed:         {speed:.0} entries/s (target to beat: 33.3k walker, gate 333k MFT)");
    Ok(())
}
