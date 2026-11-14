//! bytemap-core: arena tree, roll-ups, categorizer, and layout for ByteMap.

pub mod tree;

pub use tree::{Children, Node, NodeId, StringArena, Tree, NONE};

/// Placeholder version marker.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
