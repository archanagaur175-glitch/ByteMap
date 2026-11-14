//! bytemap-actions: dry run, recycle, archive, undo journal for ByteMap.
//! Highest-risk crate in the product: every destructive path needs tests
//! and owner review (AGENTS.md rule 7). Implemented in Phase 5.

/// Placeholder version marker; replaced by action code in Phase 5.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_exposed() {
        assert!(!super::VERSION.is_empty());
    }
}
