//! Undo: restore recycled items using the append-only journal. Recycle Bin
//! items are found by `orig_path` (the journal records exactly that), so no
//! recycle-bin internals leak into ByteMap.

use std::collections::HashSet;

/// Result of one undo attempt.
#[derive(Debug, Default)]
pub struct UndoReport {
    pub action: String,
    /// Paths physically restored.
    pub restored: Vec<String>,
    /// Journaled paths not found in the Recycle Bin (already restored
    /// manually, emptied, or purged).
    pub not_found: Vec<String>,
}

/// Restore every journaled path that is still in the Recycle Bin.
pub fn undo_recycle(paths: &[String]) -> Result<UndoReport, String> {
    let wanted: HashSet<&str> = paths.iter().map(|p| p.as_str()).collect();
    let all = trash::os_limited::list().map_err(|e| e.to_string())?;

    let mut matches = Vec::new();
    let mut not_found: Vec<String> = Vec::new();
    let mut found: HashSet<String> = HashSet::new();
    for item in all {
        let orig = item.original_path().to_string_lossy().into_owned();
        if wanted.contains(orig.as_str()) {
            matches.push(item);
            found.insert(orig);
        }
    }
    for p in paths {
        if !found.contains(p) {
            not_found.push(p.clone());
        }
    }

    if !matches.is_empty() {
        trash::os_limited::restore_all(matches).map_err(|e| e.to_string())?;
    }

    Ok(UndoReport {
        action: "recycle".to_string(),
        restored: paths
            .iter()
            .filter(|p| !not_found.contains(p))
            .cloned()
            .collect(),
        not_found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real round trip on a %TEMP% fixture: recycle via trash, then undo
    /// restores the file to its original path. Nothing is left in the
    /// Recycle Bin afterwards (restore_all removes the entry).
    #[test]
    fn recycle_undo_restores_original() {
        let dir = std::env::temp_dir().join("bytemap-undo-1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("undo-me.txt");
        std::fs::write(&p, b"undo fixture").unwrap();

        trash::delete(&p).unwrap();
        assert!(!p.exists(), "recycled -> original gone");

        let rep = undo_recycle(&[p.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(rep.restored.len(), 1, "rep: {rep:?}");
        assert!(rep.not_found.is_empty());
        assert!(p.exists(), "undo restored the original file");
        assert_eq!(std::fs::read(&p).unwrap(), b"undo fixture");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_reports_paths_never_recycled() {
        let rep = undo_recycle(&[r"C:\bytemap-never-recycled.txt".to_string()]).unwrap();
        assert!(rep.restored.is_empty());
        assert_eq!(rep.not_found, vec![r"C:\bytemap-never-recycled.txt"]);
    }
}
