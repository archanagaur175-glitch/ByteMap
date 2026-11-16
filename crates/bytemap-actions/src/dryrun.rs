//! Dry run + stale-scan guard (brief "Action Hub safety"): before any
//! action, every basket path is re-checked against the filesystem — size
//! and modified time are compared with the scan snapshot, and changed or
//! missing items are DROPPED (never acted on). The in-use status comes from
//! an injected probe (Restart Manager in production; a stub in tests), so
//! this module stays testable without FFI.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::basket::BasketItem;

/// Outcome of the re-check for one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleStatus {
    /// Exists, size (files) and mtime match the scan snapshot.
    Ok,
    /// Exists but size or mtime differs from the snapshot — must be dropped.
    Changed,
    /// No longer exists — must be dropped.
    Missing,
}

/// One dry-run report line (what the UI shows before executing).
#[derive(Debug, Clone)]
pub struct DryRunEntry {
    pub node_id: u32,
    pub path: String,
    /// Logical size at scan time.
    pub logical_scan: u64,
    /// Current size on disk for files (None for folders: re-walking a
    /// folder just for the dry run is too expensive; folders are guarded
    /// by mtime + existence).
    pub size_now: Option<u64>,
    pub mtime_scan: i64,
    pub mtime_now: Option<i64>,
    pub tier: u8,
    pub cat: u8,
    /// Restart Manager says a process holds this path open. Unknown only
    /// when the probe failed — then the entry also carries `probe_error`.
    pub in_use: bool,
    pub probe_error: Option<String>,
    pub status: StaleStatus,
}

/// Probe returns (in_use, error) — error means "could not determine".
pub type InUseProbe<'a> = &'a dyn Fn(&str) -> (bool, Option<String>);

/// The dry run. Never mutates anything. Items whose status is not `Ok`
/// must be dropped by the caller before executing any action.
pub fn dry_run(items: &[BasketItem], probe: InUseProbe) -> Vec<DryRunEntry> {
    items.iter().map(|i| check_item(i, probe)).collect()
}

fn check_item(item: &BasketItem, probe: InUseProbe) -> DryRunEntry {
    let (in_use, probe_error) = probe(&item.path);

    let (size_now, mtime_now, status) = match fs::metadata(&item.path) {
        Err(_) => (None, None, StaleStatus::Missing),
        Ok(md) => {
            let mtime_now = system_time_to_filetime(&md.modified().ok());
            let is_file = md.is_file();
            let size_now = if is_file { Some(md.len()) } else { None };
            let status =
                if (is_file && size_now != Some(item.logical)) || mtime_now != Some(item.mtime) {
                    StaleStatus::Changed
                } else {
                    StaleStatus::Ok
                };
            (size_now, mtime_now, status)
        }
    };

    DryRunEntry {
        node_id: item.node_id,
        path: item.path.clone(),
        logical_scan: item.logical,
        size_now,
        mtime_scan: item.mtime,
        mtime_now,
        tier: item.tier,
        cat: item.cat,
        in_use,
        probe_error,
        status,
    }
}

/// SystemTime -> Windows FILETIME (100 ns ticks since 1601-01-01).
/// Used everywhere mtime is compared; None (unavailable mtime) maps to None.
pub fn system_time_to_filetime(t: &Option<SystemTime>) -> Option<i64> {
    let d = t.as_ref()?.duration_since(UNIX_EPOCH).ok()?;
    // 11,644,473,600,000,000 = ticks between 1601-01-01 and 1970-01-01.
    const EPOCH_DIFF_100NS: i64 = 11_644_473_600_000_000;
    let ticks = d.as_secs() as i64 * 10_000_000 + (d.subsec_nanos() as i64) / 100;
    Some(ticks + EPOCH_DIFF_100NS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier;

    fn item(id: u32, path: &str, logical: u64, mtime: i64) -> BasketItem {
        BasketItem {
            node_id: id,
            path: path.to_string(),
            name: "n".to_string(),
            logical,
            on_disk: logical,
            mtime,
            tier: tier::SAFE,
            cat: 6,
        }
    }

    fn free_probe(_p: &str) -> (bool, Option<String>) {
        (false, None)
    }
    fn used_probe(_p: &str) -> (bool, Option<String>) {
        (true, None)
    }

    #[test]
    fn unchanged_file_is_ok() {
        let dir = std::env::temp_dir().join("bytemap-dryrun-ok");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        fs::write(&p, b"12345").unwrap();
        let md = fs::metadata(&p).unwrap();
        let mtime = system_time_to_filetime(&md.modified().ok()).unwrap();

        let entries = dry_run(&[item(1, p.to_str().unwrap(), 5, mtime)], &free_probe);
        assert_eq!(entries[0].status, StaleStatus::Ok);
        assert_eq!(entries[0].size_now, Some(5));
        assert!(!entries[0].in_use);
        assert!(entries[0].probe_error.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn size_change_is_caught() {
        let dir = std::env::temp_dir().join("bytemap-dryrun-size");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("b.txt");
        fs::write(&p, b"12345").unwrap();
        let md = fs::metadata(&p).unwrap();
        let mtime = system_time_to_filetime(&md.modified().ok()).unwrap();

        // Snapshot says 100 bytes; reality is 5 -> Changed.
        let entries = dry_run(&[item(1, p.to_str().unwrap(), 100, mtime)], &free_probe);
        assert_eq!(entries[0].status, StaleStatus::Changed);

        fs::write(&p, b"12345").unwrap(); // keep mtime; restore size
                                          // Stale mtime (scan says 0, file is current) -> Changed.
        let entries = dry_run(&[item(1, p.to_str().unwrap(), 5, 0)], &free_probe);
        assert_eq!(entries[0].status, StaleStatus::Changed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_caught() {
        let entries = dry_run(
            &[item(1, r"C:\bytemap-does-not-exist-at-all", 1, 0)],
            &free_probe,
        );
        assert_eq!(entries[0].status, StaleStatus::Missing);
        assert_eq!(entries[0].size_now, None);
    }

    #[test]
    fn folder_guard_uses_mtime_and_existence() {
        let dir = std::env::temp_dir().join("bytemap-dryrun-dir");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let md = fs::metadata(&dir).unwrap();
        let mtime = system_time_to_filetime(&md.modified().ok()).unwrap();

        // Folders: size is not re-checked (None), mtime decides.
        let entries = dry_run(&[item(1, dir.to_str().unwrap(), 0, mtime)], &free_probe);
        assert_eq!(entries[0].status, StaleStatus::Ok);
        assert_eq!(entries[0].size_now, None);

        let entries = dry_run(&[item(1, dir.to_str().unwrap(), 0, 0)], &free_probe);
        assert_eq!(entries[0].status, StaleStatus::Changed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_use_is_reported_from_probe() {
        let dir = std::env::temp_dir().join("bytemap-dryrun-use");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.txt");
        fs::write(&p, b"x").unwrap();
        let md = fs::metadata(&p).unwrap();
        let mtime = system_time_to_filetime(&md.modified().ok()).unwrap();

        let entries = dry_run(&[item(1, p.to_str().unwrap(), 1, mtime)], &used_probe);
        assert!(entries[0].in_use);

        // Probe failure is surfaced, not silently "free".
        let failing = |_p: &str| (false, Some("rm down".to_string()));
        let entries = dry_run(&[item(1, p.to_str().unwrap(), 1, mtime)], &failing);
        assert!(!entries[0].in_use);
        assert_eq!(entries[0].probe_error.as_deref(), Some("rm down"));
        let _ = fs::remove_dir_all(&dir);
    }
}
