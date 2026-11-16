//! Executing cleanup actions: Recycle Bin (default) and permanent delete.
//!
//! Safety (brief + AGENTS.md rule 7):
//! - EVERY item is re-gated here with [`crate::can_offer_delete`] — the
//!   basket gate is not enough (defense in depth).
//! - Permanent delete REQUIRES the exact confirmation string "PERMANENT"
//!   from the UI (the user typed it); it is never implicit.
//! - Per-item outcomes: one failure never aborts the batch.

use std::fs;
use std::path::Path;

use crate::basket::BasketItem;

/// Exact string the UI must send after the user typed it.
pub const PERMANENT_CONFIRMATION: &str = "PERMANENT";

/// Outcome for one item.
#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub node_id: u32,
    pub path: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Move each item to the Recycle Bin (trash crate). Protected/unknown
/// tiers are refused per item; the batch continues.
pub fn recycle_items(items: &[BasketItem]) -> Vec<ActionOutcome> {
    items
        .iter()
        .map(|item| match can_delete(item) {
            Err(e) => ActionOutcome {
                node_id: item.node_id,
                path: item.path.clone(),
                ok: false,
                error: Some(e),
            },
            Ok(()) => match trash::delete(&item.path) {
                Ok(()) => ActionOutcome {
                    node_id: item.node_id,
                    path: item.path.clone(),
                    ok: true,
                    error: None,
                },
                Err(e) => ActionOutcome {
                    node_id: item.node_id,
                    path: item.path.clone(),
                    ok: false,
                    error: Some(e.to_string()),
                },
            },
        })
        .collect()
}

/// Permanently delete each item. Refuses everything unless
/// `confirmation == PERMANENT_CONFIRMATION` (brief: typed confirmation).
pub fn permanent_delete_items(
    items: &[BasketItem],
    confirmation: Option<&str>,
) -> Vec<ActionOutcome> {
    if confirmation != Some(PERMANENT_CONFIRMATION) {
        return items
            .iter()
            .map(|i| ActionOutcome {
                node_id: i.node_id,
                path: i.path.clone(),
                ok: false,
                error: Some("permanent delete requires typed confirmation".to_string()),
            })
            .collect();
    }
    items
        .iter()
        .map(|item| match can_delete(item) {
            Err(e) => ActionOutcome {
                node_id: item.node_id,
                path: item.path.clone(),
                ok: false,
                error: Some(e),
            },
            Ok(()) => {
                let p = Path::new(&item.path);
                let r = if p.is_dir() {
                    fs::remove_dir_all(p)
                } else {
                    fs::remove_file(p)
                };
                match r {
                    Ok(()) => ActionOutcome {
                        node_id: item.node_id,
                        path: item.path.clone(),
                        ok: true,
                        error: None,
                    },
                    Err(e) => ActionOutcome {
                        node_id: item.node_id,
                        path: item.path.clone(),
                        ok: false,
                        error: Some(e.to_string()),
                    },
                }
            }
        })
        .collect()
}

/// The gate again, with a human-readable reason.
fn can_delete(item: &BasketItem) -> Result<(), String> {
    if crate::can_offer_delete(item.tier) {
        Ok(())
    } else if item.tier == crate::tier::PROTECTED {
        Err("protected item is never deletable".to_string())
    } else {
        Err("unknown tier fails closed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier;

    fn item(id: u32, tier_id: u8, create: bool, dir: &Path) -> BasketItem {
        let path = dir.join(format!("t{id}.txt"));
        if create {
            fs::write(&path, b"data").unwrap();
        }
        BasketItem {
            node_id: id,
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            logical: 4,
            on_disk: 4,
            mtime: 0,
            tier: tier_id,
            cat: 6,
        }
    }

    #[test]
    fn permanent_delete_requires_confirmation() {
        let dir = std::env::temp_dir().join("bytemap-perm-1");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let items = vec![item(1, tier::SAFE, true, &dir)];
        let out = permanent_delete_items(&items, None);
        assert!(!out[0].ok);
        assert!(items[0].path_is_present());
        let out = permanent_delete_items(&items, Some("delete"));
        assert!(!out[0].ok, "wrong string refused");
        assert!(items[0].path_is_present());
        let out = permanent_delete_items(&items, Some(PERMANENT_CONFIRMATION));
        assert!(out[0].ok);
        assert!(!items[0].path_is_present(), "file really gone");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn permanent_delete_refuses_protected() {
        let dir = std::env::temp_dir().join("bytemap-perm-2");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let items = vec![item(1, tier::PROTECTED, true, &dir)];
        let out = permanent_delete_items(&items, Some(PERMANENT_CONFIRMATION));
        assert!(!out[0].ok);
        assert_eq!(
            out[0].error.as_deref(),
            Some("protected item is never deletable")
        );
        assert!(items[0].path_is_present(), "protected file untouched");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycle_moves_to_recycle_bin_and_fails_closed_on_unknown_tier() {
        let dir = std::env::temp_dir().join("bytemap-recycle-1");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // One safe file, one unknown-tier item (same path; tier alone differs).
        let items = vec![item(1, tier::SAFE, true, &dir), item(2, 99, true, &dir)];
        let out = recycle_items(&items);
        assert!(out[0].ok, "safe item recycles: {:?}", out[0].error);
        assert!(!out[1].ok, "unknown tier refused");
        assert_eq!(out[1].error.as_deref(), Some("unknown tier fails closed"));
        assert!(!items[0].path_is_present(), "recycled -> original gone");
        assert!(items[1].path_is_present(), "refused item untouched");
        let _ = fs::remove_dir_all(&dir);
        // The recycled file stays in the Recycle Bin (OS-level; not our
        // business to purge — undo in T5 restores by original path).
    }

    /// Path::exists helper on the recorded path.
    trait Presence {
        fn path_is_present(&self) -> bool;
    }
    impl Presence for BasketItem {
        fn path_is_present(&self) -> bool {
            Path::new(&self.path).exists()
        }
    }
}
