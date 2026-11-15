//! Windows metadata helpers: attribute flags, cluster size, size on disk.
//!
//! Rules from PROJECT_BRIEF / DECISIONS.md:
//! - Reparse points (0x400) are flagged, never followed.
//! - OneDrive placeholders (0x400000 recall-on-data-access, 0x40000
//!   recall-on-open) are flagged, never hydrated (we only read metadata).
//! - Size on disk defaults to logical size rounded up to the volume cluster
//!   size (cluster size queried once per volume). GetCompressedFileSizeW is
//!   called only for files with the compressed (0x800) or sparse (0x200)
//!   attribute — no per-file syscall otherwise.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Storage::FileSystem::{
    GetCompressedFileSizeW, GetDiskFreeSpaceW, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400_0000;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// Cluster size in bytes for the volume containing `path`.
/// Returns 4096 as a safe fallback if the query fails.
pub fn cluster_size(path: &Path) -> u32 {
    // GetDiskFreeSpaceW wants the volume root, e.g. "C:\".
    let mut prefix: Option<String> = None;
    let mut comps = path.components();
    if let Some(std::path::Component::Prefix(p)) = comps.next() {
        let s = p.as_os_str().to_string_lossy().to_string();
        // "\\?\C:" style verbatim prefixes -> "C:\"
        let trimmed = s.trim_start_matches(r"\\?\");
        prefix = Some(format!("{trimmed}\\"));
    }
    let Some(root) = prefix else { return 4096 };
    let root_w = wide(Path::new(&root));
    let (mut sectors, mut bytes, mut free, mut total): (u32, u32, u32, u32) = (0, 0, 0, 0);
    let ok = unsafe {
        GetDiskFreeSpaceW(
            root_w.as_ptr(),
            &mut sectors,
            &mut bytes,
            &mut free,
            &mut total,
        )
    };
    if ok != 0 && sectors > 0 && bytes > 0 {
        sectors.saturating_mul(bytes)
    } else {
        4096
    }
}

/// Size on disk: cluster-rounded by default; exact query for
/// compressed/sparse files. `logical` is the file's logical size.
pub fn size_on_disk(path: &Path, logical: u64, attrs: u32, cluster: u32) -> u64 {
    let rounded = if logical == 0 {
        0
    } else {
        logical.div_ceil(cluster as u64) * cluster as u64
    };
    let compressed_or_sparse =
        attrs & FILE_ATTRIBUTE_COMPRESSED != 0 || attrs & FILE_ATTRIBUTE_SPARSE_FILE != 0;
    if !compressed_or_sparse {
        return rounded;
    }
    let w = wide(path);
    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
    // INVALID_FILE_SIZE (0xFFFFFFFF) with GetLastError == NO_ERROR is legal
    // for files > 4 GiB only when high is set; any other failure falls back.
    if low == 0xFFFF_FFFF && high == 0 {
        return rounded;
    }
    ((high as u64) << 32) | low as u64
}

/// Windows FILETIME (100 ns ticks since 1601-01-01) from a SystemTime.
pub fn filetime(t: SystemTime) -> i64 {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000; // 100 ns ticks between 1601 and 1970
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let ticks = (d.as_nanos() / 100) as i64;
            EPOCH_DIFF.saturating_add(ticks)
        }
        Err(_) => 0,
    }
}

/// NTFS file identity: (volume serial number, file index). Requires opening
/// a handle per file, so callers must gate this behind an option (P1.T3b).
/// Returns None for reparse points, placeholders, and any failure.
pub fn file_id(path: &Path, attrs: u32) -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_READ_ATTRIBUTES, BY_HANDLE_FILE_INFORMATION,
    };

    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || attrs & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
    {
        return None;
    }
    let w = wide(path);
    let handle = unsafe {
        CreateFileW(
            w.as_ptr(),
            FILE_READ_ATTRIBUTES,
            // Share everything: we only peek at attributes.
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            3, // OPEN_EXISTING
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return None;
    }
    let index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
    Some((info.dwVolumeSerialNumber as u64, index))
}

/// Map raw Windows attributes to ByteMap node flags.
/// Returns (node_flags, is_dir).
pub fn classify(attrs: u32) -> (u16, bool) {
    use bytemap_core::tree::flags;
    let mut f = 0u16;
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        f |= flags::REPARSE;
    }
    if attrs & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 {
        f |= flags::PLACEHOLDER;
    }
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    (f, is_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_epoch_math() {
        // 2020-01-01T00:00:00Z == 132228384000000000 FILETIME ticks.
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_578_364_800);
        assert_eq!(filetime(t), 132_228_384_000_000_000);
    }

    #[test]
    fn classify_flags() {
        let (f, dir) = classify(FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT);
        assert!(dir);
        assert_eq!(f & bytemap_core::tree::flags::REPARSE, bytemap_core::tree::flags::REPARSE);
        let (f, dir) = classify(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS);
        assert!(!dir);
        assert_eq!(f & bytemap_core::tree::flags::PLACEHOLDER, bytemap_core::tree::flags::PLACEHOLDER);
    }

    #[test]
    fn cluster_size_falls_back_on_missing_path() {
        assert!(cluster_size(Path::new(r"\\?\Z:\definitely-not-here")) > 0);
    }
}
