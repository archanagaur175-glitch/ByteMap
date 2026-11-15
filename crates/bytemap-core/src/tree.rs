//! Flat arena tree: `Vec<Node>` with `u32` indices and a shared string arena.
//!
//! Design rules (PROJECT_BRIEF):
//! - Rust owns the tree; the UI never receives it, only views.
//! - Target <= 80 bytes per entry (asserted by a test).
//! - Children use an intrusive first-child / next-sibling list to keep the
//!   node struct small; child iteration is a cheap linked-list walk and
//!   "top N children by size" is gathered + sorted on demand in Rust.

/// Sentinel index meaning "no node".
pub const NONE: u32 = u32::MAX;

/// Handle to a node in the arena.
pub type NodeId = u32;

/// Node flags.
pub mod flags {
    pub const DIR: u16 = 1;
    /// Junction, symlink or any other reparse point. Never followed by scanners.
    pub const REPARSE: u16 = 2;
    /// OneDrive-style placeholder (recall on open/data access). Never hydrated.
    pub const PLACEHOLDER: u16 = 4;
    /// Enumeration or metadata of this entry was denied; sizes may be partial.
    pub const DENIED: u16 = 8;
    /// Scan found a hardlink duplicate (same file id already counted).
    pub const DUPLICATE_HARDLINK: u16 = 16;
}

/// One tree entry. Layout target: <= 80 bytes (`size_of` is asserted in tests).
#[derive(Clone, Copy, Debug)]
pub struct Node {
    /// Parent index, [`NONE`] for the root.
    pub(crate) parent: u32,
    /// First child index, [`NONE`] for leaves.
    pub(crate) first_child: u32,
    /// Next sibling index, [`NONE`] for the last child.
    pub(crate) next_sibling: u32,
    /// Number of direct children.
    pub(crate) child_count: u32,
    /// Aggregate logical size (own size for files; rolled-up sum for dirs).
    pub(crate) agg_logical: u64,
    /// Aggregate size on disk (same convention as `agg_logical`).
    pub(crate) agg_on_disk: u64,
    /// Offset of the name in the shared string arena.
    pub(crate) name_off: u32,
    /// Name length in bytes (UTF-8). Windows names are <= 255 UTF-16 units,
    /// so this always fits in 16 bits.
    pub(crate) name_len: u16,
    /// Depth below the root (root = 0).
    pub(crate) depth: u16,
    /// Combination of [`flags`] constants.
    pub(crate) flags: u16,
    /// Last write time as Windows FILETIME (100 ns ticks since 1601-01-01).
    pub(crate) mtime: i64,
}

impl Node {
    /// Combination of [`flags`] constants.
    pub fn flags(&self) -> u16 {
        self.flags
    }

    /// Last write time as Windows FILETIME (100 ns ticks since 1601-01-01).
    pub fn mtime(&self) -> i64 {
        self.mtime
    }

    /// Number of direct children.
    pub fn child_count(&self) -> u32 {
        self.child_count
    }

    /// Aggregate logical size (own size for files; rolled-up sum for dirs).
    pub fn agg_logical(&self) -> u64 {
        self.agg_logical
    }

    /// Aggregate size on disk (same convention as [`Self::agg_logical`]).
    pub fn agg_on_disk(&self) -> u64 {
        self.agg_on_disk
    }
}

const _: () = assert!(
    std::mem::size_of::<Node>() <= 80,
    "Node exceeds the 80-byte-per-entry target"
);

/// Append-only UTF-8 string arena shared by all node names.
#[derive(Default, Debug)]
pub struct StringArena {
    buf: Vec<u8>,
}

impl StringArena {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Store a string, returning its offset and byte length.
    pub fn push(&mut self, s: &str) -> (u32, u16) {
        let off = self.buf.len() as u32;
        let len = s.len();
        assert!(
            len <= u16::MAX as usize,
            "name longer than 65535 UTF-8 bytes"
        );
        self.buf.extend_from_slice(s.as_bytes());
        (off, len as u16)
    }

    /// Borrow the stored string.
    pub fn get(&self, off: u32, len: u16) -> &str {
        let start = off as usize;
        let end = start + len as usize;
        std::str::from_utf8(&self.buf[start..end]).expect("arena strings are valid UTF-8")
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// The tree. Owns all nodes and name storage.
#[derive(Default, Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    strings: StringArena,
    root: Option<NodeId>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    pub fn name(&self, id: NodeId) -> &str {
        let n = &self.nodes[id as usize];
        self.strings.get(n.name_off, n.name_len)
    }

    /// Depth below the root (root = 0).
    pub fn depth(&self, id: NodeId) -> u16 {
        self.nodes[id as usize].depth
    }

    /// Parent of `id`, or `None` for the root.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        match self.nodes[id as usize].parent {
            NONE => None,
            p => Some(p),
        }
    }

    /// Insert the root directory. Panics if a root already exists.
    pub fn insert_root(&mut self, name: &str) -> NodeId {
        assert!(self.root.is_none(), "root already inserted");
        let id = self.push_node(None, name, flags::DIR, 0);
        self.root = Some(id);
        id
    }

    /// Insert a child under `parent`. Returns the new node id.
    /// File sizes land in the aggregate fields directly; directory aggregates
    /// are computed by the roll-up pass.
    pub fn insert_child(
        &mut self,
        parent: NodeId,
        name: &str,
        node_flags: u16,
        mtime: i64,
        logical: u64,
        on_disk: u64,
    ) -> NodeId {
        let id = self.push_node(Some(parent), name, node_flags, mtime);
        if node_flags & flags::DIR == 0 {
            let node = &mut self.nodes[id as usize];
            node.agg_logical = logical;
            node.agg_on_disk = on_disk;
        }
        id
    }

    fn push_node(&mut self, parent: Option<NodeId>, name: &str, node_flags: u16, mtime: i64) -> NodeId {
        let (name_off, name_len) = self.strings.push(name);
        let depth = match parent {
            Some(p) => self.nodes[p as usize].depth + 1,
            None => 0,
        };
        assert!(
            self.nodes.len() < u32::MAX as usize,
            "arena exhausted: more than u32::MAX nodes"
        );
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            parent: parent.unwrap_or(NONE),
            first_child: NONE,
            next_sibling: NONE,
            child_count: 0,
            agg_logical: 0,
            agg_on_disk: 0,
            name_off,
            name_len,
            depth,
            flags: node_flags,
            mtime,
        });
        if let Some(p) = parent {
            let p = p as usize;
            let head = self.nodes[p].first_child;
            self.nodes[id as usize].next_sibling = head;
            self.nodes[p].first_child = id;
            self.nodes[p].child_count += 1;
        }
        id
    }

    /// Iterate the direct children of `id`.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.nodes[id as usize].first_child,
        }
    }

    /// Find a direct child by name (linear scan; used by scanners to link paths).
    pub fn find_child(&self, id: NodeId, name: &str) -> Option<NodeId> {
        self.children(id).find(|&c| self.name(c) == name)
    }
}

/// Iterator over the direct children of a node.
pub struct Children<'a> {
    tree: &'a Tree,
    next: u32,
}

impl<'a> Iterator for Children<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.next;
        if id == NONE {
            return None;
        }
        self.next = self.tree.nodes[id as usize].next_sibling;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_fits_80_byte_target() {
        assert!(
            std::mem::size_of::<Node>() <= 80,
            "Node is {} bytes, target is 80",
            std::mem::size_of::<Node>()
        );
    }

    #[test]
    fn insert_and_traverse() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let docs = t.insert_child(root, "docs", flags::DIR, 0, 0, 0);
        let file = t.insert_child(docs, "a.txt", 0, 0, 100, 4096);
        let other = t.insert_child(root, "zips", flags::DIR, 0, 0, 0);

        assert_eq!(t.root(), Some(root));
        assert_eq!(t.len(), 4);
        assert_eq!(t.name(root), "C:");
        assert_eq!(t.name(docs), "docs");
        assert_eq!(t.depth(file), 2);
        assert_eq!(t.node(file).parent, docs);
        assert_eq!(t.node(docs).child_count, 1);
        assert_eq!(t.node(root).child_count, 2);

        let kids: Vec<NodeId> = t.children(root).collect();
        // Insertion order is reversed (newest first) by the intrusive list.
        assert_eq!(kids, vec![other, docs]);
    }

    #[test]
    fn find_child_by_name() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        t.insert_child(root, "alpha", flags::DIR, 0, 0, 0);
        let beta = t.insert_child(root, "beta", flags::DIR, 0, 0, 0);
        assert_eq!(t.find_child(root, "beta"), Some(beta));
        assert_eq!(t.find_child(root, "missing"), None);
    }

    #[test]
    fn string_arena_roundtrip() {
        let mut s = StringArena::new();
        let (off, len) = s.push("hello");
        assert_eq!(s.get(off, len), "hello");
        let (off2, len2) = s.push("wörld");
        assert_eq!(s.get(off2, len2), "wörld");
        assert_eq!(s.get(off, len), "hello");
        assert!(!s.is_empty());
    }
}
