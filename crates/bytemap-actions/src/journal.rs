//! Append-only undo journal (brief): every executed action appends one
//! JSONL line; nothing is ever rewritten or deleted. Default location
//! %LOCALAPPDATA%\ByteMap\undo\journal.jsonl (local-only product; no
//! network). Tests override the directory via `open_in`.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One journaled action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Wall-clock epoch milliseconds.
    pub ts_ms: u64,
    /// "recycle" | "permanent" | "archive" (string ids, stable for the UI).
    pub action: String,
    pub items: Vec<JournalItem>,
    /// Archive mode only: absolute path of the .bma container (undo reads
    /// it back). Absent (null) for recycle/permanent entries.
    #[serde(default)]
    pub archive: Option<String>,
}

/// One item of a journaled action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalItem {
    /// Absolute original path (Recycle Bin undo matches by this).
    pub path: String,
    /// "file" | "dir".
    pub kind: String,
    /// Logical size at scan time (informational).
    pub logical: u64,
}

pub struct Journal {
    file: PathBuf,
}

impl Journal {
    /// Default location (or BYTEMAP_UNDO_DIR override for local testing).
    pub fn open() -> std::io::Result<Journal> {
        let dir = std::env::var_os("BYTEMAP_UNDO_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_undo_dir);
        Journal::open_in(&dir)
    }

    /// Explicit directory (tests pass a %TEMP% folder).
    pub fn open_in(dir: &Path) -> std::io::Result<Journal> {
        std::fs::create_dir_all(dir)?;
        Ok(Journal {
            file: dir.join("journal.jsonl"),
        })
    }

    /// Append one entry. Never rewrites existing lines.
    pub fn append(&self, entry: &JournalEntry) -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")
    }

    /// All entries, oldest first. Corrupt lines are skipped (journal is
    /// advisory; a torn last line after a crash must not break undo).
    pub fn all(&self) -> std::io::Result<Vec<JournalEntry>> {
        let f = std::fs::File::open(&self.file)?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(e) = serde_json::from_str::<JournalEntry>(&line) {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Most recent entry.
    pub fn last(&self) -> std::io::Result<Option<JournalEntry>> {
        Ok(self.all()?.pop())
    }
}

fn default_undo_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("ByteMap").join("undo")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str, path: &str) -> JournalEntry {
        JournalEntry {
            ts_ms: 1_733_000_000_000,
            action: action.to_string(),
            items: vec![JournalItem {
                path: path.to_string(),
                kind: "file".to_string(),
                logical: 42,
            }],
            archive: None,
        }
    }

    #[test]
    fn append_then_read_roundtrip() {
        let dir = std::env::temp_dir().join("bytemap-journal-1");
        let _ = std::fs::remove_dir_all(&dir);
        let j = Journal::open_in(&dir).unwrap();
        j.append(&entry("recycle", r"C:\t\a.txt")).unwrap();
        j.append(&entry("permanent", r"C:\t\b.txt")).unwrap();
        let all = j.all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].action, "recycle");
        assert_eq!(all[1].action, "permanent");
        assert_eq!(all[1].items[0].path, r"C:\t\b.txt");
        let last = j.last().unwrap().unwrap();
        assert_eq!(last.action, "permanent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let dir = std::env::temp_dir().join("bytemap-journal-2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("journal.jsonl");
        std::fs::write(
            &file,
            "{\"ts_ms\":1,\"action\":\"recycle\",\"items\":[]}\nNOT-JSON\n",
        )
        .unwrap();
        let j = Journal::open_in(&dir).unwrap();
        let all = j.all().unwrap();
        assert_eq!(all.len(), 1, "torn/corrupt line skipped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_only_no_rewrite() {
        let dir = std::env::temp_dir().join("bytemap-journal-3");
        let _ = std::fs::remove_dir_all(&dir);
        let j = Journal::open_in(&dir).unwrap();
        j.append(&entry("recycle", r"C:\t\a.txt")).unwrap();
        let before = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
        j.append(&entry("recycle", r"C:\t\c.txt")).unwrap();
        let after = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
        assert!(
            after.starts_with(&before),
            "append-only: old bytes untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
