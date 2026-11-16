//! Thin adapter: builds views and orbit layouts from the session tree.
//! The heavy lifting (top-N, folding, ancestor chain, sunburst, binary
//! formats) lives in bytemap-core.

use bytemap_core::categorizer::CategoryMap;
use bytemap_core::{build_orbit, build_view, serialize_orbit, serialize_view, Tree};

/// Serialize the view for `node_id` into the packed binary format
/// documented in bytemap-core::view.
pub fn view_bytes(tree: &Tree, cats: &CategoryMap, node_id: u32, limit: u32) -> Vec<u8> {
    serialize_view(&build_view(tree, cats, node_id, limit))
}

/// Serialize the orbit (sunburst) layout for `node_id` into the packed
/// binary format documented in bytemap-core::orbit.
pub fn orbit_bytes(
    tree: &Tree,
    cats: &CategoryMap,
    node_id: u32,
    rings: u16,
    max_arcs: u32,
) -> Vec<u8> {
    serialize_orbit(&build_orbit(tree, cats, node_id, rings, max_arcs))
}
