//! bytemap-core: arena tree, roll-ups, categorizer, and layout for ByteMap.

pub mod orbit;
pub mod rollup;
pub mod tree;
pub mod view;

pub use orbit::{
    build_orbit, serialize_orbit, Orbit, OrbitArc, MAX_ARCS, MIN_SWEEP, ORBIT_VERSION, TURN,
};
pub use rollup::{children_by_size, recompute_rollups};
pub use tree::{Children, Node, NodeId, StringArena, Tree, NONE};
pub use view::{build_view, serialize_view, View, ViewItem, FOLDED_ID, VIEW_FOLDED, VIEW_VERSION};

/// Placeholder version marker.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
