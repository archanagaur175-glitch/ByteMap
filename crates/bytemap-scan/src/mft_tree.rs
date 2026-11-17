//! Tier A (MFT) tree building: an arena [`Tree`] from the helper's Record
//! stream (P6.T3b).
//!
//! The helper streams [`Record`]s for the requested subtree in MFT record
//! number order, which is not directory order in general (defensively, a
//! child can arrive before its parent). The builder therefore keeps
//!
//! - `dirs`: record number -> [`NodeId`] of every directory inserted so far,
//! - `pending`: parent record number -> children that arrived too early,
//!
//! and flushes `pending` whenever a directory lands. The first frame must
//! be the scan root (helper contract, bytemap-helper's mft.rs): it carries
//! the root's record number, which is how root children attach. The root
//! node itself is built from the canonicalized path exactly like the
//! walker (name = last component, `flags::DIR`, mtime 0 — the walker's
//! depth-0 quirk, mirrored for tier parity); the wire root name is
//! informational and ignored.
//!
//! Sizes: files take the wire logical size; size on disk is cluster
//! rounding (the helper sends `allocated = 0`), except compressed/sparse
//! files (`0x800`/`0x200`), which are re-measured on their real path after
//! the stream ends — the same rule the walker applies inline. Directory
//! own-disk usage stays 0 (rollups sum children; DECISIONS.md).
//!
//! Hard links: the helper emits one record per name inside the subtree,
//! matching walker runs with `hardlink_dedup = false` (the default; owner
//! decision 2026-11-17). Reparse points are recorded, never followed: the
//! helper's subtree filter already stops the descent at junctions.
//!
//! Entries whose parent never arrives (a directory freed between the
//! helper's two `$MFT` passes) cannot be placed; they are counted in
//! [`ScanStats::denied`] and dropped, keeping the scan partial but valid.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytemap_core::tree::{flags, NodeId, Tree};
use bytemap_helper::proto::Record;

use crate::error::ScanError;
use crate::metadata as m;
use crate::walker::ScanStats;

/// Low 48 bits of an MFT reference: the record number.
const RECORD_NUMBER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Build a tree from the helper's record stream for the scan root `root`.
///
/// The root's own path must exist; `records` must start with the root
/// record (`file_ref == parent_ref`, helper contract).
pub fn build_tree_from_records<I>(root: &Path, records: I) -> Result<(Tree, ScanStats), ScanError>
where
    I: IntoIterator<Item = Record>,
{
    let canonical = std::fs::canonicalize(root).map_err(|e| ScanError::Root(root.to_owned(), e))?;
    let cluster = m::cluster_size(&canonical);

    let mut records = records.into_iter();
    let first = records
        .next()
        .ok_or_else(|| ScanError::Stream("record stream was empty".to_owned()))?;

    let mut builder = Builder {
        cluster,
        tree: Tree::new(),
        dirs: HashMap::new(),
        pending: HashMap::new(),
        restat: Vec::new(),
        stats: ScanStats::default(),
        root_path: canonical.clone(),
    };
    // Root node, exactly like the walker's depth-0 handling.
    let root_id = builder.tree.insert_root(&last_component(&canonical));
    builder.stats.dirs += 1;

    let mut seen_root = false;
    for record in std::iter::once(first).chain(records) {
        if !seen_root {
            seen_root = true;
            let number = record.file_ref & RECORD_NUMBER_MASK;
            if number != record.parent_ref & RECORD_NUMBER_MASK {
                return Err(ScanError::Stream(format!(
                    "first record is not the scan root: {record:?}"
                )));
            }
            builder.dirs.insert(number, root_id);
            continue;
        }
        builder.insert(record);
    }

    // Children whose parent never showed up: not placeable, counted and
    // dropped (partial coverage, like denied entries).
    let orphans: usize = builder.pending.values().map(Vec::len).sum();
    builder.stats.denied += orphans as u64;

    // Compressed/sparse files: exact on-disk size from the real path.
    let restat = std::mem::take(&mut builder.restat);
    for (id, logical, attrs) in restat {
        let path = builder.path_of(id);
        let on_disk = m::size_on_disk(&path, logical, attrs, cluster);
        builder.tree.set_on_disk(id, on_disk);
    }

    bytemap_core::recompute_rollups(&mut builder.tree);
    Ok((builder.tree, builder.stats))
}

struct Builder {
    cluster: u32,
    tree: Tree,
    /// Record number -> NodeId for every directory inserted so far.
    dirs: HashMap<u64, NodeId>,
    /// parent record number -> children that arrived before their parent.
    pending: HashMap<u64, Vec<Record>>,
    /// (node id, logical, attrs) of compressed/sparse files to re-measure.
    restat: Vec<(NodeId, u64, u32)>,
    stats: ScanStats,
    /// Canonicalized scan root; paths are built under it.
    root_path: PathBuf,
}

impl Builder {
    fn insert(&mut self, record: Record) {
        let parent_number = record.parent_ref & RECORD_NUMBER_MASK;
        match self.dirs.get(&parent_number) {
            Some(&parent_id) => self.insert_under(parent_id, record),
            None => self.pending.entry(parent_number).or_default().push(record),
        }
    }

    fn insert_under(&mut self, parent_id: NodeId, record: Record) {
        let is_dir = record.attributes & m::FILE_ATTRIBUTE_DIRECTORY != 0;
        let (mut node_flags, _) = m::classify(record.attributes);
        if is_dir {
            node_flags |= flags::DIR;
            let number = record.file_ref & RECORD_NUMBER_MASK;
            let id =
                self.tree
                    .insert_child(parent_id, &record.name, node_flags, record.mtime, 0, 0);
            self.dirs.insert(number, id);
            self.stats.dirs += 1;
            // Everything that queued up under this directory is placeable
            // now (recursion: queued children may be directories with
            // queues of their own).
            let queued = self.pending.remove(&number).unwrap_or_default();
            for child in queued {
                self.insert_under(id, child);
            }
        } else {
            let logical = record.logical_size;
            // Default on-disk rule: cluster rounding (metadata.rs).
            let rounded = if logical == 0 {
                0
            } else {
                logical.div_ceil(u64::from(self.cluster)) * u64::from(self.cluster)
            };
            let id = self.tree.insert_child(
                parent_id,
                &record.name,
                node_flags,
                record.mtime,
                logical,
                rounded,
            );
            self.stats.files += 1;
            if logical > 0
                && record.attributes
                    & (m::FILE_ATTRIBUTE_COMPRESSED | m::FILE_ATTRIBUTE_SPARSE_FILE)
                    != 0
            {
                self.restat.push((id, logical, record.attributes));
            }
        }
    }

    /// Full path of a node: the canonical root plus the names above it
    /// (the root tree node's own name is skipped; the root path already
    /// ends in the root's real position).
    fn path_of(&self, id: NodeId) -> PathBuf {
        let mut names: Vec<&str> = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            names.push(self.tree.name(node));
            current = self.tree.parent(node);
        }
        let mut path = self.root_path.clone();
        for name in names.into_iter().skip(1).rev() {
            path.push(name);
        }
        path
    }
}

/// The last path component, like the walker's root naming.
fn last_component(path: &Path) -> String {
    path.components()
        .next_back()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemap_core::tree::flags;

    fn record(file_ref: u64, parent_ref: u64, name: &str, attrs: u32, logical: u64) -> Record {
        Record {
            file_ref,
            parent_ref,
            logical_size: logical,
            allocated_size: 0,
            attributes: attrs,
            mtime: 1_325_000_000_000_000_000,
            name: name.to_owned(),
        }
    }

    fn fixture_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bytemap-mft-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Out-of-order arrival: the child of a late directory queues up and
    /// flushes when the directory lands.
    #[test]
    fn pending_children_flush_when_the_parent_arrives() {
        let root = fixture_root();
        let dir_flag = m::FILE_ATTRIBUTE_DIRECTORY;
        // Root(1); late dir(10) under root arrives AFTER its child file(11).
        let records = [
            record(1, 1, "r", dir_flag, 0),
            record(11, 10, "late-file.txt", 0, 5_000),
            record(10, 1, "late-dir", dir_flag, 0),
        ];
        let (tree, stats) = build_tree_from_records(&root, records).unwrap();
        let root_id = tree.root().unwrap();
        let late = tree.find_child(root_id, "late-dir").expect("late dir");
        assert!(tree.find_child(late, "late-file.txt").is_some());
        assert_eq!(stats.dirs, 2, "root + late-dir");
        assert_eq!(stats.files, 1);
        assert_eq!(stats.denied, 0);
        assert_eq!(tree.node(late).agg_logical(), 5_000);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Orphans (parent never arrives) are counted as denied and dropped.
    #[test]
    fn orphans_are_counted_and_dropped() {
        let root = fixture_root();
        let dir_flag = m::FILE_ATTRIBUTE_DIRECTORY;
        let records = [
            record(1, 1, "r", dir_flag, 0),
            record(50, 999, "orphan.txt", 0, 100), // parent 999 never comes
            record(2, 1, "real.txt", 0, 10),
        ];
        let (tree, stats) = build_tree_from_records(&root, records).unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.denied, 1);
        let root_id = tree.root().unwrap();
        assert!(tree.find_child(root_id, "orphan.txt").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Flags and sizes flow through: reparse + placeholder flags, cluster
    /// rounding, compressed re-measurement (non-compressed file here, so
    /// GetCompressedFileSize returns the allocation size == rounding).
    #[test]
    fn flags_sizes_and_restat() {
        let root = fixture_root();
        let file_path = root.join("c.txt");
        std::fs::write(&file_path, vec![b'x'; 5_000]).expect("write");
        let dir_flag = m::FILE_ATTRIBUTE_DIRECTORY;
        let records = [
            record(1, 1, "r", dir_flag, 0),
            // Compressed attribute forces the deferred re-measurement path.
            record(2, 1, "c.txt", m::FILE_ATTRIBUTE_COMPRESSED, 5_000),
            record(
                3,
                1,
                "junction",
                dir_flag | m::FILE_ATTRIBUTE_REPARSE_POINT,
                0,
            ),
        ];
        let (tree, stats) = build_tree_from_records(&root, records).unwrap();
        let root_id = tree.root().unwrap();
        let c = tree.find_child(root_id, "c.txt").unwrap();
        // 5000 bytes on a 4096-cluster volume -> 8192 either way.
        assert_eq!(tree.node(c).agg_logical(), 5_000);
        assert_eq!(tree.node(c).agg_on_disk(), 8_192);
        // The wire mtime flows through untouched.
        assert_eq!(tree.node(c).mtime(), 1_325_000_000_000_000_000);
        let j = tree.find_child(root_id, "junction").unwrap();
        assert_eq!(tree.node(j).flags() & flags::REPARSE, flags::REPARSE);
        assert_eq!(stats.dirs, 2, "root + junction dir");
        assert_eq!(stats.files, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Empty stream and a wrong first record are hard errors.
    #[test]
    fn stream_errors() {
        let root = fixture_root();
        assert!(matches!(
            build_tree_from_records(&root, Vec::new()),
            Err(ScanError::Stream(_))
        ));
        let bad = [record(7, 8, "not-root", 0, 0)];
        assert!(matches!(
            build_tree_from_records(&root, bad),
            Err(ScanError::Stream(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }
}
