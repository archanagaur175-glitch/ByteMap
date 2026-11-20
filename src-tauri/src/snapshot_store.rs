//! Snapshot storage (Phase 8): one file per saved scan under
//! `%LOCALAPPDATA%\ByteMap\snapshots`, plus the four commands the UI calls.
//!
//! Files are written next to their final name (`<name>.part`), flushed, then
//! renamed, so a crash or a full disk never leaves a half-written snapshot
//! that the list would show as valid. The listing reads only the bounded
//! header prefix ([`MAX_HEADER_LEN`]), never the compressed body.
//!
//! Loading a snapshot replaces the session tree; snapshot node ids are only
//! valid for that snapshot, so the basket is cleared exactly like a new scan.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bytemap_core::categorizer::CategoryMap;
use bytemap_core::snapshot::{
    load_snapshot, read_header, save_snapshot_parts, tree_counts, ScanTier, SnapshotHeader,
    SnapshotMeta, MAX_HEADER_LEN,
};
use bytemap_core::Tree;
use serde::Serialize;
use serde_json::json;
use tauri::State;

use crate::state::SharedSession;

/// Snapshot file extension.
pub const SNAPSHOT_EXT: &str = "bms";

/// One row of the snapshot list (header + file size; no decompression).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    pub name: String,
    /// File size on disk in bytes.
    pub bytes: u64,
    pub created_unix: u64,
    pub root_name: String,
    /// "walker" or "mft".
    pub tier: String,
    pub scan_ms: u64,
    pub nodes: u64,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub total_logical: u64,
    pub total_on_disk: u64,
    /// Set when the file exists but its header cannot be parsed. The row is
    /// still listed so the user can delete the broken file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Snapshot folder: `%LOCALAPPDATA%\ByteMap\snapshots` (local-only product).
/// `BYTEMAP_SNAPSHOT_DIR` overrides it for manual testing.
pub fn snapshot_dir() -> PathBuf {
    std::env::var_os("BYTEMAP_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("ByteMap")
                .join("snapshots")
        })
}

/// A snapshot name is a bare file name: no separators, no traversal.
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("invalid snapshot name".to_string());
    }
    if !name.ends_with(&format!(".{SNAPSHOT_EXT}")) {
        return Err(format!("snapshot names end with .{SNAPSHOT_EXT}"));
    }
    if name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("invalid snapshot name".to_string());
    }
    Ok(())
}

fn path_for(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    Ok(dir.join(name))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn entry_from_header(name: String, bytes: u64, header: SnapshotHeader) -> SnapshotEntry {
    SnapshotEntry {
        name,
        bytes,
        created_unix: header.meta.created_unix,
        root_name: header.meta.root_name,
        tier: header.meta.tier.as_str().to_string(),
        scan_ms: header.meta.scan_ms,
        nodes: header.counts.nodes,
        files: header.counts.files,
        dirs: header.counts.dirs,
        denied: header.counts.denied,
        total_logical: header.counts.total_logical,
        total_on_disk: header.counts.total_on_disk,
        error: None,
    }
}

/// Read a file's header prefix and turn it into a list row. Never fails:
/// an unreadable/corrupt file becomes a row with `error` set.
fn read_entry(dir: &Path, name: &str) -> SnapshotEntry {
    let path = dir.join(name);
    let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let parsed = (|| -> Result<SnapshotHeader, String> {
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(MAX_HEADER_LEN as u64)
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        read_header(&buf).map_err(|e| e.to_string())
    })();

    match parsed {
        Ok(header) => entry_from_header(name.to_string(), bytes, header),
        Err(error) => SnapshotEntry {
            name: name.to_string(),
            bytes,
            created_unix: 0,
            root_name: String::new(),
            tier: String::new(),
            scan_ms: 0,
            nodes: 0,
            files: 0,
            dirs: 0,
            denied: 0,
            total_logical: 0,
            total_on_disk: 0,
            error: Some(error),
        },
    }
}

/// List every `.bms` file in `dir`, newest first. A missing folder is an
/// empty list, not an error (nothing has been saved yet).
pub fn list_in(dir: &Path) -> Vec<SnapshotEntry> {
    let mut entries: Vec<SnapshotEntry> = Vec::new();
    let Ok(read) = fs::read_dir(dir) else {
        return entries;
    };
    for item in read.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(&format!(".{SNAPSHOT_EXT}")) {
            continue;
        }
        entries.push(read_entry(dir, name));
    }
    entries.sort_by(|a, b| {
        b.created_unix
            .cmp(&a.created_unix)
            .then_with(|| b.name.cmp(&a.name))
    });
    entries
}

/// Pick the next free `bytemap-<created>.bms` name (same-second saves get
/// `-2`, `-3`, ... rather than overwriting).
fn unique_path(dir: &Path, created_unix: u64) -> Result<(String, PathBuf), String> {
    let base = format!("bytemap-{created_unix}");
    for n in 1..=9999 {
        let name = if n == 1 {
            format!("{base}.{SNAPSHOT_EXT}")
        } else {
            format!("{base}-{n}.{SNAPSHOT_EXT}")
        };
        let path = dir.join(&name);
        if !path.exists() {
            return Ok((name, path));
        }
    }
    Err("too many snapshots saved in the same second".to_string())
}

/// Encode and write one snapshot. Returns the list row for the new file.
pub fn save_in(
    dir: &Path,
    tree: &Tree,
    cats: &CategoryMap,
    meta: &SnapshotMeta,
) -> Result<SnapshotEntry, String> {
    let bytes = save_snapshot_parts(tree, cats, meta).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let (name, path) = unique_path(dir, meta.created_unix)?;

    // Write beside the final name, flush, then rename: a failed save never
    // leaves a file that looks like a valid snapshot.
    let tmp = path.with_extension("part");
    {
        let mut file = fs::File::create(&tmp).map_err(|e| format!("create {name}: {e}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("write {name}: {e}"))?;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("finish {name}: {e}"));
    }

    let header = read_header(&bytes).map_err(|e| e.to_string())?;
    Ok(entry_from_header(name, bytes.len() as u64, header))
}

/// Load and parse one snapshot by name.
pub fn load_in(dir: &Path, name: &str) -> Result<bytemap_core::Snapshot, String> {
    let path = path_for(dir, name)?;
    let bytes = fs::read(&path).map_err(|e| format!("read {name}: {e}"))?;
    load_snapshot(&bytes).map_err(|e| format!("{name}: {e}"))
}

/// Delete one snapshot. Deleting a file that is already gone is not an
/// error (the listing may be stale by one click).
pub fn delete_in(dir: &Path, name: &str) -> Result<(), String> {
    let path = path_for(dir, name)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("delete {name}: {e}")),
    }
}

// --- Commands (registered in lib.rs; generate_handler lives there) ---

/// Save the current tree as a new snapshot. Refused while a scan is running
/// and when there is nothing loaded.
#[tauri::command]
pub fn snapshot_save(session: State<'_, SharedSession>) -> Result<SnapshotEntry, String> {
    let shared = session.inner();
    let s = shared.lock().unwrap();
    if s.running {
        return Err("a scan is running; finish or cancel it before saving".to_string());
    }
    if s.tree.is_empty() || s.root_path.as_os_str().is_empty() {
        return Err("nothing to save: scan a folder or load a snapshot first".to_string());
    }
    let meta = SnapshotMeta {
        created_unix: now_unix(),
        scan_ms: s.scan_ms,
        tier: s.scan_tier.unwrap_or(ScanTier::Walker),
        root_name: s.root_path.to_string_lossy().into_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    save_in(&snapshot_dir(), &s.tree, &s.cats, &meta)
}

/// Every saved snapshot, newest first (header only, no decompression).
#[tauri::command]
pub fn snapshot_list() -> Result<Vec<SnapshotEntry>, String> {
    Ok(list_in(&snapshot_dir()))
}

/// Load a snapshot into the session. Node ids become snapshot-local: the
/// basket is cleared, and the UI shows a snapshot banner.
#[tauri::command]
pub fn snapshot_load(
    name: String,
    session: State<'_, SharedSession>,
) -> Result<serde_json::Value, String> {
    let snapshot = load_in(&snapshot_dir(), &name)?;
    apply_snapshot(session.inner(), &name, snapshot)
}

/// Swap a parsed snapshot into the session (testable core). Refused while a
/// scan is in flight, because the running job owns the tree.
pub fn apply_snapshot(
    shared: &SharedSession,
    name: &str,
    snapshot: bytemap_core::Snapshot,
) -> Result<serde_json::Value, String> {
    let root = snapshot
        .tree
        .root()
        .ok_or_else(|| format!("{name}: snapshot has no root node"))?;
    let counts = tree_counts(&snapshot.tree);
    let meta = snapshot.meta;

    let mut s = shared.lock().unwrap();
    if s.running {
        return Err("a scan is running; finish or cancel it before loading".to_string());
    }
    s.root_path = PathBuf::from(&meta.root_name);
    s.scan_tier = Some(meta.tier);
    s.scan_ms = meta.scan_ms;
    s.tree = snapshot.tree;
    s.cats = snapshot.cats;
    s.basket.clear();
    s.loaded_snapshot = Some(name.to_string());

    Ok(json!({
        "rootId": root,
        "name": name,
        "rootName": meta.root_name,
        "createdUnix": meta.created_unix,
        "tier": meta.tier.as_str(),
        "scanMs": meta.scan_ms,
        "counts": {
            "nodes": counts.nodes,
            "files": counts.files,
            "dirs": counts.dirs,
            "denied": counts.denied,
            "totalLogical": counts.total_logical,
            "totalOnDisk": counts.total_on_disk,
        },
    }))
}

/// Delete a snapshot by name.
#[tauri::command]
pub fn snapshot_delete(name: String) -> Result<(), String> {
    delete_in(&snapshot_dir(), &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemap_core::categorizer::{categorize, Rules};
    use bytemap_core::rollup::recompute_rollups;
    use bytemap_core::tree::flags;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::state::{Session, SharedSession};
    use bytemap_actions::{Basket, BasketItem};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Fresh fixture folder per test (never the real app-data folder).
    fn temp_dir(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("bytemap-snap-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build() -> (Tree, CategoryMap) {
        let mut tree = Tree::new();
        let root = tree.insert_root("C:\\fixture");
        tree.insert_child(root, "docs", flags::DIR, 0, 0, 0);
        tree.insert_child(root, "a.txt", 0, 7, 100, 100);
        tree.insert_child(root, "b.bin", 0, 0, 50, 50);
        recompute_rollups(&mut tree);
        let cats = categorize(&tree, &Rules::default_rules());
        (tree, cats)
    }

    fn meta(created_unix: u64) -> SnapshotMeta {
        SnapshotMeta {
            created_unix,
            scan_ms: 42,
            tier: ScanTier::Walker,
            root_name: "C:\\fixture".to_string(),
            app_version: "test".to_string(),
        }
    }

    #[test]
    fn save_list_load_delete_roundtrip() {
        let dir = temp_dir("roundtrip");
        let (tree, cats) = build();

        let entry = save_in(&dir, &tree, &cats, &meta(1_700_000_000)).unwrap();
        assert_eq!(entry.name, "bytemap-1700000000.bms");
        assert!(entry.bytes > 0);
        assert_eq!(entry.nodes, 4);
        assert_eq!(entry.files, 2);
        assert_eq!(entry.dirs, 2); // root + docs
        assert_eq!(entry.total_logical, 150);
        assert_eq!(entry.root_name, "C:\\fixture");
        assert_eq!(entry.tier, "walker");
        assert!(entry.error.is_none());

        let list = list_in(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, entry.name);

        let snapshot = load_in(&dir, &entry.name).unwrap();
        assert_eq!(snapshot.tree.len(), 4);
        assert_eq!(snapshot.tree.name(0), "C:\\fixture");
        assert_eq!(snapshot.meta.created_unix, 1_700_000_000);

        delete_in(&dir, &entry.name).unwrap();
        assert!(list_in(&dir).is_empty());
        // Idempotent: deleting twice is fine.
        delete_in(&dir, &entry.name).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_second_saves_get_distinct_names() {
        let dir = temp_dir("unique");
        let (tree, cats) = build();
        let a = save_in(&dir, &tree, &cats, &meta(1_700_000_001)).unwrap();
        let b = save_in(&dir, &tree, &cats, &meta(1_700_000_001)).unwrap();
        assert_ne!(a.name, b.name);
        assert_eq!(list_in(&dir).len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_cannot_escape_the_snapshot_folder() {
        let dir = temp_dir("traversal");
        for bad in [
            "",
            "..\\evil.bms",
            "../evil.bms",
            "sub/evil.bms",
            "evil.txt",
            ".hidden",
            "a b.bms",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} must be refused");
            assert!(delete_in(&dir, bad).is_err(), "{bad:?} must be refused");
        }
        assert!(validate_name("bytemap-1700000000.bms").is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_files_are_listed_so_they_can_be_deleted() {
        let dir = temp_dir("corrupt");
        fs::write(dir.join("broken.bms"), b"not a snapshot").unwrap();
        let list = list_in(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "broken.bms");
        assert!(list[0].error.is_some());
        assert!(load_in(&dir, "broken.bms").is_err());

        delete_in(&dir, "broken.bms").unwrap();
        assert!(list_in(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_ignores_other_files_and_missing_folder() {
        let dir = temp_dir("filter");
        fs::write(dir.join("notes.txt"), b"hi").unwrap();
        fs::write(dir.join("snapshot.bms.part"), b"tmp").unwrap();
        assert!(list_in(&dir).is_empty());
        assert!(list_in(&dir.join("does-not-exist")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A live session that differs from the saved snapshot in every field the
    /// swap must overwrite.
    fn live_session() -> SharedSession {
        let mut tree = Tree::new();
        let root = tree.insert_root("D:\\live");
        tree.insert_child(root, "keep.tmp", 0, 0, 1, 1);
        recompute_rollups(&mut tree);
        let len = tree.len();
        let mut session = Session {
            tree,
            cats: CategoryMap {
                cat: vec![0; len],
                tier: vec![0; len],
            },
            root_path: PathBuf::from("D:\\live"),
            basket: Basket::new(),
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            scan_tier: Some(ScanTier::Mft),
            scan_ms: 9_999,
            loaded_snapshot: None,
        };
        session
            .basket
            .add(BasketItem {
                node_id: 1,
                path: "D:\\live\\keep.tmp".to_string(),
                name: "keep.tmp".to_string(),
                logical: 1,
                on_disk: 1,
                mtime: 0,
                tier: bytemap_actions::tier::SAFE,
                cat: 0,
            })
            .unwrap();
        Arc::new(Mutex::new(session))
    }

    #[test]
    fn apply_snapshot_swaps_the_session_and_clears_the_basket() {
        let dir = temp_dir("apply");
        let (tree, cats) = build();
        let entry = save_in(&dir, &tree, &cats, &meta(1_700_000_300)).unwrap();
        let snapshot = load_in(&dir, &entry.name).unwrap();
        let shared = live_session();

        let value = apply_snapshot(&shared, &entry.name, snapshot).unwrap();
        assert_eq!(value["rootId"], 0);
        assert_eq!(value["name"], entry.name.as_str());
        assert_eq!(value["rootName"], "C:\\fixture");
        assert_eq!(value["tier"], "walker");
        assert_eq!(value["scanMs"], 42);
        assert_eq!(value["counts"]["nodes"], 4);
        assert_eq!(value["counts"]["totalLogical"], 150);

        let s = shared.lock().unwrap();
        assert_eq!(s.tree.len(), 4);
        assert_eq!(s.tree.name(0), "C:\\fixture");
        assert_eq!(s.cats.cat.len(), 4);
        assert_eq!(s.cats.tier.len(), 4);
        assert_eq!(s.root_path, PathBuf::from("C:\\fixture"));
        // Snapshot node ids are meaningless for the old basket.
        assert!(s.basket.is_empty());
        assert_eq!(s.loaded_snapshot.as_deref(), Some(entry.name.as_str()));
        assert_eq!(s.scan_tier, Some(ScanTier::Walker));
        assert_eq!(s.scan_ms, 42);
        drop(s);

        // The swapped tree is usable through the normal node path logic.
        let root = shared.lock().unwrap().tree.root().unwrap();
        assert_eq!(root, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_snapshot_is_refused_while_a_scan_is_running() {
        let dir = temp_dir("apply-running");
        let (tree, cats) = build();
        let entry = save_in(&dir, &tree, &cats, &meta(1_700_000_400)).unwrap();
        let snapshot = load_in(&dir, &entry.name).unwrap();
        let shared = live_session();
        shared.lock().unwrap().running = true;

        let err = apply_snapshot(&shared, &entry.name, snapshot).unwrap_err();
        assert!(err.contains("scan is running"), "unexpected error: {err}");

        // Nothing changed: the live tree, basket and metadata survive.
        let s = shared.lock().unwrap();
        assert_eq!(s.tree.name(0), "D:\\live");
        assert_eq!(s.basket.len(), 1);
        assert_eq!(s.scan_tier, Some(ScanTier::Mft));
        assert_eq!(s.scan_ms, 9_999);
        assert!(s.loaded_snapshot.is_none());
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
