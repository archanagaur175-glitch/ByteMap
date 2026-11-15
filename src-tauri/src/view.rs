//! Thin adapter: builds a binary view from the session tree.
//! The heavy lifting (top-N, folding, ancestor chain, binary format)
//! lives in bytemap-core::view.

use bytemap_core::{build_view, serialize_view, Tree};

/// Serialize the view for `node_id` into the packed binary format
/// documented in bytemap-core::view.
pub fn view_bytes(tree: &Tree, node_id: u32, limit: u32) -> Vec<u8> {
    serialize_view(&build_view(tree, node_id, limit))
}
