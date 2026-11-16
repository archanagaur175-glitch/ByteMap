//! Scan session state. Rust owns the tree; the UI never receives it.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use bytemap_core::Tree;

pub struct Session {
    /// The most recently completed scan (empty while scanning or idle).
    pub tree: Tree,
    /// Categories + tiers for `tree` (computed once per completed scan).
    pub cats: bytemap_core::categorizer::CategoryMap,
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
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub type SharedSession = Arc<Mutex<Session>>;
