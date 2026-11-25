//! Scan session state. Rust owns the tree; the UI never receives it.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use bytemap_actions::Basket;
use bytemap_core::Tree;

pub struct Session {
    /// The most recently completed scan (empty while scanning or idle).
    pub tree: Tree,
    /// Categories + tiers for `tree` (computed once per completed scan).
    pub cats: bytemap_core::categorizer::CategoryMap,
    /// Absolute scan root; node paths resolve as root + parent-chain names.
    /// Set when a scan starts; node ids are only valid for this root.
    pub root_path: PathBuf,
    /// Cleanup staging list (Action Hub). Cleared on every new scan because
    /// node ids from a previous scan are meaningless afterwards.
    pub basket: Basket,
    /// True while a scan is in flight; start_scan rejects concurrent scans.
    pub running: bool,
    /// Cooperative cancel flag for the running scan (or the next one).
    pub cancel: Arc<AtomicBool>,
    /// How the loaded tree was produced ("walker"/"mft"), for snapshots.
    /// `None` until a scan completes or a snapshot is loaded.
    pub scan_tier: Option<bytemap_core::ScanTier>,
    /// Duration of the scan that produced the tree (0 = unknown).
    pub scan_ms: u64,
    /// File name of the loaded snapshot, `None` for live scan data. The UI
    /// shows a snapshot banner while this is set.
    pub loaded_snapshot: Option<String>,
    /// USN journal cursor captured when the tree was produced (P8.T3).
    /// `None` when the tree cannot be refreshed (walker scan, folder MFT
    /// scan, or an old snapshot).
    pub usn_checkpoint: Option<bytemap_core::UsnCheckpoint>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            tree: Tree::new(),
            cats: bytemap_core::categorizer::CategoryMap {
                cat: Vec::new(),
                tier: Vec::new(),
            },
            root_path: PathBuf::new(),
            basket: Basket::new(),
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            scan_tier: None,
            scan_ms: 0,
            loaded_snapshot: None,
            usn_checkpoint: None,
        }
    }
}

pub type SharedSession = Arc<Mutex<Session>>;
