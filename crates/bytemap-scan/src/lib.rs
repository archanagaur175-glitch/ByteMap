//! bytemap-scan: scanner tiers for ByteMap.
//! Tier A (MFT, Phase 6), Tier B (jwalk + rayon walker, Phase 1), Tier C (network/removable).

/// Placeholder version marker; replaced by scanner code in Phase 1.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_exposed() {
        assert!(!super::VERSION.is_empty());
    }
}
