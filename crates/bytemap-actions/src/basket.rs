//! Basket: the staging list for cleanup actions (brief "Action Hub safety").
//!
//! The UI stages node IDs; the resolved path snapshot travels with the item
//! (Rust resolved it — the frontend never sends path strings). Every add is
//! gated by [`crate::can_offer_delete`]: Protected and unknown tiers are
//! refused fail-closed, so a basket can never contain a non-deletable item.
//! Executing actions may still re-verify (defense in depth).

/// One staged cleanup candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct BasketItem {
    /// Tree node id (UI refers to items by node ID only).
    pub node_id: u32,
    /// Absolute path, resolved by Rust at add time.
    pub path: String,
    /// File/folder name (last path component).
    pub name: String,
    /// Logical size at scan time (roll-up for folders).
    pub logical: u64,
    /// Size on disk at scan time.
    pub on_disk: u64,
    /// Last write time at scan time (Windows FILETIME, 100 ns since 1601).
    /// The stale-scan guard compares this against the current mtime.
    pub mtime: i64,
    /// Tier id (mirrored in [`crate::tier`]).
    pub tier: u8,
    /// Category id (0 Code .. 6 Other).
    pub cat: u8,
}

/// Why an add was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddError {
    /// Tier is Protected (pagefile.sys, Windows folder, ...).
    Protected,
    /// Tier id not in the fail-closed allowlist (Safe | Review).
    UnknownTier,
    /// This node id is already in the basket.
    Duplicate,
}

/// The staging list. Kept in scan order; no sorting here (UI decides).
#[derive(Debug, Default, Clone)]
pub struct Basket {
    items: Vec<BasketItem>,
}

impl Basket {
    pub fn new() -> Self {
        Basket::default()
    }

    /// Stage one item. Refuses Protected/unknown tiers and duplicates.
    pub fn add(&mut self, item: BasketItem) -> Result<(), AddError> {
        if !crate::can_offer_delete(item.tier) {
            return Err(if item.tier == crate::tier::PROTECTED {
                AddError::Protected
            } else {
                AddError::UnknownTier
            });
        }
        if self.items.iter().any(|i| i.node_id == item.node_id) {
            return Err(AddError::Duplicate);
        }
        self.items.push(item);
        Ok(())
    }

    /// Stage many; returns (added_count, refused) so the UI can show what
    /// was skipped (e.g. Protected rows) without aborting the bulk add.
    pub fn add_many(&mut self, batch: Vec<BasketItem>) -> (usize, Vec<(u32, AddError)>) {
        let mut added = 0;
        let mut refused = Vec::new();
        for item in batch {
            let id = item.node_id;
            match self.add(item) {
                Ok(()) => added += 1,
                Err(e) => refused.push((id, e)),
            }
        }
        (added, refused)
    }

    /// Remove by node id; true if it was present.
    pub fn remove(&mut self, node_id: u32) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.node_id != node_id);
        self.items.len() != before
    }

    /// Empty the basket (no confirmation here — the UI owns that).
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn items(&self) -> &[BasketItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier;

    fn item(id: u32, tier_id: u8) -> BasketItem {
        BasketItem {
            node_id: id,
            path: format!(r"C:\tmp\item{id}"),
            name: format!("item{id}"),
            logical: 100,
            on_disk: 100,
            mtime: 0,
            tier: tier_id,
            cat: 6,
        }
    }

    #[test]
    fn safe_and_review_stage_ok() {
        let mut b = Basket::new();
        assert!(b.add(item(1, tier::SAFE)).is_ok());
        assert!(b.add(item(2, tier::REVIEW)).is_ok());
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn protected_is_refused() {
        let mut b = Basket::new();
        assert_eq!(b.add(item(1, tier::PROTECTED)), Err(AddError::Protected));
        assert!(b.is_empty());
    }

    #[test]
    fn unknown_tiers_are_refused() {
        let mut b = Basket::new();
        assert_eq!(b.add(item(1, 99)), Err(AddError::UnknownTier));
        assert_eq!(b.add(item(2, 255)), Err(AddError::UnknownTier));
        assert!(b.is_empty());
    }

    #[test]
    fn duplicates_are_refused() {
        let mut b = Basket::new();
        b.add(item(1, tier::SAFE)).unwrap();
        assert_eq!(b.add(item(1, tier::SAFE)), Err(AddError::Duplicate));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn remove_and_clear() {
        let mut b = Basket::new();
        b.add(item(1, tier::SAFE)).unwrap();
        b.add(item(2, tier::REVIEW)).unwrap();
        assert!(b.remove(1));
        assert!(!b.remove(1)); // already gone
        assert_eq!(b.len(), 1);
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn add_many_counts_and_reports() {
        let mut b = Basket::new();
        let batch = vec![
            item(1, tier::SAFE),
            item(2, tier::PROTECTED), // refused
            item(3, tier::REVIEW),
            item(3, tier::REVIEW), // duplicate
            item(4, 42),           // unknown tier
        ];
        let (added, refused) = b.add_many(batch);
        assert_eq!(added, 2);
        assert_eq!(refused.len(), 3);
        assert_eq!(refused[0], (2, AddError::Protected));
        assert_eq!(refused[1], (3, AddError::Duplicate));
        assert_eq!(refused[2], (4, AddError::UnknownTier));
        assert_eq!(b.len(), 2);
    }
}
