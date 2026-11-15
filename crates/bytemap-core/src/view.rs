//! Tree views: the UI never receives the whole tree (PROJECT_BRIEF).
//! A view = ancestor chain (for the breadcrumb) + top-N children by size,
//! with the rest folded into one "N small items" entry.
//!
//! The wire format is packed little-endian binary:
//! ```text
//! u16 version (= 1)
//! u16 ancestor_count
//! u32 requested_node_id
//! per ancestor:        u32 id, u16 name_len, name bytes (UTF-8)
//! u32 total_child_count   (before folding)
//! u32 item_count          (after folding)
//! per item:            u32 id (u32::MAX = folded entry), u64 logical,
//!                      u64 on_disk, u16 flags, u16 name_len, name bytes
//! ```

use crate::rollup::children_by_size;
use crate::tree::{flags, NodeId, Tree, NONE};

/// Sentinel node id used for the folded "N small items" entry.
pub const FOLDED_ID: u32 = NONE;

/// Extra node flag marking the folded entry (only exists inside views).
pub const VIEW_FOLDED: u16 = 32;

pub const VIEW_VERSION: u16 = 1;

/// One entry of a view (after folding).
#[derive(Debug, Clone)]
pub struct ViewItem {
    pub id: u32,
    pub logical: u64,
    pub on_disk: u64,
    pub flags: u16,
    pub name: String,
}

/// A full view response for one node.
#[derive(Debug, Clone)]
pub struct View {
    pub requested: NodeId,
    /// Ancestor chain from root to the parent of `requested` (ids + names),
    /// for breadcrumbs. Empty for the root.
    pub ancestors: Vec<(u32, String)>,
    pub total_children: u32,
    pub items: Vec<ViewItem>,
}

/// Build the view for `id`: ancestor chain + top `limit` children by size,
/// folding the remainder into one summary entry when folded.
pub fn build_view(tree: &Tree, id: NodeId, limit: u32) -> View {
    let mut ancestors = Vec::new();
    let mut cur = tree.parent(id);
    while let Some(p) = cur {
        ancestors.push((p, tree.name(p).to_string()));
        cur = tree.parent(p);
    }
    ancestors.reverse();

    let sorted = children_by_size(tree, id);
    let total_children = sorted.len() as u32;
    let limit = limit.max(1) as usize;
    // Folded entry consumes one slot, so a folded view shows limit-1 items.
    let folded = sorted.len() > limit;
    let top = if folded { limit - 1 } else { sorted.len() };
    let mut items: Vec<ViewItem> = Vec::new();
    for &child in &sorted[..top] {
        let n = tree.node(child);
        items.push(ViewItem {
            id: child,
            logical: n.agg_logical,
            on_disk: n.agg_on_disk,
            flags: n.flags,
            name: tree.name(child).to_string(),
        });
    }
    if folded {
        let rest = &sorted[top..];
        let logical: u64 = rest.iter().map(|&c| tree.node(c).agg_logical).sum();
        let on_disk: u64 = rest.iter().map(|&c| tree.node(c).agg_on_disk).sum();
        let n = rest.len();
        items.push(ViewItem {
            id: FOLDED_ID,
            logical,
            on_disk,
            flags: VIEW_FOLDED | flags::DIR,
            name: format!("{n} small items"),
        });
    }

    View {
        requested: id,
        ancestors,
        total_children,
        items,
    }
}

/// Serialize a [`View`] in the binary format documented above.
pub fn serialize_view(view: &View) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, VIEW_VERSION);
    put_u16(&mut out, view.ancestors.len() as u16);
    put_u32(&mut out, view.requested);
    for (id, name) in &view.ancestors {
        put_u32(&mut out, *id);
        put_str(&mut out, name);
    }
    put_u32(&mut out, view.total_children);
    put_u32(&mut out, view.items.len() as u32);
    for item in &view.items {
        put_u32(&mut out, item.id);
        put_u64(&mut out, item.logical);
        put_u64(&mut out, item.on_disk);
        put_u16(&mut out, item.flags);
        put_str(&mut out, &item.name);
    }
    out
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

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u16(out, s.len() as u16);
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Tree {
        //  root
        //   |- big1      1000
        //   |- big2      900
        //   |- small1    10
        //   |- small2    5
        //   |- small3    1
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        t.insert_child(root, "big1", 0, 0, 1000, 1000);
        t.insert_child(root, "big2", 0, 0, 900, 900);
        t.insert_child(root, "small1", 0, 0, 10, 4096);
        t.insert_child(root, "small2", 0, 0, 5, 4096);
        t.insert_child(root, "small3", 0, 0, 1, 4096);
        t
    }

    #[test]
    fn top_n_and_folded_summary() {
        let t = build();
        let root = t.root().unwrap();
        let view = build_view(&t, root, 3);
        assert_eq!(view.total_children, 5);
        assert_eq!(view.items.len(), 3);
        assert!(view.items[0].id != FOLDED_ID);
        assert_eq!(view.items[0].name, "big1");
        assert_eq!(view.items[1].name, "big2");
        let folded = &view.items[2];
        assert_eq!(folded.id, FOLDED_ID);
        assert_eq!(folded.name, "3 small items");
        assert_eq!(folded.logical, 10 + 5 + 1);
        assert_eq!(folded.on_disk, 3 * 4096);
        assert_eq!(folded.flags & VIEW_FOLDED, VIEW_FOLDED);
    }

    #[test]
    fn ancestors_chain_for_breadcrumb() {
        let mut t = build();
        let root = t.root().unwrap();
        let sub = t.insert_child(root, "sub", flags::DIR, 0, 0, 0);
        let file = t.insert_child(sub, "f.txt", 0, 0, 1, 1);
        let view = build_view(&t, file, 10);
        assert_eq!(
            view.ancestors,
            vec![(root, "C:".to_string()), (sub, "sub".to_string())]
        );
        let root_view = build_view(&t, root, 10);
        assert!(root_view.ancestors.is_empty());
    }

    #[test]
    fn serialize_roundtrip() {
        let mut t = build();
        let root = t.root().unwrap();
        let sub = t.insert_child(root, "sub", flags::DIR, 0, 0, 0);
        t.insert_child(sub, "f.txt", 0, 0, 1, 1);
        let view = build_view(&t, sub, 10);
        let bytes = serialize_view(&view);
        let d = &mut Decoder::new(&bytes);
        assert_eq!(d.u16(), VIEW_VERSION);
        assert_eq!(d.u16(), 1); // one ancestor (root)
        assert_eq!(d.u32(), sub);
        assert_eq!(d.u32(), root);
        assert_eq!(d.str(), "C:");
        assert_eq!(d.u32(), 1); // total children
        assert_eq!(d.u32(), 1); // items
        assert_eq!(d.u32(), sub + 1); // file id
        assert_eq!(d.u64(), 1);
        assert_eq!(d.u64(), 1);
        d.u16(); // flags
        assert_eq!(d.str(), "f.txt");
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
            let len = self.u16() as usize;
            String::from_utf8(self.take(len).to_vec()).unwrap()
        }
        fn at_end(&self) -> bool {
            self.pos == self.buf.len()
        }
    }
}
