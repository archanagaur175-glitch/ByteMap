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
        }
    }
}

pub type SharedSession = Arc<Mutex<Session>>;
