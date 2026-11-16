//! Node id -> absolute path resolution. The frontend sends node IDs only;
//! paths are resolved here (PROJECT_BRIEF safety rule).

use std::path::{Path, PathBuf};

use bytemap_core::{NodeId, Tree};

/// Absolute path of `id`: the scan root + names along the parent chain.
/// The root node's own name is already the last component of `root`
/// (walker names the root after the scan folder), so it is not appended.
/// Returns `None` for out-of-range ids and for nodes not under the root.
pub fn node_path(tree: &Tree, root: &Path, id: NodeId) -> Option<PathBuf> {
    let root_id = tree.root()?;
    if id as usize >= tree.len() {
        return None;
    }
    if id == root_id {
        return Some(root.to_path_buf());
    }
    let mut names: Vec<String> = Vec::new();
    let mut cur = Some(id);
    while let Some(x) = cur {
        if x == root_id {
            names.reverse();
            let mut p = root.to_path_buf();
            for n in names {
                p.push(n);
            }
            return Some(p);
        }
        names.push(tree.name(x).to_string());
        cur = tree.parent(x);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemap_core::tree::flags;

    fn tree() -> Tree {
        let mut t = Tree::new();
        let root = t.insert_root("fixture");
        let sub = t.insert_child(root, "sub", flags::DIR, 0, 0, 0);
        t.insert_child(sub, "a.txt", 0, 0, 1, 1);
        t
    }

    #[test]
    fn resolves_nested_paths() {
        let t = tree();
        let root = PathBuf::from(r"C:\tmp\fixture");
        // root=0, sub=1, a.txt=2 (insert order)
        assert_eq!(
            node_path(&t, &root, 0),
            Some(root.clone()),
            "root resolves to the scan root itself"
        );
        assert_eq!(node_path(&t, &root, 1), Some(root.join("sub")));
        assert_eq!(node_path(&t, &root, 2), Some(root.join("sub").join("a.txt")));
    }

    #[test]
    fn out_of_range_and_orphans_are_none() {
        let t = tree();
        let root = PathBuf::from(r"C:\tmp\fixture");
        assert_eq!(node_path(&t, &root, 99), None);
        // Rootless tree: node ids exist but no root anchor.
        assert_eq!(node_path(&Tree::new(), &root, 0), None);
    }
}
