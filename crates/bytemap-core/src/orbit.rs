//! Orbit (sunburst) layout: center = current folder, rings = depth,
//! arc angle = share of the parent's aggregate size. Layout is computed in
//! Rust (PROJECT_BRIEF); the UI receives packed little-endian bytes:
//!
//! ```text
//! u16 version (= 1)
//! u32 center_id
//! u64 center_logical, u64 center_on_disk
//! u16 ring_count (layout depth limit, not arcs len)
//! u32 arc_count
//! per arc: u32 id, u16 depth (1 = first ring), u16 start, u16 sweep,
//!          u64 logical, u64 on_disk, u16 flags, u16 name_len, name bytes
//! ```
//!
//! Angles are fixed point: 65536 = one full turn (2*pi rad). A sweep below
//! [`MIN_SWEEP`] is skipped entirely (invisible sliver); the arc budget caps
//! the message at [`MAX_ARCS`] entries (brief: <= 4000 drawn primitives).

use crate::rollup::children_by_size;
use crate::tree::{flags, NodeId, Tree};

pub const ORBIT_VERSION: u16 = 1;

/// One full turn in fixed point (65536 -> 2*pi).
pub const TURN: u32 = 65536;

/// Minimum sweep to draw, fixed point. 16/65536 of a turn ~ 0.09 degrees.
pub const MIN_SWEEP: u32 = 16;

/// Hard cap on emitted arcs (brief perf gate: 60 fps at <= 4000 primitives).
pub const MAX_ARCS: u32 = 4000;

/// One drawn wedge of the sunburst.
#[derive(Debug, Clone)]
pub struct OrbitArc {
    pub id: u32,
    /// 1 = child of the center, 2 = grandchild, ...
    pub depth: u16,
    /// Fixed-point start angle (0..65536 = full turn).
    pub start: u16,
    /// Fixed-point angular width.
    pub sweep: u16,
    pub logical: u64,
    pub on_disk: u64,
    pub flags: u16,
    pub name: String,
}

/// A whole orbit layout for one center node.
#[derive(Debug, Clone)]
pub struct Orbit {
    pub center: NodeId,
    pub center_logical: u64,
    pub center_on_disk: u64,
    pub rings: u16,
    pub arcs: Vec<OrbitArc>,
}

/// Build the orbit layout for `id`, descending at most `rings` levels,
/// emitting at most `max_arcs` wedges (0 = default [`MAX_ARCS`]).
///
/// Children of a parent are laid out in descending size order, each taking
/// `parent_sweep * child_agg / parent_agg` of the parent's angular span.
pub fn build_orbit(tree: &Tree, id: NodeId, rings: u16, max_arcs: u32) -> Orbit {
    let center = tree.node(id);
    let mut orbit = Orbit {
        center: id,
        center_logical: center.agg_logical(),
        center_on_disk: center.agg_on_disk(),
        rings: rings.max(1),
        arcs: Vec::new(),
    };
    let budget = match max_arcs {
        0 => MAX_ARCS as usize,
        n => n as usize,
    };
    if orbit.center_logical == 0 {
        return orbit; // nothing to proportion
    }
    let mut builder = OrbitBuilder {
        tree,
        rings: orbit.rings,
        budget,
        out: &mut orbit.arcs,
    };
    builder.lay_out(id, 1, 0, TURN as u64);
    orbit
}

/// Carries the immutable layout context through the recursion.
struct OrbitBuilder<'a, 'b> {
    tree: &'a Tree,
    rings: u16,
    budget: usize,
    out: &'b mut Vec<OrbitArc>,
}

impl OrbitBuilder<'_, '_> {
    /// Lay out one ring: children of `parent` inside
    /// [start, start + sweep) in descending-size order.
    fn lay_out(&mut self, parent: NodeId, depth: u16, start: u64, sweep: u64) {
        if depth > self.rings || self.out.len() >= self.budget || sweep == 0 {
            return;
        }
        let parent_agg = self.tree.node(parent).agg_logical();
        if parent_agg == 0 {
            return;
        }
        let mut cursor = start;
        for child in children_by_size(self.tree, parent) {
            if self.out.len() >= self.budget {
                return;
            }
            let n = self.tree.node(child);
            let child_sweep = sweep * n.agg_logical() / parent_agg;
            let keep = child_sweep >= MIN_SWEEP as u64;
            if keep {
                self.out.push(OrbitArc {
                    id: child,
                    depth,
                    start: (cursor % TURN as u64) as u16,
                    sweep: child_sweep.min(TURN as u64 - 1) as u16,
                    logical: n.agg_logical(),
                    on_disk: n.agg_on_disk(),
                    flags: n.flags(),
                    name: self.tree.name(child).to_string(),
                });
                if n.flags() & flags::DIR != 0 && depth < self.rings {
                    self.lay_out(child, depth + 1, cursor, child_sweep);
                }
            }
            cursor += child_sweep;
        }
    }
}

/// Serialize an [`Orbit`] in the binary format documented above.
pub fn serialize_orbit(orbit: &Orbit) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, ORBIT_VERSION);
    put_u32(&mut out, orbit.center);
    put_u64(&mut out, orbit.center_logical);
    put_u64(&mut out, orbit.center_on_disk);
    put_u16(&mut out, orbit.rings);
    put_u32(&mut out, orbit.arcs.len() as u32);
    for arc in &orbit.arcs {
        put_u32(&mut out, arc.id);
        put_u16(&mut out, arc.depth);
        put_u16(&mut out, arc.start);
        put_u16(&mut out, arc.sweep);
        put_u64(&mut out, arc.logical);
        put_u64(&mut out, arc.on_disk);
        put_u16(&mut out, arc.flags);
        put_str(&mut out, &arc.name);
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

    /// root 1000: a 500 (dir) -> a1 400, a2 100; b 300; c 200
    fn build() -> Tree {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let a = t.insert_child(root, "a", flags::DIR, 0, 0, 0);
        t.insert_child(a, "a1", 0, 0, 400, 400);
        t.insert_child(a, "a2", 0, 0, 100, 100);
        t.insert_child(root, "b", 0, 0, 300, 300);
        t.insert_child(root, "c", 0, 0, 200, 200);
        crate::rollup::recompute_rollups(&mut t);
        t
    }

    fn find<'a>(orbit: &'a Orbit, name: &str) -> &'a OrbitArc {
        orbit
            .arcs
            .iter()
            .find(|arc| arc.name == name)
            .unwrap_or_else(|| panic!("arc {name} missing"))
    }

    #[test]
    fn ring1_angles_proportional_and_ordered() {
        let t = build();
        let root = t.root().unwrap();
        let orbit = build_orbit(&t, root, 2, 0);
        assert_eq!(orbit.rings, 2);
        assert_eq!(orbit.center_logical, 1000);
        // Ring 1: a(1/2 turn), b(3/10), c(1/5), laid out sequentially.
        let a = find(&orbit, "a");
        let b = find(&orbit, "b");
        let c = find(&orbit, "c");
        assert_eq!(a.start, 0);
        assert_eq!(a.sweep as u32, TURN / 2);
        assert_eq!(b.start as u32, TURN / 2);
        assert_eq!(b.sweep as u32, 3 * TURN / 10);
        assert_eq!(c.start as u32, TURN / 2 + 3 * TURN / 10);
    }

    #[test]
    fn ring2_splits_parent_arc() {
        let t = build();
        let root = t.root().unwrap();
        let orbit = build_orbit(&t, root, 2, 0);
        // a1 and a2 live inside a's arc: a takes 80% of it.
        let a1 = find(&orbit, "a1");
        let a2 = find(&orbit, "a2");
        assert_eq!(a1.depth, 2);
        assert_eq!(a1.start, 0);
        assert_eq!(a1.sweep as u32, TURN / 2 * 4 / 5); // 400 of a's 500
        assert_eq!(a2.depth, 2);
        assert_eq!(a2.start as u32, TURN / 2 * 4 / 5); // after a1
        assert_eq!(a2.sweep as u32, TURN / 2 / 5); // 100 of a's 500
    }

    #[test]
    fn tiny_arcs_skipped_children_still_advance_cursor() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        t.insert_child(root, "big", 0, 0, 1_000_000, 1_000_000);
        t.insert_child(root, "tiny", 0, 0, 1, 1); // sweep 0 < MIN_SWEEP
        crate::rollup::recompute_rollups(&mut t);
        let orbit = build_orbit(&t, root, 1, 0);
        assert_eq!(orbit.arcs.len(), 1, "tiny arc below MIN_SWEEP skipped");
        assert_eq!(orbit.arcs[0].name, "big");
        // big's sweep is proportional to the parent total, not inflated.
        assert_eq!(
            orbit.arcs[0].sweep as u64,
            TURN as u64 * 1_000_000 / 1_000_001
        );
    }

    #[test]
    fn arc_budget_capped() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        for i in 0..50 {
            t.insert_child(root, &format!("f{i:02}"), 0, 0, 100 - i as u64, 100 - i as u64);
        }
        crate::rollup::recompute_rollups(&mut t);
        let orbit = build_orbit(&t, root, 1, 7);
        assert_eq!(orbit.arcs.len(), 7);
    }

    #[test]
    fn serialize_roundtrip() {
        let t = build();
        let root = t.root().unwrap();
        let orbit = build_orbit(&t, root, 2, 0);
        let bytes = serialize_orbit(&orbit);
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.u16(), ORBIT_VERSION);
        assert_eq!(d.u32(), root);
        assert_eq!(d.u64(), 1000);
        assert_eq!(d.u64(), 1000);
        assert_eq!(d.u16(), 2);
        let count = d.u32() as usize;
        assert_eq!(count, orbit.arcs.len());
        let mut prev_end = 0u64;
        for arc in &orbit.arcs {
            assert_eq!(d.u32(), arc.id);
            assert_eq!(d.u16(), arc.depth);
            let start = d.u16() as u64;
            let sweep = d.u16() as u64;
            assert_eq!(start, arc.start as u64);
            assert_eq!(sweep, arc.sweep as u64);
            d.u64();
            d.u64();
            d.u16();
            assert_eq!(d.str(), arc.name);
            if arc.depth == 1 {
                assert!(start >= prev_end, "ring-1 arcs are sequential");
                prev_end = start + sweep;
            }
        }
        assert!(d.at_end());
    }

    #[test]
    fn empty_center_yields_no_arcs() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let orbit = build_orbit(&t, root, 3, 0);
        assert!(orbit.arcs.is_empty());
        assert_eq!(orbit.center_logical, 0);
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
