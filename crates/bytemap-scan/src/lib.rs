//! bytemap-scan: scanner tiers for ByteMap.
//! Tier A (MFT, Phase 6), Tier B (jwalk + rayon walker), Tier C (network/removable).

pub mod error;
pub mod metadata;
#[cfg(windows)]
pub mod mft_client;
#[cfg(windows)]
pub mod mft_tree;
#[cfg(windows)]
pub mod spawn;
pub mod walker;

pub use error::ScanError;
pub use walker::{scan_folder, ScanOptions, ScanProgress, ScanStats};

/// Placeholder version marker.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
