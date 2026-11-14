//! Roll-ups: aggregate logical and on-disk sizes bottom-up, and child views
//! sorted by size (the basis of the "top N children" IPC view in Phase 2).

use crate::tree::{flags, NodeId, Tree, NONE};

/// Recompute the aggregate sizes of every directory, bottom-up.
///
/// Call once after bulk construction (scanner inserts do not maintain parent
/// aggregates on their own). File nodes keep their own sizes; directory nodes
/// get the sum over their subtree. Directory *own* disk usage (e.g. the
/// directory entry itself) is not counted — documented in DECISIONS.md.
pub fn recompute_rollups(tree: &mut Tree) {
    let root = match tree.root() {
        Some(r) => r,
        None => return,
    };

    struct Frame {
        node: NodeId,
        next_child: u32,
        sum_logical: u64,
        sum_on_disk: u64,
    }

    let mut frames: Vec<Frame> = Vec::new();
    let root_first_child = tree.node(root).first_child;
    frames.push(Frame {
        node: root,
        next_child: root_first_child,
        sum_logical: 0,
        sum_on_disk: 0,
    });

    while let Some(top) = frames.last_mut() {
        let next_child = top.next_child;
        if next_child != NONE {
            let child = tree.node(next_child);
            top.next_child = child.next_sibling;
            let is_file = child.flags & flags::DIR == 0;
            let (own_logical, own_on_disk) = (child.agg_logical, child.agg_on_disk);
            if is_file {
                // Files carry their own size in the aggregate fields.
                top.sum_logical += own_logical;
                top.sum_on_disk += own_on_disk;
            } else {
                let fc = child.first_child;
                frames.push(Frame {
                    node: next_child,
                    next_child: fc,
                    sum_logical: 0,
                    sum_on_disk: 0,
                });
            }
        } else {
            let done = frames.pop().expect("stack never empty here");
            let node = tree.node_mut(done.node);
            node.agg_logical = done.sum_logical;
            node.agg_on_disk = done.sum_on_disk;
            if let Some(parent) = frames.last_mut() {
                parent.sum_logical += done.sum_logical;
                parent.sum_on_disk += done.sum_on_disk;
            }
        }
    }
}

/// Direct children of `id`, sorted by aggregate logical size, descending.
/// The scanner builds child lists in reverse insertion order, so views must
/// always sort explicitly.
pub fn children_by_size(tree: &Tree, id: NodeId) -> Vec<NodeId> {
    let mut kids: Vec<NodeId> = tree.children(id).collect();
    kids.sort_by(|&a, &b| {
        let (na, nb) = (tree.node(a), tree.node(b));
        nb.agg_logical.cmp(&na.agg_logical)
    });
    kids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Tree;

    fn build() -> Tree {
        //        C: (root)
        //        |- docs (dir)
        //        |   |- big.zip        logical 1000, disk 1024
        //        |   |- small.txt      logical 10,  disk 4096
        //        |- media (dir)
        //        |   |- video.mkv      logical 500, disk 512
        //        |- empty (dir)
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let docs = t.insert_child(root, "docs", flags::DIR, 0, 0, 0);
        t.insert_child(docs, "big.zip", 0, 0, 1000, 1024);
        t.insert_child(docs, "small.txt", 0, 0, 10, 4096);
        let media = t.insert_child(root, "media", flags::DIR, 0, 0, 0);
        t.insert_child(media, "video.mkv", 0, 0, 500, 512);
        t.insert_child(root, "empty", flags::DIR, 0, 0, 0);
        t
    }

    #[test]
    fn rollup_sums_subtree_sizes() {
        let mut t = build();
        recompute_rollups(&mut t);
        let root = t.root().unwrap();
        assert_eq!(t.node(root).agg_logical, 1510);
        assert_eq!(t.node(root).agg_on_disk, 5632);
        let docs = t.find_child(root, "docs").unwrap();
        assert_eq!(t.node(docs).agg_logical, 1010);
        assert_eq!(t.node(docs).agg_on_disk, 5120);
        let empty = t.find_child(root, "empty").unwrap();
        assert_eq!(t.node(empty).agg_logical, 0);
        // Files keep their own sizes.
        let big = t.find_child(docs, "big.zip").unwrap();
        assert_eq!(t.node(big).agg_logical, 1000);
    }

    #[test]
    fn children_by_size_descending() {
        let mut t = build();
        recompute_rollups(&mut t);
        let root = t.root().unwrap();
        let sorted = children_by_size(&t, root);
        let names: Vec<&str> = sorted.iter().map(|&id| t.name(id)).collect();
        assert_eq!(names, vec!["docs", "media", "empty"]);
    }

    #[test]
    fn rollup_empty_tree_ok() {
        let mut t = Tree::new();
        recompute_rollups(&mut t); // no root: must not panic
        assert!(t.is_empty());
    }
}
