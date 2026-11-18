//! Strata (layered treemap) layout: area = share of the parent's aggregate
//! size, nested `max_depth` levels deep inside the current node. Layout is
//! computed in Rust (PROJECT_BRIEF); the UI receives packed little-endian
//! bytes:
//!
//! ```text
//! u16 version (= 1)
//! u32 center_id
//! u64 center_logical, u64 center_on_disk
//! u16 max_depth
//! u32 rect_count
//! per rect: u32 id, u16 depth (1 = first layer),
//!           u32 x, u32 y, u32 w, u32 h   (units of a CANVAS x CANVAS square)
//!           u64 logical, u64 on_disk, u16 flags, u8 category, u8 tier,
//!           u16 name_len, name bytes
//! ```
//!
//! Coordinates live in a fixed normalized canvas ([`CANVAS`] units), so a
//! window resize never re-runs the layout: the UI scales units to pixels.
//! Parents always appear before their children, and every child rectangle
//! lies inside its parent's — gaps left by tiles too small to draw show the
//! parent's color, never empty space.
//!
//! Squarified layout (Bruls, Huizing, van Wijk): each parent's children are
//! sorted by size descending, packed row by row along the shorter side,
//! keeping tiles as close to square as the sizes allow.

use crate::categorizer::CategoryMap;
use crate::rollup::children_by_size;
use crate::tree::{flags, NodeId, Tree};

/// v1: the Strata treemap (Phase 7).
pub const STRATA_VERSION: u16 = 1;

/// The normalized canvas: every rectangle is expressed in units of a
/// `CANVAS x CANVAS` square, whatever the real pixel size is.
pub const CANVAS: u32 = 10_000;

/// Hard cap on emitted rectangles (brief perf gate: 60 fps at <= 4000
/// drawn primitives; the budget is shared across all layers).
pub const MAX_RECTS: u32 = 4000;

/// A tile thinner than this (in canvas units) is not emitted; because
/// children are sorted by size, everything after it is smaller too.
const MIN_SIDE: f64 = 1.0;

/// At most this many children per parent get a tile. Content behind the
/// cap is real but not drawn (the parent's color shows through), keeping
/// the primitive budget under control on directories with 10k children.
const MAX_CHILDREN: usize = 32;

/// One drawn tile of the treemap.
#[derive(Debug, Clone)]
pub struct StrataRect {
    pub id: u32,
    /// 1 = child of the center, 2 = grandchild, ...
    pub depth: u16,
    /// Canvas units, `0..=CANVAS`.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub logical: u64,
    pub on_disk: u64,
    pub flags: u16,
    /// Category id (categorizer::category::*).
    pub cat: u8,
    /// Safety tier id (categorizer::tier::*).
    pub tier: u8,
    pub name: String,
}

/// A whole treemap for one center node.
#[derive(Debug, Clone)]
pub struct Strata {
    pub center: NodeId,
    pub center_logical: u64,
    pub center_on_disk: u64,
    pub max_depth: u16,
    pub rects: Vec<StrataRect>,
}

/// Build the Strata layout for `id`, nesting at most `max_depth` layers
/// and emitting at most `max_rects` tiles (0 = default [`MAX_RECTS`]).
pub fn build_strata(
    tree: &Tree,
    cats: &CategoryMap,
    id: NodeId,
    max_depth: u16,
    max_rects: u32,
) -> Strata {
    let center = tree.node(id);
    let mut strata = Strata {
        center: id,
        center_logical: center.agg_logical(),
        center_on_disk: center.agg_on_disk(),
        max_depth: max_depth.clamp(1, 4),
        rects: Vec::new(),
    };
    if strata.center_logical == 0 {
        return strata; // nothing to proportion
    }
    let budget = match max_rects {
        0 => MAX_RECTS as usize,
        n => n as usize,
    };
    let mut builder = StrataBuilder {
        tree,
        cats,
        max_depth: strata.max_depth,
        budget,
        out: &mut strata.rects,
    };
    builder.lay_out(id, 1, Rect::full());
    strata
}

/// Floating-point rectangle while laying out; converted to canvas units
/// (integers) only when a tile is emitted.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn full() -> Self {
        Rect {
            x: 0.0,
            y: 0.0,
            w: CANVAS as f64,
            h: CANVAS as f64,
        }
    }
}

/// Carries the immutable layout context through the recursion.
struct StrataBuilder<'a, 'b> {
    tree: &'a Tree,
    cats: &'a CategoryMap,
    max_depth: u16,
    budget: usize,
    out: &'b mut Vec<StrataRect>,
}

impl StrataBuilder<'_, '_> {
    /// Lay out the children of `parent` inside `rect`, then recurse into
    /// directories while the depth allows.
    fn lay_out(&mut self, parent: NodeId, depth: u16, rect: Rect) {
        if depth > self.max_depth || self.out.len() >= self.budget {
            return;
        }
        let children = children_by_size(self.tree, parent);
        if children.is_empty() {
            return;
        }
        let sizes: Vec<u64> = children
            .iter()
            .map(|&child| self.tree.node(child).agg_logical())
            .collect();
        let total: u64 = sizes.iter().sum();
        if total == 0 {
            return;
        }
        let rects = squarify(rect, &sizes, total);
        for (at, &child) in children.iter().enumerate() {
            if at >= MAX_CHILDREN || self.out.len() >= self.budget {
                return;
            }
            let tile = rects[at];
            if tile.w.round() < MIN_SIDE || tile.h.round() < MIN_SIDE {
                return; // sorted by size: the rest are smaller still
            }
            let node = self.tree.node(child);
            self.out.push(StrataRect {
                id: child,
                depth,
                x: tile.x.round().clamp(0.0, CANVAS as f64) as u32,
                y: tile.y.round().clamp(0.0, CANVAS as f64) as u32,
                w: tile.w.round().clamp(0.0, CANVAS as f64) as u32,
                h: tile.h.round().clamp(0.0, CANVAS as f64) as u32,
                logical: node.agg_logical(),
                on_disk: node.agg_on_disk(),
                flags: node.flags(),
                cat: self.cats.cat.get(child as usize).copied().unwrap_or(0),
                tier: self.cats.tier.get(child as usize).copied().unwrap_or(0),
                name: self.tree.name(child).to_string(),
            });
            if node.flags() & flags::DIR != 0 && depth < self.max_depth {
                self.lay_out(child, depth + 1, tile);
            }
        }
    }
}

/// Squarified layout of `sizes` (descending) inside `rect`; returns one
/// rectangle per size, same order. Areas are proportional to their share
/// of `total`.
fn squarify(rect: Rect, sizes: &[u64], total: u64) -> Vec<Rect> {
    let mut out = Vec::with_capacity(sizes.len());
    if sizes.is_empty() || total == 0 {
        return out;
    }
    let total_area = rect.w * rect.h;
    let areas: Vec<f64> = sizes
        .iter()
        .map(|&size| total_area * (size as f64) / (total as f64))
        .collect();

    let mut free = rect;
    let mut at = 0usize;
    while at < areas.len() {
        let side = free.w.min(free.h);
        if side <= 0.0 {
            break; // degenerate: the rest are zero-area slivers
        }
        // Grow the row while it keeps the worst aspect ratio from worsening.
        let mut row_sum = 0.0;
        let mut worst = f64::INFINITY;
        let mut end = at;
        while end < areas.len() {
            let next_sum = row_sum + areas[end];
            let ratio = worst_ratio(&areas[at..=end], next_sum, side);
            if end == at || ratio <= worst {
                row_sum = next_sum;
                worst = ratio;
                end += 1;
            } else {
                break;
            }
        }
        if end == at {
            break; // no progress; unreachable with positive areas
        }

        let thickness = row_sum / side;
        if free.w >= free.h {
            // Vertical strip on the left; tiles stack top to bottom.
            let mut y = free.y;
            for &area in &areas[at..end] {
                let h = area / thickness;
                out.push(Rect {
                    x: free.x,
                    y,
                    w: thickness,
                    h,
                });
                y += h;
            }
            free.x += thickness;
            free.w -= thickness;
        } else {
            // Horizontal strip at the top; tiles run left to right.
            let mut x = free.x;
            for &area in &areas[at..end] {
                let w = area / thickness;
                out.push(Rect {
                    x,
                    y: free.y,
                    w,
                    h: thickness,
                });
                x += w;
            }
            free.y += thickness;
            free.h -= thickness;
        }
        at = end;
    }
    // Zero-area or degenerate leftovers still need an entry so the caller
    // can pair rectangles with children by index; they are invisible.
    while out.len() < sizes.len() {
        out.push(Rect {
            x: free.x,
            y: free.y,
            w: 0.0,
            h: 0.0,
        });
    }
    out
}

/// Worst tile aspect ratio for `row` laid along `side` (Bruls et al.):
/// `max(side^2 * max / sum^2, sum^2 / (side^2 * min))`.
fn worst_ratio(row: &[f64], sum: f64, side: f64) -> f64 {
    let mut max = f64::MIN;
    let mut min = f64::MAX;
    for &area in row {
        max = max.max(area);
        min = min.min(area);
    }
    if sum <= 0.0 || min <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = side * side;
    let sum2 = sum * sum;
    (s2 * max / sum2).max(sum2 / (s2 * min))
}

/// Serialize a [`Strata`] in the binary format documented above.
pub fn serialize_strata(strata: &Strata) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, STRATA_VERSION);
    put_u32(&mut out, strata.center);
    put_u64(&mut out, strata.center_logical);
    put_u64(&mut out, strata.center_on_disk);
    put_u16(&mut out, strata.max_depth);
    put_u32(&mut out, strata.rects.len() as u32);
    for rect in &strata.rects {
        put_u32(&mut out, rect.id);
        put_u16(&mut out, rect.depth);
        put_u32(&mut out, rect.x);
        put_u32(&mut out, rect.y);
        put_u32(&mut out, rect.w);
        put_u32(&mut out, rect.h);
        put_u64(&mut out, rect.logical);
        put_u64(&mut out, rect.on_disk);
        put_u16(&mut out, rect.flags);
        out.push(rect.cat);
        out.push(rect.tier);
        put_str(&mut out, &rect.name);
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

    use crate::categorizer::{categorize, Rules};

    fn cats(t: &Tree) -> CategoryMap {
        categorize(t, &Rules::default_rules())
    }

    /// root 900: a 400 (dir) -> a1 200 (dir) -> a1x 200; a2 200; b 300; c 200
    fn build() -> Tree {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let a = t.insert_child(root, "a", flags::DIR, 0, 0, 0);
        let a1 = t.insert_child(a, "a1", flags::DIR, 0, 0, 0);
        t.insert_child(a1, "a1x", 0, 0, 200, 200);
        t.insert_child(a, "a2", 0, 0, 200, 200);
        t.insert_child(root, "b", 0, 0, 300, 300);
        t.insert_child(root, "c", 0, 0, 200, 200);
        crate::rollup::recompute_rollups(&mut t);
        t
    }

    fn by_name<'a>(strata: &'a Strata, name: &str) -> &'a StrataRect {
        strata
            .rects
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("rect {name} missing"))
    }

    #[test]
    fn layer_one_areas_are_proportional() {
        let t = build();
        let root = t.root().unwrap();
        let s = build_strata(&t, &cats(&t), root, 1, 0);
        assert_eq!(s.center_logical, 900, "dirs aggregate their children");
        assert_eq!(s.max_depth, 1);
        // Areas of layer-1 tiles: 4/9, 3/9, 2/9 of the canvas.
        let canvas_area = (CANVAS * CANVAS) as f64;
        for (name, share) in [("a", 4.0 / 9.0), ("b", 3.0 / 9.0), ("c", 2.0 / 9.0)] {
            let r = by_name(&s, name);
            let area = (r.w as f64) * (r.h as f64);
            let expected = canvas_area * share;
            let error = (area - expected).abs() / expected;
            assert!(
                error < 0.01,
                "{name}: area {area} vs {expected} ({:.3}% off)",
                error * 100.0
            );
        }
    }

    #[test]
    fn largest_child_is_first_and_every_layer_covers_the_canvas() {
        let t = build();
        let root = t.root().unwrap();
        let s = build_strata(&t, &cats(&t), root, 1, 0);
        assert_eq!(s.rects[0].name, "a", "descending size order");
        let total: f64 = s.rects.iter().map(|r| (r.w as f64) * (r.h as f64)).sum();
        // Tiles plus rounding must fill the square (no clipped/overlapping).
        let canvas = (CANVAS * CANVAS) as f64;
        assert!(
            (total - canvas).abs() / canvas < 0.01,
            "tiles fill {total} of {canvas}"
        );
    }

    #[test]
    fn children_stay_inside_their_parent() {
        let t = build();
        let root = t.root().unwrap();
        let s = build_strata(&t, &cats(&t), root, 3, 0);
        assert!(s.rects.iter().any(|r| r.depth == 3), "nested depth 3");
        for rect in &s.rects {
            // Walk up to the nearest ancestor rect (same id chain).
            let mut parent = t.parent(rect.id);
            let mut found = None;
            while let Some(p) = parent {
                if let Some(pr) = s.rects.iter().find(|r| r.id == p) {
                    found = Some(pr);
                    break;
                }
                parent = t.parent(p);
            }
            let Some(pr) = found else { continue }; // depth-1 tiles
            assert!(
                rect.x >= pr.x
                    && rect.y >= pr.y
                    && rect.x + rect.w <= pr.x + pr.w
                    && rect.y + rect.h <= pr.y + pr.h,
                "{} ({},{},{},{}) escapes {} ({},{},{},{})",
                rect.name,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                pr.name,
                pr.x,
                pr.y,
                pr.w,
                pr.h
            );
        }
    }

    #[test]
    fn depth_cap_budget_and_leaf_stop() {
        let t = build();
        let root = t.root().unwrap();
        let flat = build_strata(&t, &cats(&t), root, 1, 0);
        assert!(flat.rects.iter().all(|r| r.depth == 1));
        let capped = build_strata(&t, &cats(&t), root, 4, 3);
        assert_eq!(capped.rects.len(), 3, "budget respected");
        // A file center has no children: header only.
        let file = t
            .children(root)
            .find(|&c| t.name(c) == "b")
            .expect("fixture file b");
        let none = build_strata(&t, &cats(&t), file, 3, 0);
        assert!(none.rects.is_empty());
    }

    #[test]
    fn zero_size_children_do_not_emit_tiles() {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        t.insert_child(root, "data", 0, 0, 100, 100);
        t.insert_child(root, "empty", flags::DIR, 0, 0, 0);
        crate::rollup::recompute_rollups(&mut t);
        let s = build_strata(&t, &cats(&t), root, 2, 0);
        assert_eq!(s.rects.len(), 1);
        assert_eq!(s.rects[0].name, "data");
    }

    #[test]
    fn layout_is_deterministic() {
        let t = build();
        let root = t.root().unwrap();
        let a = serialize_strata(&build_strata(&t, &cats(&t), root, 3, 0));
        let b = serialize_strata(&build_strata(&t, &cats(&t), root, 3, 0));
        assert_eq!(a, b);
    }

    #[test]
    fn serialize_roundtrip() {
        let t = build();
        let root = t.root().unwrap();
        let s = build_strata(&t, &cats(&t), root, 1, 0);
        let bytes = serialize_strata(&s);
        let d = &mut Decoder::new(&bytes);
        assert_eq!(d.u16(), STRATA_VERSION);
        assert_eq!(d.u32(), root);
        assert_eq!(d.u64(), 900);
        assert_eq!(d.u64(), 900);
        assert_eq!(d.u16(), 1); // max_depth
        let count = d.u32();
        assert_eq!(count as usize, s.rects.len());
        let first = &s.rects[0];
        assert_eq!(d.u32(), first.id);
        assert_eq!(d.u16(), 1); // depth
        assert_eq!(d.u32(), first.x);
        assert_eq!(d.u32(), first.y);
        assert_eq!(d.u32(), first.w);
        assert_eq!(d.u32(), first.h);
        assert_eq!(d.u64(), first.logical);
        assert_eq!(d.u64(), first.on_disk);
        assert_eq!(d.u16(), first.flags);
        assert_eq!(d.u8(), first.cat);
        assert_eq!(d.u8(), first.tier);
        assert_eq!(d.str(), first.name);
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
            let len = self.u16() as usize;
            String::from_utf8(self.take(len).to_vec()).unwrap()
        }
    }
}
