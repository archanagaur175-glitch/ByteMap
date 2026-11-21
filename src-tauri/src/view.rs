//! Thin adapter: builds views and orbit layouts from the session tree.
//! The heavy lifting (top-N, folding, ancestor chain, sunburst, binary
//! formats) lives in bytemap-core.

use bytemap_core::categorizer::CategoryMap;
use bytemap_core::{
    build_orbit, build_strata, build_view, diff_snapshots, serialize_diff, serialize_orbit,
    serialize_strata, serialize_view, DiffError, Snapshot, Tree,
};

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

/// Serialize the Strata (layered treemap) layout for `node_id` into the
/// packed binary format documented in bytemap-core::strata.
pub fn strata_bytes(
    tree: &Tree,
    cats: &CategoryMap,
    node_id: u32,
    depth: u16,
    max_rects: u32,
) -> Vec<u8> {
    serialize_strata(&build_strata(tree, cats, node_id, depth, max_rects))
}

/// Compare two snapshots by path and serialize the result (format documented
/// in bytemap-core::diff). Refused when the roots differ.
pub fn diff_bytes(old: &Snapshot, new: &Snapshot, max_entries: u32) -> Result<Vec<u8>, DiffError> {
    Ok(serialize_diff(&diff_snapshots(
        old,
        new,
        max_entries as usize,
    )?))
}
