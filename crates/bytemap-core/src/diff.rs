//! Snapshot diff (Phase 8, P8.T2): compare two saved scans by PATH.
//!
//! Node ids are snapshot-local, so the trees are walked together from their
//! roots: children are matched by name (case-insensitively, because NTFS is)
//! and only the nodes where something actually changed become entries.
//! Directories that exist in both snapshots are descended into and never
//! listed themselves — otherwise every ancestor of a changed file would
//! double-count the same bytes. A directory that only exists in one snapshot
//! is listed ONCE with its whole subtree total (`is_dir = true`), and its
//! children are not listed separately.
//!
//! `grew`/`shrank` are file-level logical-size changes (`after - before`).
//! `added`/`removed` carry the size on the side where they exist. The summary
//! counts every change; `entries` is ranked by absolute delta and capped
//! (the UI shows the top movers), so `entries.len() < total_changed` means
//! the list is truncated.
//!
//! Wire format (packed little-endian, version 1):
//! ```text
//! u16 version (= 1)
//! u32 returned_entry_count
//! u32 total_changed
//! u32 added_count, u32 removed_count, u32 grew_count, u32 shrank_count
//! u64 added_bytes, u64 removed_bytes, u64 grew_bytes, u64 shrank_bytes
//! u64 before_total, u64 after_total
//! per entry: u8 kind, u8 is_dir, u8 category, u8 tier,
//!            u64 before, u64 after, u32 path_len, path bytes (UTF-8)
//! ```
//! `kind` ids: 0 added, 1 removed, 2 grew, 3 shrank.

use std::fmt;

use std::collections::HashMap;

use crate::categorizer::CategoryMap;
use crate::snapshot::Snapshot;
use crate::tree::{flags, NodeId, Tree};

/// Wire version for [`serialize_diff`].
pub const DIFF_VERSION: u16 = 1;

/// Default number of ranked entries returned.
pub const DEFAULT_DIFF_ENTRIES: usize = 200;

/// Hard cap on returned entries (the summary still counts everything).
pub const MAX_DIFF_ENTRIES: usize = 5000;

/// What happened to one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// Present only in the new snapshot.
    Added = 0,
    /// Present only in the old snapshot.
    Removed = 1,
    /// A file that got larger.
    Grew = 2,
    /// A file that got smaller.
    Shrank = 3,
}

impl DiffKind {
    /// Wire id (`u8`).
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    pub kind: DiffKind,
    /// Absolute path (root name + parent chain), as spelled by the snapshot
    /// the entry comes from.
    pub path: String,
    pub is_dir: bool,
    /// Logical size in the old snapshot (0 for added).
    pub before: u64,
    /// Logical size in the new snapshot (0 for removed).
    pub after: u64,
    pub cat: u8,
    pub tier: u8,
}

impl DiffEntry {
    /// Signed logical-size change (`after - before`).
    pub fn delta(&self) -> i64 {
        self.after as i64 - self.before as i64
    }
}

/// Counts and byte totals for a whole diff (never capped).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub total_changed: u64,
    pub added_count: u64,
    pub removed_count: u64,
    pub grew_count: u64,
    pub shrank_count: u64,
    /// New bytes introduced by added entries (whole subtrees for folders).
    pub added_bytes: u64,
    /// Old bytes gone with removed entries.
    pub removed_bytes: u64,
    /// Sum of positive file deltas.
    pub grew_bytes: u64,
    /// Sum of absolute negative file deltas.
    pub shrank_bytes: u64,
    /// Root logical size of the old / new snapshot.
    pub before_total: u64,
    pub after_total: u64,
}

impl DiffSummary {
    /// Total logical-size change of the scan root.
    pub fn net(&self) -> i64 {
        self.after_total as i64 - self.before_total as i64
    }
}

/// A diff result: complete summary + capped, ranked entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub summary: DiffSummary,
    /// Ranked by absolute delta, descending; path ascending on ties.
    pub entries: Vec<DiffEntry>,
}

impl Diff {
    /// True when the entry list was cut (the summary still has everything).
    pub fn truncated(&self) -> bool {
        self.summary.total_changed > self.entries.len() as u64
    }
}

/// Why two snapshots cannot be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    /// A snapshot has no root node (never produced by this app).
    NoRoot(&'static str),
    /// Different scan roots: a diff of C:\ and D:\ is meaningless.
    RootMismatch { old: String, new: String },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffError::NoRoot(which) => write!(f, "the {which} snapshot has no root node"),
            DiffError::RootMismatch { old, new } => {
                write!(f, "snapshots have different roots: {old} vs {new}")
            }
        }
    }
}

impl std::error::Error for DiffError {}

/// Compare `old` with `new`. `max_entries` 0 = [`DEFAULT_DIFF_ENTRIES`],
/// clamped to [`MAX_DIFF_ENTRIES`].
pub fn diff_snapshots(
    old: &Snapshot,
    new: &Snapshot,
    max_entries: usize,
) -> Result<Diff, DiffError> {
    let old_root = old.tree.root().ok_or(DiffError::NoRoot("old"))?;
    let new_root = new.tree.root().ok_or(DiffError::NoRoot("new"))?;
    let old_name = old.tree.name(old_root);
    let new_name = new.tree.name(new_root);
    if old_name.to_lowercase() != new_name.to_lowercase() {
        return Err(DiffError::RootMismatch {
            old: old_name.to_string(),
            new: new_name.to_string(),
        });
    }

    let mut summary = DiffSummary {
        before_total: old.tree.node(old_root).agg_logical,
        after_total: new.tree.node(new_root).agg_logical,
        ..DiffSummary::default()
    };
    let mut entries: Vec<DiffEntry> = Vec::new();

    // Explicit stack (no recursion): a snapshot tree can be very deep, and a
    // deep diff must never overflow the command thread's stack.
    let mut stack: Vec<(NodeId, NodeId, String)> = vec![(old_root, new_root, old_name.to_string())];
    while let Some((a, b, path)) = stack.pop() {
        // Children of the old node, keyed by lowercase name.
        let mut old_children: HashMap<String, NodeId> = HashMap::new();
        for child in old.tree.children(a) {
            old_children.insert(old.tree.name(child).to_lowercase(), child);
        }

        for child in new.tree.children(b) {
            let key = new.tree.name(child).to_lowercase();
            let child_path = join_path(&path, new.tree.name(child));
            match old_children.remove(&key) {
                Some(old_child) => {
                    let old_dir = is_dir(&old.tree, old_child);
                    let new_dir = is_dir(&new.tree, child);
                    match (old_dir, new_dir) {
                        // Same directory on both sides: compare its children.
                        (true, true) => stack.push((old_child, child, child_path)),
                        // Same file: compare sizes; unchanged files are not entries.
                        (false, false) => {
                            let before = old.tree.node(old_child).agg_logical;
                            let after = new.tree.node(child).agg_logical;
                            if after > before {
                                push_entry(
                                    &mut entries,
                                    &mut summary,
                                    DiffEntry {
                                        kind: DiffKind::Grew,
                                        path: child_path,
                                        is_dir: false,
                                        before,
                                        after,
                                        cat: cat_of(&new.cats, child),
                                        tier: tier_of(&new.cats, child),
                                    },
                                );
                            } else if after < before {
                                push_entry(
                                    &mut entries,
                                    &mut summary,
                                    DiffEntry {
                                        kind: DiffKind::Shrank,
                                        path: child_path,
                                        is_dir: false,
                                        before,
                                        after,
                                        cat: cat_of(&new.cats, child),
                                        tier: tier_of(&new.cats, child),
                                    },
                                );
                            }
                        }
                        // A file replaced by a directory (or the reverse):
                        // the old node disappeared and a new one appeared.
                        (true, false) => {
                            let before = old.tree.node(old_child).agg_logical;
                            push_entry(
                                &mut entries,
                                &mut summary,
                                DiffEntry {
                                    kind: DiffKind::Removed,
                                    path: child_path.clone(),
                                    is_dir: true,
                                    before,
                                    after: 0,
                                    cat: cat_of(&old.cats, old_child),
                                    tier: tier_of(&old.cats, old_child),
                                },
                            );
                            push_entry(
                                &mut entries,
                                &mut summary,
                                DiffEntry {
                                    kind: DiffKind::Added,
                                    path: child_path,
                                    is_dir: false,
                                    before: 0,
                                    after: new.tree.node(child).agg_logical,
                                    cat: cat_of(&new.cats, child),
                                    tier: tier_of(&new.cats, child),
                                },
                            );
                        }
                        (false, true) => {
                            let before = old.tree.node(old_child).agg_logical;
                            push_entry(
                                &mut entries,
                                &mut summary,
                                DiffEntry {
                                    kind: DiffKind::Removed,
                                    path: child_path.clone(),
                                    is_dir: false,
                                    before,
                                    after: 0,
                                    cat: cat_of(&old.cats, old_child),
                                    tier: tier_of(&old.cats, old_child),
                                },
                            );
                            // Do NOT descend: the directory entry already
                            // carries the whole new subtree total.
                            push_entry(
                                &mut entries,
                                &mut summary,
                                DiffEntry {
                                    kind: DiffKind::Added,
                                    path: child_path,
                                    is_dir: true,
                                    before: 0,
                                    after: new.tree.node(child).agg_logical,
                                    cat: cat_of(&new.cats, child),
                                    tier: tier_of(&new.cats, child),
                                },
                            );
                        }
                    }
                }
                None => {
                    // Added subtree, collapsed to its topmost directory.
                    push_entry(
                        &mut entries,
                        &mut summary,
                        DiffEntry {
                            kind: DiffKind::Added,
                            path: child_path,
                            is_dir: is_dir(&new.tree, child),
                            before: 0,
                            after: new.tree.node(child).agg_logical,
                            cat: cat_of(&new.cats, child),
                            tier: tier_of(&new.cats, child),
                        },
                    );
                }
            }
        }

        // Anything left was present only in the old snapshot.
        for old_child in old_children.into_values() {
            push_entry(
                &mut entries,
                &mut summary,
                DiffEntry {
                    kind: DiffKind::Removed,
                    path: join_path(&path, old.tree.name(old_child)),
                    is_dir: is_dir(&old.tree, old_child),
                    before: old.tree.node(old_child).agg_logical,
                    after: 0,
                    cat: cat_of(&old.cats, old_child),
                    tier: tier_of(&old.cats, old_child),
                },
            );
        }
    }

    // Top movers: biggest absolute delta first, then path for determinism.
    entries.sort_by(|x, y| {
        y.delta()
            .unsigned_abs()
            .cmp(&x.delta().unsigned_abs())
            .then_with(|| x.path.cmp(&y.path))
            .then_with(|| x.kind.as_u8().cmp(&y.kind.as_u8()))
    });
    let cap = if max_entries == 0 {
        DEFAULT_DIFF_ENTRIES
    } else {
        max_entries.min(MAX_DIFF_ENTRIES)
    };
    entries.truncate(cap);

    Ok(Diff { summary, entries })
}

/// Serialize a [`Diff`] in the binary format documented above.
pub fn serialize_diff(diff: &Diff) -> Vec<u8> {
    let s = &diff.summary;
    let cap = diff
        .entries
        .iter()
        .map(|e| 20 + e.path.len())
        .sum::<usize>();
    let mut out = Vec::with_capacity(74 + cap);
    put_u16(&mut out, DIFF_VERSION);
    put_u32(&mut out, diff.entries.len() as u32);
    put_u32(&mut out, sat_u32(s.total_changed));
    put_u32(&mut out, sat_u32(s.added_count));
    put_u32(&mut out, sat_u32(s.removed_count));
    put_u32(&mut out, sat_u32(s.grew_count));
    put_u32(&mut out, sat_u32(s.shrank_count));
    put_u64(&mut out, s.added_bytes);
    put_u64(&mut out, s.removed_bytes);
    put_u64(&mut out, s.grew_bytes);
    put_u64(&mut out, s.shrank_bytes);
    put_u64(&mut out, s.before_total);
    put_u64(&mut out, s.after_total);
    for entry in &diff.entries {
        out.push(entry.kind.as_u8());
        out.push(u8::from(entry.is_dir));
        out.push(entry.cat);
        out.push(entry.tier);
        put_u64(&mut out, entry.before);
        put_u64(&mut out, entry.after);
        put_u32(&mut out, entry.path.len() as u32);
        out.extend_from_slice(entry.path.as_bytes());
    }
    out
}

fn push_entry(entries: &mut Vec<DiffEntry>, summary: &mut DiffSummary, entry: DiffEntry) {
    summary.total_changed += 1;
    match entry.kind {
        DiffKind::Added => {
            summary.added_count += 1;
            summary.added_bytes += entry.after;
        }
        DiffKind::Removed => {
            summary.removed_count += 1;
            summary.removed_bytes += entry.before;
        }
        DiffKind::Grew => {
            summary.grew_count += 1;
            summary.grew_bytes += entry.after - entry.before;
        }
        DiffKind::Shrank => {
            summary.shrank_count += 1;
            summary.shrank_bytes += entry.before - entry.after;
        }
    }
    entries.push(entry);
}

fn is_dir(tree: &Tree, id: NodeId) -> bool {
    tree.node(id).flags & flags::DIR != 0
}

fn cat_of(cats: &CategoryMap, id: NodeId) -> u8 {
    cats.cat.get(id as usize).copied().unwrap_or(0)
}

fn tier_of(cats: &CategoryMap, id: NodeId) -> u8 {
    cats.tier.get(id as usize).copied().unwrap_or(0)
}

/// Join a parent path and a child name without doubling separators
/// (drive roots like `C:\` already end with one).
fn join_path(parent: &str, name: &str) -> String {
    let mut out = String::with_capacity(parent.len() + 1 + name.len());
    out.push_str(parent);
    if !out.ends_with('\\') && !out.ends_with('/') {
        out.push('\\');
    }
    out.push_str(name);
    out
}

fn sat_u32(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categorizer::{categorize, category, tier, Rules};
    use crate::rollup::recompute_rollups;
    use crate::tree::flags;

    fn empty_snapshot(root: &str) -> Snapshot {
        let mut tree = Tree::new();
        tree.insert_root(root);
        recompute_rollups(&mut tree);
        let cats = categorize(&tree, &Rules::default_rules());
        Snapshot {
            meta: crate::snapshot::SnapshotMeta {
                created_unix: 0,
                scan_ms: 0,
                tier: crate::snapshot::ScanTier::Walker,
                root_name: root.to_string(),
                app_version: "test".to_string(),
            },
            tree,
            cats,
        }
    }

    /// Build a snapshot from a flat file list under `root`.
    fn flat(root: &str, files: &[(&str, u64)]) -> Snapshot {
        let mut snapshot = empty_snapshot(root);
        let root_id = snapshot.tree.root().unwrap();
        for (name, size) in files {
            snapshot
                .tree
                .insert_child(root_id, name, 0, 0, *size, *size);
        }
        recompute_rollups(&mut snapshot.tree);
        snapshot.cats = categorize(&snapshot.tree, &Rules::default_rules());
        snapshot
    }

    fn by_path<'a>(diff: &'a Diff, path: &str, kind: DiffKind) -> &'a DiffEntry {
        diff.entries
            .iter()
            .find(|e| e.path == path && e.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} entry for {path}: {:#?}", diff.entries))
    }

    #[test]
    fn identical_snapshots_have_no_changes() {
        let old = flat("C:\\fixture", &[("a.txt", 100), ("b.bin", 50)]);
        let new = flat("C:\\fixture", &[("a.txt", 100), ("b.bin", 50)]);
        let diff = diff_snapshots(&old, &new, 0).unwrap();
        assert!(diff.entries.is_empty());
        assert_eq!(diff.summary.total_changed, 0);
        assert_eq!(diff.summary.net(), 0);
        assert_eq!(diff.summary.before_total, 150);
        assert!(!diff.truncated());
    }

    #[test]
    fn added_and_removed_files_are_listed_with_their_sizes() {
        let old = flat("C:\\fixture", &[("gone.bin", 300)]);
        let new = flat("C:\\fixture", &[("new.bin", 500)]);
        let diff = diff_snapshots(&old, &new, 0).unwrap();

        assert_eq!(diff.entries.len(), 2);
        // Ranked by absolute delta: 500 before 300.
        assert_eq!(diff.entries[0].kind, DiffKind::Added);
        assert_eq!(diff.entries[0].path, "C:\\fixture\\new.bin");
        assert_eq!(diff.entries[0].after, 500);
        assert_eq!(diff.entries[0].before, 0);
        assert_eq!(diff.entries[0].delta(), 500);
        assert_eq!(diff.entries[1].kind, DiffKind::Removed);
        assert_eq!(diff.entries[1].before, 300);

        let s = diff.summary;
        assert_eq!(s.added_count, 1);
        assert_eq!(s.removed_count, 1);
        assert_eq!(s.added_bytes, 500);
        assert_eq!(s.removed_bytes, 300);
        assert_eq!(s.total_changed, 2);
        assert_eq!(s.net(), 200);
    }

    #[test]
    fn file_growth_and_shrink_report_deltas_and_skip_directories() {
        let old = flat(
            "C:\\fixture",
            &[("grew.txt", 100), ("shrank.bin", 200), ("same.log", 10)],
        );
        let new = flat(
            "C:\\fixture",
            &[("grew.txt", 250), ("shrank.bin", 50), ("same.log", 10)],
        );
        let diff = diff_snapshots(&old, &new, 0).unwrap();

        assert_eq!(diff.entries.len(), 2);
        let grew = by_path(&diff, "C:\\fixture\\grew.txt", DiffKind::Grew);
        assert_eq!((grew.before, grew.after, grew.delta()), (100, 250, 150));
        let shrank = by_path(&diff, "C:\\fixture\\shrank.bin", DiffKind::Shrank);
        assert_eq!(
            (shrank.before, shrank.after, shrank.delta()),
            (200, 50, -150)
        );

        assert_eq!(diff.summary.grew_count, 1);
        assert_eq!(diff.summary.grew_bytes, 150);
        assert_eq!(diff.summary.shrank_count, 1);
        assert_eq!(diff.summary.shrank_bytes, 150);
        // The root changed too, but directories are never entries.
        assert!(!diff.entries.iter().any(|e| e.is_dir));
    }

    #[test]
    fn added_folder_collapses_its_subtree_into_one_entry() {
        let old = empty_snapshot("C:\\fixture");
        let mut new = empty_snapshot("C:\\fixture");
        let new_root = new.tree.root().unwrap();
        let dir = new
            .tree
            .insert_child(new_root, "fresh", flags::DIR, 0, 0, 0);
        new.tree.insert_child(dir, "a.dat", 0, 0, 100, 100);
        new.tree.insert_child(dir, "b.dat", 0, 0, 50, 50);
        recompute_rollups(&mut new.tree);
        new.cats = categorize(&new.tree, &Rules::default_rules());

        let diff = diff_snapshots(&old, &new, 0).unwrap();
        assert_eq!(diff.entries.len(), 1, "{:#?}", diff.entries);
        let entry = &diff.entries[0];
        assert_eq!(entry.kind, DiffKind::Added);
        assert_eq!(entry.path, "C:\\fixture\\fresh");
        assert!(entry.is_dir);
        assert_eq!(entry.after, 150);
        assert_eq!(diff.summary.total_changed, 1);
        assert_eq!(diff.summary.added_bytes, 150);
        assert!(diff.entries.iter().all(|e| !e.path.ends_with("a.dat")));
    }

    #[test]
    fn removed_folder_collapses_its_subtree_into_one_entry() {
        let mut old = empty_snapshot("C:\\fixture");
        let old_root = old.tree.root().unwrap();
        let dir = old.tree.insert_child(old_root, "gone", flags::DIR, 0, 0, 0);
        old.tree.insert_child(dir, "a.dat", 0, 0, 7, 7);
        recompute_rollups(&mut old.tree);
        old.cats = categorize(&old.tree, &Rules::default_rules());
        let new = empty_snapshot("C:\\fixture");

        let diff = diff_snapshots(&old, &new, 0).unwrap();
        assert_eq!(diff.entries.len(), 1, "{:#?}", diff.entries);
        assert_eq!(diff.entries[0].kind, DiffKind::Removed);
        assert_eq!(diff.entries[0].path, "C:\\fixture\\gone");
        assert!(diff.entries[0].is_dir);
        assert_eq!(diff.entries[0].before, 7);
        assert_eq!(diff.summary.removed_bytes, 7);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let old = flat("C:\\fixture", &[("Report.TXT", 100)]);
        let new = flat("c:\\FIXTURE", &[("report.txt", 100)]);
        let diff = diff_snapshots(&old, &new, 0).unwrap();
        assert!(diff.entries.is_empty(), "{:#?}", diff.entries);
        assert_eq!(diff.summary.total_changed, 0);
    }

    #[test]
    fn file_replaced_by_folder_is_a_remove_plus_an_add() {
        let old = flat("C:\\fixture", &[("thing", 10)]);
        let mut new = empty_snapshot("C:\\fixture");
        let new_root = new.tree.root().unwrap();
        let dir = new
            .tree
            .insert_child(new_root, "thing", flags::DIR, 0, 0, 0);
        new.tree.insert_child(dir, "inner.bin", 0, 0, 70, 70);
        recompute_rollups(&mut new.tree);
        new.cats = categorize(&new.tree, &Rules::default_rules());

        let diff = diff_snapshots(&old, &new, 0).unwrap();
        assert_eq!(diff.entries.len(), 2, "{:#?}", diff.entries);
        assert!(by_path(&diff, "C:\\fixture\\thing", DiffKind::Removed).before == 10);
        let added = by_path(&diff, "C:\\fixture\\thing", DiffKind::Added);
        assert!(added.is_dir);
        assert_eq!(added.after, 70);
        // The new subtree is collapsed: inner.bin is not a separate entry.
        assert!(diff.entries.iter().all(|e| !e.path.ends_with("inner.bin")));
    }

    #[test]
    fn top_movers_are_capped_but_the_summary_is_complete() {
        let old = flat(
            "C:\\fixture",
            &[("f1", 1), ("f2", 20), ("f3", 300), ("f4", 4), ("f5", 50)],
        );
        let new = flat(
            "C:\\fixture",
            &[("f1", 11), ("f2", 40), ("f3", 600), ("f4", 9), ("f5", 80)],
        );
        let diff = diff_snapshots(&old, &new, 2).unwrap();
        assert_eq!(diff.entries.len(), 2);
        assert_eq!(diff.entries[0].path, "C:\\fixture\\f3"); // +300
        assert_eq!(diff.entries[1].path, "C:\\fixture\\f5"); // +30
        assert_eq!(diff.summary.total_changed, 5);
        assert_eq!(diff.summary.grew_count, 5);
        assert!(diff.truncated());

        // max_entries is clamped to the hard cap.
        let diff = diff_snapshots(&old, &new, MAX_DIFF_ENTRIES + 1000).unwrap();
        assert_eq!(diff.entries.len(), 5);
    }

    #[test]
    fn different_roots_and_missing_roots_are_refused() {
        let a = flat("C:\\fixture", &[("a", 1)]);
        let b = flat("D:\\other", &[("a", 1)]);
        assert_eq!(
            diff_snapshots(&a, &b, 0).unwrap_err(),
            DiffError::RootMismatch {
                old: "C:\\fixture".to_string(),
                new: "D:\\other".to_string(),
            }
        );

        let empty = Snapshot {
            meta: a.meta.clone(),
            tree: Tree::new(),
            cats: CategoryMap {
                cat: Vec::new(),
                tier: Vec::new(),
            },
        };
        assert_eq!(
            diff_snapshots(&empty, &a, 0).unwrap_err(),
            DiffError::NoRoot("old")
        );
        assert_eq!(
            diff_snapshots(&a, &empty, 0).unwrap_err(),
            DiffError::NoRoot("new")
        );
    }

    #[test]
    fn categories_come_from_the_side_the_entry_exists_on() {
        let old = flat("C:\\fixture", &[("gone.exe", 10)]);
        let new = flat("C:\\fixture", &[("new.txt", 20)]);
        let diff = diff_snapshots(&old, &new, 0).unwrap();
        let added = by_path(&diff, "C:\\fixture\\new.txt", DiffKind::Added);
        assert_eq!(added.cat, category::DOCUMENTS);
        assert_eq!(added.tier, tier::REVIEW);
        let removed = by_path(&diff, "C:\\fixture\\gone.exe", DiffKind::Removed);
        assert_eq!(removed.cat, category::APPS);
        assert_eq!(removed.tier, tier::REVIEW);
    }

    #[test]
    fn wire_format_is_stable_and_deterministic() {
        let old = flat("C:\\fixture", &[("gone.bin", 300)]);
        let new = flat("C:\\fixture", &[("new.txt", 500)]);
        let diff = diff_snapshots(&old, &new, 0).unwrap();
        let bytes = serialize_diff(&diff);
        assert_eq!(bytes, serialize_diff(&diff));

        let d = &mut Decoder::new(&bytes);
        assert_eq!(d.u16(), DIFF_VERSION);
        assert_eq!(d.u32(), 2); // entries
        assert_eq!(d.u32(), 2); // total_changed
        assert_eq!(d.u32(), 1); // added
        assert_eq!(d.u32(), 1); // removed
        assert_eq!(d.u32(), 0); // grew
        assert_eq!(d.u32(), 0); // shrank
        assert_eq!(d.u64(), 500);
        assert_eq!(d.u64(), 300);
        assert_eq!(d.u64(), 0);
        assert_eq!(d.u64(), 0);
        assert_eq!(d.u64(), 300); // before total
        assert_eq!(d.u64(), 500); // after total

        let added = &diff.entries[0];
        assert_eq!(d.u8(), DiffKind::Added.as_u8());
        assert_eq!(d.u8(), 0);
        assert_eq!(d.u8(), added.cat);
        assert_eq!(d.u8(), added.tier);
        assert_eq!(d.u64(), 0);
        assert_eq!(d.u64(), 500);
        assert_eq!(d.str(), "C:\\fixture\\new.txt");

        let removed = &diff.entries[1];
        assert_eq!(d.u8(), DiffKind::Removed.as_u8());
        assert_eq!(d.u8(), 0);
        assert_eq!(d.u8(), removed.cat);
        assert_eq!(d.u8(), removed.tier);
        assert_eq!(d.u64(), 300);
        assert_eq!(d.u64(), 0);
        assert_eq!(d.str(), "C:\\fixture\\gone.bin");
        assert!(d.at_end());
    }

    /// Minimal little-endian reader used only by tests.
    struct Decoder<'a> {
        buf: &'a [u8],
        pos: usize,
    }

    impl<'a> Decoder<'a> {
        fn new(buf: &'a [u8]) -> Self {
            Decoder { buf, pos: 0 }
        }
        fn take(&mut self, n: usize) -> &'a [u8] {
            let s = &self.buf[self.pos..self.pos + n];
            self.pos += n;
            s
        }
        fn u8(&mut self) -> u8 {
            self.take(1)[0]
        }
        fn u16(&mut self) -> u16 {
            u16::from_le_bytes(self.take(2).try_into().unwrap())
        }
        fn u32(&mut self) -> u32 {
            u32::from_le_bytes(self.take(4).try_into().unwrap())
        }
        fn u64(&mut self) -> u64 {
            u64::from_le_bytes(self.take(8).try_into().unwrap())
        }
        fn str(&mut self) -> String {
            let len = self.u32() as usize;
            String::from_utf8(self.take(len).to_vec()).unwrap()
        }
        fn at_end(&self) -> bool {
            self.pos == self.buf.len()
        }
    }
}
