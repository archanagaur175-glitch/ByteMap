//! bytemap-actions: dry run, recycle, archive, undo journal for ByteMap.
//! Highest-risk crate in the product: every destructive path needs tests
//! and owner review (AGENTS.md rule 7). Implemented in Phase 5.
//!
//! Already present in Phase 4: the Protected-tier gate. Delete affordances
//! may only exist for Safe/Review items; Protected is never deletable
//! (pagefile.sys, hiberfil.sys, swapfile.sys, Windows folder, System Volume
//! Information, $Recycle.Bin).

pub mod basket;

pub use basket::{AddError, Basket, BasketItem};

/// Placeholder version marker; replaced by action code in Phase 5.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Safety tier ids, mirrored from bytemap-core::categorizer (kept in sync
/// by that crate's wire-format tests; actions must not depend on core).
pub mod tier {
    pub const SAFE: u8 = 0;
    pub const REVIEW: u8 = 1;
    pub const PROTECTED: u8 = 2;
}

/// May the UI offer a delete/recycle/archive action for this tier?
/// This is the single gate every Phase 5 action path must call first.
/// Fail-closed: only Safe and Review allow deletion; anything else
/// (including unknown ids) is refused.
pub fn can_offer_delete(tier: u8) -> bool {
    matches!(tier, tier::SAFE | tier::REVIEW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_exposed() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn protected_is_never_deletable() {
        assert!(!can_offer_delete(tier::PROTECTED));
    }

    #[test]
    fn safe_and_review_are_deletable() {
        assert!(can_offer_delete(tier::SAFE));
        assert!(can_offer_delete(tier::REVIEW));
    }

    #[test]
    fn unknown_tier_ids_fail_closed() {
        // Anything unrecognised must NOT allow deletion: fail closed.
        assert!(!can_offer_delete(99));
        assert!(!can_offer_delete(255));
    }
}
