//! bytemap-core: arena tree, roll-ups, categorizer, and layout for ByteMap.

/// Placeholder version marker; replaced by real model code in Phase 1.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_exposed() {
        assert!(!super::VERSION.is_empty());
    }
}
