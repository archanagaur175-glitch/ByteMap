//! Tier B scanner: parallel walker (jwalk + rayon), works without admin.

use std::collections::HashMap;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytemap_core::tree::{flags, NodeId, Tree};
use jwalk::WalkDirGeneric;

use crate::error::ScanError;
use crate::metadata as m;

/// Per-entry payload computed on rayon threads inside `process_read_dir`,
/// so the expensive metadata calls run in parallel.
#[derive(Clone, Copy, Debug, Default)]
struct EntryInfo {
    is_dir: bool,
    node_flags: u16,
    logical: u64,
    on_disk: u64,
    mtime: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Deduplicate hardlinks by file ID. When set, the second and later
    /// names of a hardlinked file are flagged `DUPLICATE_HARDLINK` and
    /// contribute no size. Costs one handle-open per file (benchmarked in
    /// P1.T3b). Default is OFF (owner decision 2026-11-17: 82.0k entries/s
    /// without dedup vs 38.0k with; MFT tier dedups by file ID regardless).
    pub hardlink_dedup: bool,
    /// Cooperative cancel flag, checked between entries. When triggered the
    /// scan stops early and returns [`ScanError::Cancelled`].
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Totals from one scan.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanStats {
    pub dirs: u64,
    pub files: u64,
    /// Directories or files whose enumeration/metadata was denied.
    pub denied: u64,
    pub elapsed_ms: u128,
}

/// Walk `root` in parallel and build `tree` (owned by Rust, never sent to UI).
///
/// Guarantees:
/// - junctions/symlinks/reparse points are recorded but never followed;
/// - OneDrive placeholders are recorded but never hydrated;
/// - access-denied entries are counted in `ScanStats::denied`, not fatal;
/// - long paths work (the root is canonicalized to a `\\?\`-style path);
/// - sizes: logical + size on disk;
/// - `options.cancel` stops the scan early with `ScanError::Cancelled`;
/// - `on_progress` (if given) is called from the consumer loop with running
///   counters; the CALLER throttles it (UI batches at <= 20/s upstream).
#[derive(Debug, Clone, Copy)]
pub struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub elapsed_ms: u128,
}

pub fn scan_folder(
    root: &Path,
    tree: &mut Tree,
    options: &ScanOptions,
    on_progress: Option<&dyn Fn(ScanProgress)>,
) -> Result<ScanStats, ScanError> {
    let start = Instant::now();
    // canonicalize() returns a \\?\-style path on Windows: long paths work.
    let canonical = std::fs::canonicalize(root).map_err(|e| ScanError::Root(root.to_owned(), e))?;
    let cluster = m::cluster_size(&canonical);
    let denied = Arc::new(AtomicU64::new(0));
    let denied_cb = Arc::clone(&denied);
    // Hardlink dedup state, shared across rayon threads (only touched when
    // the option is on). Key: (volume serial, file index).
    let seen_files: Arc<Mutex<HashMap<(u64, u64), ()>>> = Arc::new(Mutex::new(HashMap::new()));
    let dedup = options.hardlink_dedup;
    let seen_files_cb = Arc::clone(&seen_files);

    let walk = WalkDirGeneric::<((), EntryInfo)>::new(&canonical)
        .skip_hidden(false) // a disk analyzer must see hidden/system files
        .process_read_dir(move |_depth, _path, _state, children| {
            children.retain_mut(|child| {
                let entry = match child {
                    Err(_) => {
                        denied_cb.fetch_add(1, Ordering::Relaxed);
                        return false;
                    }
                    Ok(entry) => entry,
                };
                // symlink_metadata semantics: never follows the reparse point.
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        denied_cb.fetch_add(1, Ordering::Relaxed);
                        return false;
                    }
                };
                let attrs = meta.file_attributes();
                let (node_flags, is_dir) = m::classify(attrs);
                if attrs & m::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    // Belt and braces: never descend into junctions/symlinks.
                    entry.read_children = None;
                }
                let path = entry.path();
                let logical = if is_dir { 0 } else { meta.len() };
                let on_disk = if is_dir {
                    0
                } else {
                    m::size_on_disk(&path, logical, attrs, cluster)
                };
                let mtime = meta.modified().map(m::filetime).unwrap_or(0);
                if !is_dir && dedup && node_flags & flags::REPARSE == 0 {
                    // Dedup costs one handle-open per file (P1.T3b).
                    if let Some(id) = m::file_id(&path, attrs) {
                        let mut seen = seen_files_cb.lock().unwrap();
                        if seen.insert(id, ()).is_some() {
                            drop(seen);
                            // Later names of the same file: flag them and
                            // contribute no size (counted once by file ID).
                            entry.client_state = EntryInfo {
                                is_dir,
                                node_flags: node_flags | flags::DUPLICATE_HARDLINK,
                                logical: 0,
                                on_disk: 0,
                                mtime,
                            };
                            return true;
                        }
                    }
                }
                entry.client_state = EntryInfo {
                    is_dir,
                    node_flags,
                    logical,
                    on_disk,
                    mtime,
                };
                true
            });
        });

    let mut dir_map: HashMap<String, NodeId> = HashMap::new();
    let mut stats = ScanStats::default();

    for entry in walk {
        if let Some(cancel) = &options.cancel {
            if cancel.load(Ordering::Relaxed) {
                return Err(ScanError::Cancelled);
            }
        }
        let entry = entry?;
        // A failed read_dir is attached to the PARENT entry's read_children
        // (jwalk still yields the parent as Ok), so check it explicitly.
        if entry
            .read_children
            .as_ref()
            .map(|rc| rc.error().is_some())
            .unwrap_or(false)
        {
            stats.denied += 1;
        }
        let info = entry.client_state;
        let name = entry.file_name().to_string_lossy().into_owned();

        if entry.depth() == 0 {
            // The root itself; its name is the last path component.
            let root_name = entry
                .path()
                .components()
                .next_back()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
            let root_id = tree.insert_root(&root_name);
            dir_map.insert(canonical.to_string_lossy().into_owned(), root_id);
            stats.dirs += 1;
            continue;
        }

        let parent_key = entry.parent_path().to_string_lossy().into_owned();
        let parent = dir_map
            .get(&parent_key)
            .copied()
            .ok_or_else(|| ScanError::MissingParent(entry.path()))?;

        if info.is_dir {
            let id = tree.insert_child(
                parent,
                &name,
                info.node_flags | flags::DIR,
                info.mtime,
                0,
                0,
            );
            dir_map.insert(entry.path().to_string_lossy().into_owned(), id);
            stats.dirs += 1;
        } else {
            tree.insert_child(
                parent,
                &name,
                info.node_flags,
                info.mtime,
                info.logical,
                info.on_disk,
            );
            stats.files += 1;
        }
        if let Some(on_progress) = on_progress {
            on_progress(ScanProgress {
                files: stats.files,
                dirs: stats.dirs,
                denied: stats.denied,
                elapsed_ms: start.elapsed().as_millis(),
            });
        }
    }

    // Err children removed in process_read_dir + read_dir-level failures
    // (attached by jwalk to the parent's read_children, counted in the loop).
    stats.denied += denied.load(Ordering::Relaxed);
    stats.elapsed_ms = start.elapsed().as_millis();
    bytemap_core::recompute_rollups(tree);
    Ok(stats)
}
