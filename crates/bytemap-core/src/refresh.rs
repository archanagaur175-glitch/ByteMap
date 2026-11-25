//! Applying a USN change delta to a snapshot tree (P8.T3).
//!
//! The elevated helper turns a USN journal range into [`ChangeFact`]s: the
//! current absolute path of every touched entry, whether it still exists,
//! and (when it does) its current metadata. This module folds those facts
//! into an existing snapshot tree without touching the disk.
//!
//! Paths, not file ids: snapshots deliberately store no MFT file ids (they
//! would be useless after a format or copy, and the format stays smaller),
//! so the refresh matches by path. The helper resolves USN file ids to
//! live paths before sending facts (see docs/agent/DECISIONS.md).
//!
//! Matching is case-insensitive per NTFS rules. A fact replaces the state
//! of exactly its path: the last fact for a path wins, and a delete that
//! was later followed by a create drops the old subtree first (so a
//! recreated folder never inherits stale children from the snapshot).
//!
//! Facts that cannot be placed — a child of a path the snapshot does not
//! know and no fact created — are counted as `unresolved` instead of being
//! guessed into the tree. The caller decides how loud to be about them.
//!
//! The input tree is never modified; the result is a fresh tree with child
//! links and depth rebuilt for the new arena, then rolled up. Root name is
//! preserved, so a volume snapshot keeps its `\` root.

use std::cell::Cell;
use std::collections::BTreeMap;

use crate::rollup::recompute_rollups;
use crate::tree::{flags, NodeId, Tree};

/// Hard ceiling on distinct changed paths for one refresh. A larger delta
/// means something is off (a wrapped journal, a huge burst); the caller
/// falls back to a full rescan rather than applying half a delta.
pub const MAX_REFRESH_CHANGES: usize = 500_000;

/// One changed path from the elevated helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFact {
    /// Absolute path as it exists now (or existed before removal).
    pub path: String,
    /// `true` = the entry exists now, `false` = it was removed.
    pub exists: bool,
    /// [`flags::DIR`], [`flags::REPARSE`], [`flags::PLACEHOLDER`] for an
    /// existing entry; ignored when `exists` is false.
    pub flags: u16,
    /// Logical size for files (0 for directories).
    pub logical: u64,
    /// Size on disk for files (0 for directories).
    pub on_disk: u64,
    /// Last write time as Windows FILETIME ticks.
    pub mtime: i64,
}

/// What one refresh did, for the UI and the log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshStats {
    /// Facts received (after the caller's own dedupe).
    pub facts: usize,
    /// Nodes inserted (new entries, recreated folders, replacements).
    pub added: u64,
    /// Nodes dropped (deletions and replaced subtrees).
    pub removed: u64,
    /// Existing nodes whose metadata was refreshed.
    pub updated: u64,
    /// Facts that could not be placed in this tree.
    pub unresolved: u64,
    pub nodes_before: u64,
    pub nodes_after: u64,
}

/// Why a refresh cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    /// The tree has no root, so there is nothing to refresh.
    EmptyTree,
    /// More distinct paths than [`MAX_REFRESH_CHANGES`].
    TooManyChanges { max: usize },
    /// A fact path is not inside the snapshot root.
    OutsideRoot { path: String, root: String },
    /// A fact path has an empty, `.` or `..` component.
    BadPath { path: String },
}

impl std::fmt::Display for RefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefreshError::EmptyTree => f.write_str("cannot refresh a tree without a root"),
            RefreshError::TooManyChanges { max } => {
                write!(
                    f,
                    "more than {max} changed paths; a full rescan is required"
                )
            }
            RefreshError::OutsideRoot { path, root } => {
                write!(f, "{path} is not inside the snapshot root {root}")
            }
            RefreshError::BadPath { path } => write!(f, "bad change path: {path}"),
        }
    }
}

impl std::error::Error for RefreshError {}

/// Fold a delta into `tree` and return the refreshed tree.
pub fn apply_changes(
    tree: &Tree,
    root_path: &str,
    changes: &[ChangeFact],
) -> Result<(Tree, RefreshStats), RefreshError> {
    if tree.root().is_none() {
        return Err(RefreshError::EmptyTree);
    }
    if changes.len() > MAX_REFRESH_CHANGES {
        return Err(RefreshError::TooManyChanges {
            max: MAX_REFRESH_CHANGES,
        });
    }

    let mut root = ChangeNode::default();
    for fact in changes {
        let components = relative_components(&fact.path, root_path).ok_or_else(|| {
            // Same root, but a malformed component: that is a bad path, not
            // a foreign one.
            let inside = match (path_components(&fact.path), path_components(root_path)) {
                (Some(parts), Some(root_parts)) => prefix_eq(&parts, &root_parts),
                _ => false,
            };
            if inside {
                RefreshError::BadPath {
                    path: fact.path.clone(),
                }
            } else {
                RefreshError::OutsideRoot {
                    path: fact.path.clone(),
                    root: root_path.to_owned(),
                }
            }
        })?;
        insert_fact(&mut root, &components, fact);
    }

    let old_root = tree.root().expect("checked above");
    let mut stats = RefreshStats {
        facts: changes.len(),
        nodes_before: tree.len() as u64,
        ..RefreshStats::default()
    };

    let mut out = Tree::new();
    let new_root = out.insert_root(tree.name(old_root));
    let mut stack = vec![Frame {
        old: Some(old_root),
        new: new_root,
        trie: &root,
    }];

    while let Some(frame) = stack.pop() {
        let trie = frame.trie;
        trie.visited.set(true);

        // Changed children of this directory, keyed by lowercase name.
        let mut pending: BTreeMap<String, &ChangeNode> = trie
            .children
            .iter()
            .map(|(key, node)| (key.clone(), node))
            .collect();

        let mut emit: Vec<Emit> = Vec::new();
        if let Some(old_id) = frame.old {
            let children: Vec<NodeId> = tree.children(old_id).collect();
            for old_child in children {
                let key = tree.name(old_child).to_lowercase();
                match pending.remove(&key) {
                    Some(change) => {
                        change.visited.set(true);
                        push_changed(&mut emit, &mut stats, tree, old_child, change);
                    }
                    None => emit.push(Emit::Keep(old_child)),
                }
            }
        }

        // Paths the snapshot does not know: new entries from the journal.
        for change in pending.values() {
            change.visited.set(true);
            match &change.state {
                Some(ChangeState::Present(fact)) => emit.push(Emit::New(fact, change)),
                Some(ChangeState::Absent) => mark_visited(change),
                None => { /* only an intermediate path; children stay unresolved */ }
            }
        }

        // `insert_child` pushes at the head, so inserting in reverse keeps
        // the snapshot's child order (and the saved file byte-identical
        // for an unchanged subtree).
        for item in emit.iter().rev() {
            match item {
                Emit::Keep(old_child) => {
                    let node = tree.node(*old_child);
                    let new_child = out.insert_child(
                        frame.new,
                        tree.name(*old_child),
                        node.flags,
                        node.mtime(),
                        node.agg_logical(),
                        node.agg_on_disk(),
                    );
                    if node.flags & flags::DIR != 0 {
                        copy_subtree(tree, *old_child, &mut out, new_child);
                    }
                }
                Emit::Descend(old_child, change) => {
                    let node = tree.node(*old_child);
                    let new_child = out.insert_child(
                        frame.new,
                        tree.name(*old_child),
                        node.flags,
                        node.mtime(),
                        node.agg_logical(),
                        node.agg_on_disk(),
                    );
                    if node.flags & flags::DIR != 0 {
                        stack.push(Frame {
                            old: Some(*old_child),
                            new: new_child,
                            trie: change,
                        });
                    }
                }
                Emit::Update(old_child, fact, change) => {
                    let node = tree.node(*old_child);
                    let new_child = out.insert_child(
                        frame.new,
                        &change.name,
                        merged_flags(node.flags, fact.flags),
                        fact.mtime,
                        fact.logical,
                        fact.on_disk,
                    );
                    stats.updated += 1;
                    if node.flags & flags::DIR != 0 {
                        if change.children.is_empty() {
                            copy_subtree(tree, *old_child, &mut out, new_child);
                        } else {
                            stack.push(Frame {
                                old: Some(*old_child),
                                new: new_child,
                                trie: change,
                            });
                        }
                    }
                }
                Emit::ReplaceDir(old_child, fact, change) => {
                    stats.removed += count_subtree(tree, *old_child);
                    let new_child = out.insert_child(
                        frame.new,
                        &change.name,
                        merged_flags(0, fact.flags),
                        fact.mtime,
                        fact.logical,
                        fact.on_disk,
                    );
                    stats.added += 1;
                    stack.push(Frame {
                        old: None,
                        new: new_child,
                        trie: change,
                    });
                }
                Emit::ReplaceFile(old_child, fact, change) => {
                    stats.removed += count_subtree(tree, *old_child);
                    out.insert_child(
                        frame.new,
                        &change.name,
                        merged_flags(0, fact.flags),
                        fact.mtime,
                        fact.logical,
                        fact.on_disk,
                    );
                    stats.added += 1;
                }
                Emit::New(fact, change) => {
                    let new_child = out.insert_child(
                        frame.new,
                        &change.name,
                        merged_flags(0, fact.flags),
                        fact.mtime,
                        fact.logical,
                        fact.on_disk,
                    );
                    stats.added += 1;
                    if fact.flags & flags::DIR != 0 {
                        stack.push(Frame {
                            old: None,
                            new: new_child,
                            trie: change,
                        });
                    }
                }
            }
        }
    }

    stats.unresolved = count_unresolved(&root);
    stats.nodes_after = out.len() as u64;
    recompute_rollups(&mut out);
    Ok((out, stats))
}

/// Decide what happens to one old node that has a fact.
fn push_changed<'a>(
    emit: &mut Vec<Emit<'a>>,
    stats: &mut RefreshStats,
    tree: &Tree,
    old_child: NodeId,
    change: &'a ChangeNode,
) {
    let node = tree.node(old_child);
    let old_is_dir = node.flags & flags::DIR != 0;
    match &change.state {
        // The caller deduped facts, so this can only mean "still absent".
        Some(ChangeState::Absent) => {
            stats.removed += count_subtree(tree, old_child);
            mark_visited(change);
        }
        Some(ChangeState::Present(fact)) => {
            let new_is_dir = fact.flags & flags::DIR != 0;
            match (old_is_dir, new_is_dir) {
                (true, true) => {
                    if change.removed_before {
                        // Deleted and re-created at the same path: the old
                        // children are gone, never inherit them.
                        emit.push(Emit::ReplaceDir(old_child, fact, change));
                    } else {
                        emit.push(Emit::Update(old_child, fact, change));
                    }
                }
                (false, false) => emit.push(Emit::Update(old_child, fact, change)),
                (true, false) => emit.push(Emit::ReplaceFile(old_child, fact, change)),
                (false, true) => emit.push(Emit::ReplaceDir(old_child, fact, change)),
            }
        }
        // A path that only exists to reach a deeper fact: keep this node's
        // metadata but descend so the nested facts can land.
        None => {
            if change.children.is_empty() {
                emit.push(Emit::Keep(old_child));
            } else {
                emit.push(Emit::Descend(old_child, change));
            }
        }
    }
}

/// What to write into the refreshed tree for one child.
enum Emit<'a> {
    Keep(NodeId),
    Descend(NodeId, &'a ChangeNode),
    Update(NodeId, &'a ChangeFact, &'a ChangeNode),
    ReplaceDir(NodeId, &'a ChangeFact, &'a ChangeNode),
    ReplaceFile(NodeId, &'a ChangeFact, &'a ChangeNode),
    New(&'a ChangeFact, &'a ChangeNode),
}

/// One directory whose children are being rebuilt.
struct Frame<'a> {
    /// `None` for a directory created by this refresh.
    old: Option<NodeId>,
    new: NodeId,
    trie: &'a ChangeNode,
}

/// Case-insensitive change trie. `visited` tracks facts that found a home.
#[derive(Debug, Default)]
struct ChangeNode {
    /// Display name (the fact's current spelling).
    name: String,
    state: Option<ChangeState>,
    /// A delete was seen for this path before the final state.
    removed_before: bool,
    children: BTreeMap<String, ChangeNode>,
    visited: Cell<bool>,
}

#[derive(Debug, Clone)]
enum ChangeState {
    Present(ChangeFact),
    Absent,
}

fn insert_fact(root: &mut ChangeNode, components: &[&str], fact: &ChangeFact) {
    let mut node = root;
    for component in components {
        let key = component.to_lowercase();
        node = node.children.entry(key).or_insert_with(|| ChangeNode {
            name: (*component).to_owned(),
            ..ChangeNode::default()
        });
    }
    node.name = (*components.last().expect("non-empty components")).to_owned();
    if fact.exists {
        node.state = Some(ChangeState::Present(fact.clone()));
    } else {
        node.state = Some(ChangeState::Absent);
        node.removed_before = true;
    }
}

/// Directory flags preserved from the snapshot; the rest follow the fact.
const KEEP_FLAGS: u16 = flags::DENIED | flags::DUPLICATE_HARDLINK;

fn merged_flags(old: u16, fact: u16) -> u16 {
    (fact & (flags::DIR | flags::REPARSE | flags::PLACEHOLDER)) | (old & KEEP_FLAGS)
}

fn copy_subtree(src: &Tree, src_id: NodeId, dst: &mut Tree, dst_id: NodeId) {
    let mut stack = vec![(src_id, dst_id)];
    while let Some((from, to)) = stack.pop() {
        let children: Vec<NodeId> = src.children(from).collect();
        for child in children.iter().rev() {
            let node = src.node(*child);
            let new_child = dst.insert_child(
                to,
                src.name(*child),
                node.flags,
                node.mtime(),
                node.agg_logical(),
                node.agg_on_disk(),
            );
            if node.flags & flags::DIR != 0 {
                stack.push((*child, new_child));
            }
        }
    }
}

fn count_subtree(tree: &Tree, id: NodeId) -> u64 {
    let mut count = 0;
    let mut stack = vec![id];
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend(tree.children(node));
    }
    count
}

fn mark_visited(node: &ChangeNode) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        current.visited.set(true);
        stack.extend(current.children.values());
    }
}

fn count_unresolved(root: &ChangeNode) -> u64 {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.state.is_some() && !node.visited.get() {
            count += 1;
        }
        stack.extend(node.children.values());
    }
    count
}

/// Split an absolute Windows path into components, tolerating `\\?\` and
/// either slash. Drive roots (`C:`) stay one component.
fn path_components(path: &str) -> Option<Vec<String>> {
    let trimmed = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix("//?/"))
        .unwrap_or(path);
    let parts: Vec<String> = trimmed
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn prefix_eq(path: &[String], root: &[String]) -> bool {
    path.len() > root.len()
        && path
            .iter()
            .zip(root.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Components of `path` below `root`, or `None` when outside or malformed.
fn relative_components<'a>(path: &'a str, root: &str) -> Option<Vec<&'a str>> {
    let root_parts = path_components(root)?;
    let trimmed = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix("//?/"))
        .unwrap_or(path);

    let mut parts: Vec<&str> = trimmed
        .split(['\\', '/'])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() <= root_parts.len() {
        return None;
    }
    for (part, want) in parts.iter().zip(root_parts.iter()) {
        if !part.eq_ignore_ascii_case(want) {
            return None;
        }
    }
    let rest = parts.split_off(root_parts.len());
    if rest
        .iter()
        .any(|part| *part == "." || *part == ".." || part.len() > 1024)
    {
        return None;
    }
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::flags;

    fn fact(path: &str, dir: bool, logical: u64, on_disk: u64) -> ChangeFact {
        ChangeFact {
            path: path.to_owned(),
            exists: true,
            flags: if dir { flags::DIR } else { 0 },
            logical,
            on_disk,
            mtime: 7,
        }
    }

    fn removed(path: &str) -> ChangeFact {
        ChangeFact {
            path: path.to_owned(),
            exists: false,
            flags: 0,
            logical: 0,
            on_disk: 0,
            mtime: 0,
        }
    }

    /// C:\
    ///   docs\        (a.txt 10, sub\b.txt 20)
    ///   media\c.bin  (100)
    fn base() -> Tree {
        let mut t = Tree::new();
        let root = t.insert_root(r"\");
        let docs = t.insert_child(root, "docs", flags::DIR, 0, 0, 0);
        t.insert_child(docs, "a.txt", 0, 0, 10, 10);
        let sub = t.insert_child(docs, "sub", flags::DIR, 0, 0, 0);
        t.insert_child(sub, "b.txt", 0, 0, 20, 20);
        let media = t.insert_child(root, "media", flags::DIR, 0, 0, 0);
        t.insert_child(media, "c.bin", 0, 0, 100, 100);
        crate::rollup::recompute_rollups(&mut t);
        t
    }

    fn child(t: &Tree, parent: NodeId, name: &str) -> NodeId {
        t.find_child(parent, name)
            .unwrap_or_else(|| panic!("missing child {name}"))
    }

    fn total(t: &Tree) -> (u64, u64) {
        let root = t.root().unwrap();
        (t.node(root).agg_logical(), t.node(root).agg_on_disk())
    }

    #[test]
    fn grows_a_file_and_rolls_up() {
        let old = base();
        let (new, stats) =
            apply_changes(&old, r"C:\", &[fact(r"C:\docs\sub\b.txt", false, 50, 50)]).unwrap();

        assert_eq!(stats.updated, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(stats.removed, 0);
        assert_eq!(stats.unresolved, 0);
        assert_eq!(total(&new), (160, 160));
        let docs = child(&new, new.root().unwrap(), "docs");
        assert_eq!(new.node(docs).agg_logical(), 60);
    }

    #[test]
    fn creates_and_removes_entries() {
        let old = base();
        let changes = [
            fact(r"C:\docs\new.txt", false, 5, 5),
            fact(r"C:\media", true, 0, 0),
            removed(r"C:\media"),
            fact(r"C:\docs\sub", true, 0, 0),
        ];
        // Note: the media pair has remove AFTER create, so it must be gone.
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.added, 1, "docs\\new.txt only");
        assert_eq!(stats.removed, 2, "media and its c.bin child");
        assert_eq!(stats.updated, 1, "docs\\sub stays a folder");
        assert_eq!(stats.unresolved, 0);
        assert!(new.find_child(new.root().unwrap(), "media").is_none());
        let docs = child(&new, new.root().unwrap(), "docs");
        assert_eq!(new.node(child(&new, docs, "new.txt")).agg_logical(), 5);
    }

    #[test]
    fn deletes_a_subtree() {
        let old = base();
        let (new, stats) = apply_changes(&old, r"C:\", &[removed(r"C:\docs")]).unwrap();
        assert_eq!(stats.removed, 4, "docs, a.txt, sub, b.txt");
        assert_eq!(stats.added, 0);
        assert_eq!(total(&new), (100, 100));
        assert!(new.find_child(new.root().unwrap(), "docs").is_none());
    }

    #[test]
    fn creates_a_folder_with_children() {
        let old = base();
        let changes = [
            fact(r"C:\new", true, 0, 0),
            fact(r"C:\new\deep", true, 0, 0),
            fact(r"C:\new\deep\x.log", false, 3, 3),
        ];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.added, 3);
        assert_eq!(stats.unresolved, 0);
        let root = new.root().unwrap();
        let new_dir = child(&new, root, "new");
        let deep = child(&new, new_dir, "deep");
        assert_eq!(new.node(child(&new, deep, "x.log")).agg_logical(), 3);
        assert_eq!(new.node(root).agg_logical(), 133);
    }

    #[test]
    fn case_only_rename_keeps_the_tree_shape() {
        let old = base();
        let changes = [
            removed(r"C:\docs\a.txt"),
            fact(r"C:\docs\A.TXT", false, 10, 10),
        ];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.updated, 1, "same node, new spelling");
        assert_eq!(stats.added, 0);
        assert_eq!(stats.removed, 0);
        let docs = child(&new, new.root().unwrap(), "docs");
        assert!(new.find_child(docs, "A.TXT").is_some());
        assert_eq!(new.node(docs).agg_logical(), 30);
    }

    #[test]
    fn delete_then_create_drops_stale_children() {
        let old = base();
        let changes = [
            removed(r"C:\docs"),
            fact(r"C:\docs", true, 0, 0),
            fact(r"C:\docs\fresh.txt", false, 4, 4),
        ];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.removed, 4);
        assert_eq!(stats.added, 2);
        let docs = child(&new, new.root().unwrap(), "docs");
        assert!(new.find_child(docs, "a.txt").is_none());
        assert!(new.find_child(docs, "fresh.txt").is_some());
    }

    #[test]
    fn file_and_folder_swap_places() {
        let old = base();
        let changes = [
            removed(r"C:\docs\sub"),
            fact(r"C:\docs\sub", false, 15, 15),
            removed(r"C:\media\c.bin"),
            fact(r"C:\media\c.bin", true, 0, 0),
            fact(r"C:\media\c.bin\inner.txt", false, 2, 2),
        ];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert!(stats.removed >= 3);
        let docs = child(&new, new.root().unwrap(), "docs");
        let sub = child(&new, docs, "sub");
        assert_eq!(new.node(sub).flags & flags::DIR, 0);
        assert_eq!(new.node(sub).agg_logical(), 15);

        let media = child(&new, new.root().unwrap(), "media");
        let c_bin = child(&new, media, "c.bin");
        assert_ne!(new.node(c_bin).flags & flags::DIR, 0);
        assert_eq!(new.node(child(&new, c_bin, "inner.txt")).agg_logical(), 2);
    }

    #[test]
    fn unresolved_facts_do_not_corrupt_the_tree() {
        let old = base();
        let changes = [fact(r"C:\missing\child.txt", false, 9, 9)];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.unresolved, 1);
        assert_eq!(stats.added, 0);
        assert_eq!(total(&new), (130, 130));
        assert!(new.find_child(new.root().unwrap(), "missing").is_none());
    }

    #[test]
    fn outside_root_is_refused() {
        let old = base();
        let err = apply_changes(&old, r"C:\", &[fact(r"D:\docs\x", false, 1, 1)]).unwrap_err();
        assert!(matches!(err, RefreshError::OutsideRoot { .. }));
    }

    #[test]
    fn bad_component_is_refused() {
        let old = base();
        let err = apply_changes(&old, r"C:\", &[fact(r"C:\docs\..\x", false, 1, 1)]).unwrap_err();
        assert!(matches!(err, RefreshError::BadPath { .. }));
    }

    #[test]
    fn case_insensitive_root_and_prefix() {
        let old = base();
        let (new, stats) =
            apply_changes(&old, r"c:\", &[fact(r"C:\DOCS\SUB\B.TXT", false, 30, 30)]).unwrap();
        assert_eq!(stats.updated, 1);
        assert_eq!(total(&new), (140, 140));
    }

    #[test]
    fn long_path_prefix_is_tolerated() {
        let old = base();
        let (new, stats) = apply_changes(
            &old,
            r"\\?\C:\",
            &[fact(r"\\?\C:\docs\a.txt", false, 11, 16)],
        )
        .unwrap();
        assert_eq!(stats.updated, 1);
        assert_eq!(total(&new), (131, 136));
    }

    #[test]
    fn empty_tree_is_refused() {
        let empty = Tree::new();
        assert!(matches!(
            apply_changes(&empty, r"C:\", &[]),
            Err(RefreshError::EmptyTree)
        ));
    }

    #[test]
    fn unmatched_removals_are_ignored_not_counted() {
        let old = base();
        let changes = [removed(r"C:\docs\not-there.txt"), removed(r"C:\gone")];
        let (new, stats) = apply_changes(&old, r"C:\", &changes).unwrap();
        assert_eq!(stats.unresolved, 0, "nothing to place: the tree matches");
        assert_eq!(stats.removed, 0);
        assert_eq!(new.len(), old.len());
    }
}
