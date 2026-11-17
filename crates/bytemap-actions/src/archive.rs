//! Archive mode (brief): compress, verify, then remove the original.
//!
//! Container format (owner-approved 2026-11-16): a JSONL manifest + raw file
//! bytes, the whole stream zstd-compressed. Uncompressed stream layout:
//!   line 1: {"format":"bytemap-archive","version":1}
//!   then per file: {"path":<absolute original>,"size":N,"sha256":<hex>}
//!                  followed by exactly N raw bytes.
//! Verify = decompress + size compare + SHA-256 compare (sha2 0.11.1,
//! owner-approved 2026-11-17). Originals are removed ONLY after the fresh
//! container passes verification.
//!
//! Safety: folders are refused in Phase 5 (a folder may contain Protected
//! content the folder's own tier does not describe — see OPEN_QUESTIONS);
//! every request is re-gated with can_offer_delete.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};
use zstd::stream::{read::Decoder, write::Encoder};

use crate::basket::BasketItem;
use crate::recycle::ActionOutcome;

/// Container file extension.
pub const ARCHIVE_EXT: &str = "bma";
/// Header line, first line of every uncompressed stream.
const HEADER: &str = "{\"format\":\"bytemap-archive\",\"version\":1}";

/// Result of one archive execution.
#[derive(Debug)]
pub struct ArchiveResult {
    /// Absolute path of the created container.
    pub archive_path: String,
    /// Per-item outcomes (path = original absolute path).
    pub outcomes: Vec<ActionOutcome>,
}

/// Result of restoring an archive (undo).
#[derive(Debug, Default)]
pub struct RestoreResult {
    /// Original paths written back successfully.
    pub restored: Vec<String>,
    /// Paths that already exist on disk — never overwritten.
    pub conflicts: Vec<String>,
}

/// Archive each item into one zstd container, verify the container, then
/// remove the originals. Folders and gated tiers are refused per item.
pub fn archive_items(items: &[BasketItem], archive_dir: &Path) -> Result<ArchiveResult, String> {
    // Per-item pre-checks; anything refused never touches the container.
    let mut accepted: Vec<&BasketItem> = Vec::new();
    let mut outcomes: Vec<ActionOutcome> = Vec::new();
    for item in items {
        if !crate::can_offer_delete(item.tier) {
            outcomes.push(ActionOutcome {
                node_id: item.node_id,
                path: item.path.clone(),
                ok: false,
                error: Some(if item.tier == crate::tier::PROTECTED {
                    "protected item is never deletable".to_string()
                } else {
                    "unknown tier fails closed".to_string()
                }),
            });
        } else if Path::new(&item.path).is_dir() {
            outcomes.push(ActionOutcome {
                node_id: item.node_id,
                path: item.path.clone(),
                ok: false,
                error: Some("archive supports files only (folder not archived)".to_string()),
            });
        } else {
            accepted.push(item);
        }
    }

    if accepted.is_empty() {
        return Ok(ArchiveResult {
            archive_path: String::new(),
            outcomes,
        });
    }

    fs::create_dir_all(archive_dir).map_err(|e| format!("archive dir: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let container = archive_dir.join(format!("bytemap-{ts}.{ARCHIVE_EXT}"));

    // 1. compress (whole stream: JSONL manifest + raw bytes -> zstd).
    write_container(&container, &accepted)?;
    let container_path = container.to_string_lossy().into_owned();

    // 2. verify (decompress + size + SHA-256). On failure: container is
    //    deleted and NOTHING is removed — fail closed.
    if let Err(e) = verify_container(&container) {
        let _ = fs::remove_file(&container);
        return Err(format!("archive verify failed, originals kept: {e}"));
    }

    // 3. remove originals (files only — accepted guarantees that).
    for item in &accepted {
        let r = fs::remove_file(&item.path);
        outcomes.push(ActionOutcome {
            node_id: item.node_id,
            path: item.path.clone(),
            ok: r.is_ok(),
            error: r.err().map(|e| e.to_string()),
        });
    }

    Ok(ArchiveResult {
        archive_path: container_path,
        outcomes,
    })
}

/// Write the compressed container stream (manifest + bytes interleaved).
/// Pass 1 hashes each file (manifest), pass 2 copies the bytes into the
/// stream. A file changing between passes fails the later verification —
/// fail closed, originals kept.
fn write_container(container: &Path, items: &[&BasketItem]) -> Result<(), String> {
    let f = fs::File::create(container).map_err(|e| format!("create container: {e}"))?;
    let mut enc = Encoder::new(f, 3).map_err(|e| format!("zstd encoder: {e}"))?;

    enc.write_all(HEADER.as_bytes())
        .and_then(|_| enc.write_all(b"\n"))
        .map_err(|e| format!("header: {e}"))?;

    for item in items {
        // Pass 1: size + SHA-256 of the source file.
        let (size, sha) = hash_file(&item.path)?;

        // Manifest line, then the raw bytes follow in the same stream.
        let line = format!(
            "{{\"path\":{},\"size\":{size},\"sha256\":\"{sha}\"}}",
            serde_json::to_string(&item.path).unwrap_or_default(),
        );
        let mut file = fs::File::open(&item.path).map_err(|e| format!("open: {e}"))?;
        enc.write_all(line.as_bytes())
            .and_then(|_| enc.write_all(b"\n"))
            .and_then(|_| std::io::copy(&mut file, &mut enc).map(|_| ()))
            .map_err(|e| format!("write {}: {e}", item.path))?;
    }

    enc.finish().map_err(|e| format!("zstd finish: {e}"))?;
    Ok(())
}

/// Size + lowercase-hex SHA-256 of a file (single pass, 64 KiB chunks).
fn hash_file(path: &str) -> Result<(u64, String), String> {
    let mut file = fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("stat {path}: {e}"))?
        .len();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut hashed: u64 = 0;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {path}: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        hashed += n as u64;
    }
    if hashed != size {
        return Err(format!("{path}: size changed during hashing"));
    }
    Ok((size, to_hex(&hasher.finalize())))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decompress + parse + size/SHA-256 compare. Full pass, no shortcuts.
fn verify_container(container: &Path) -> Result<(), String> {
    let f = fs::File::open(container).map_err(|e| format!("open container: {e}"))?;
    let mut dec = Decoder::new(f).map_err(|e| format!("zstd decoder: {e}"))?;

    let header = read_line(&mut dec)?;
    if header != HEADER {
        return Err(format!("bad header: {header}"));
    }

    while let Some(line) = read_line_opt(&mut dec)? {
        let entry: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("manifest: {e}"))?;
        let path = entry["path"].as_str().ok_or("manifest missing path")?;
        let size = entry["size"].as_u64().ok_or("manifest missing size")?;
        let want = entry["sha256"].as_str().ok_or("manifest missing sha256")?;

        let mut buf = vec![0u8; size as usize];
        dec.read_exact(&mut buf)
            .map_err(|e| format!("payload {path}: {e}"))?;
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        let got = to_hex(&hasher.finalize());
        if got != want {
            return Err(format!("sha256 mismatch for {path}"));
        }
    }
    Ok(())
}

/// Restore an archive: write each entry back to its original path.
/// Existing files are NEVER overwritten (reported as conflicts).
pub fn restore_archive(container: &Path) -> Result<RestoreResult, String> {
    let f = fs::File::open(container).map_err(|e| format!("open container: {e}"))?;
    let mut dec = Decoder::new(f).map_err(|e| format!("zstd decoder: {e}"))?;

    let header = read_line(&mut dec)?;
    if header != HEADER {
        return Err(format!("bad header: {header}"));
    }

    let mut report = RestoreResult::default();
    while let Some(line) = read_line_opt(&mut dec)? {
        let entry: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("manifest: {e}"))?;
        let path = entry["path"].as_str().ok_or("manifest missing path")?;
        let size = entry["size"].as_u64().ok_or("manifest missing size")?;
        let want = entry["sha256"].as_str().ok_or("manifest missing sha256")?;

        let mut buf = vec![0u8; size as usize];
        dec.read_exact(&mut buf)
            .map_err(|e| format!("payload {path}: {e}"))?;
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        if to_hex(&hasher.finalize()) != want {
            return Err(format!("sha256 mismatch for {path}"));
        }

        let target = Path::new(path);
        if target.exists() {
            report.conflicts.push(path.to_string());
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("parent {path}: {e}"))?;
        }
        fs::write(target, &buf).map_err(|e| format!("write {path}: {e}"))?;
        report.restored.push(path.to_string());
    }
    Ok(report)
}

fn read_line<R: Read>(r: &mut R) -> Result<String, String> {
    read_line_opt(r)?.ok_or_else(|| "unexpected end of archive stream".to_string())
}

/// Read one \n-terminated line (returns None at clean end of stream).
fn read_line_opt<R: Read>(r: &mut R) -> Result<Option<String>, String> {
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match r.read(&mut b) {
            Ok(0) => {
                return if line.is_empty() {
                    Ok(None)
                } else {
                    Err("torn archive stream".to_string())
                };
            }
            Ok(_) => {
                if b[0] == b'\n' {
                    return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
                }
                line.push(b[0]);
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier;

    fn item(id: u32, path: &Path, tier_id: u8) -> BasketItem {
        BasketItem {
            node_id: id,
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            logical: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            on_disk: 0,
            mtime: 0,
            tier: tier_id,
            cat: 6,
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Full Phase-5 brief round trip: archive -> verify passes -> original
    /// removed -> undo restores content byte-for-byte.
    #[test]
    fn archive_verify_remove_restore_roundtrip() {
        let dir = temp("bytemap-arch-1");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let a = src.join("a.bin");
        let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&a, &payload).unwrap();

        let adir = dir.join("archives");
        let res = archive_items(&[item(1, &a, tier::SAFE)], &adir).unwrap();
        assert!(res.outcomes[0].ok, "{res:?}");
        assert!(Path::new(&res.archive_path).exists(), "container written");
        assert!(!a.exists(), "original removed only after verify");

        // Undo: restore back to the original path, exact bytes.
        let rep = restore_archive(Path::new(&res.archive_path)).unwrap();
        assert_eq!(rep.restored, vec![a.to_string_lossy().into_owned()]);
        assert!(rep.conflicts.is_empty());
        assert_eq!(fs::read(&a).unwrap(), payload, "byte-for-byte restore");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_container_fails_verify_and_restores_nothing() {
        let dir = temp("bytemap-arch-2");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let a = src.join("a.txt");
        fs::write(&a, b"corrupt me").unwrap();
        let b = src.join("b.txt");
        fs::write(&b, b"safe").unwrap();

        let adir = dir.join("archives");
        let res = archive_items(&[item(1, &a, tier::SAFE)], &adir).unwrap();
        assert!(!a.exists(), "archived ok -> original removed");
        let container = res.archive_path.clone();
        let orig = fs::read(&container).unwrap();

        // Flip a byte in the middle of the compressed stream.
        let mut corrupt = orig.clone();
        let mid = corrupt.len() / 2;
        corrupt[mid] ^= 0xFF;
        fs::write(&container, &corrupt).unwrap();

        // Undo refuses a corrupt container, and verify_container rejects it.
        assert!(restore_archive(Path::new(&container)).is_err());
        assert!(verify_container(Path::new(&container)).is_err());
        // The other file was never part of the archive: untouched.
        assert!(b.exists());
        assert_eq!(fs::read(&b).unwrap(), b"safe");

        let _ = fs::remove_dir_all(&dir);
        let _ = orig;
    }

    #[test]
    fn gated_tiers_and_directories_are_refused() {
        let dir = temp("bytemap-arch-3");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let f = src.join("f.txt");
        fs::write(&f, b"x").unwrap();

        let items = [
            item(1, &f, tier::PROTECTED),
            item(2, &f, 99),           // unknown tier
            item(3, &src, tier::SAFE), // directory
        ];
        let adir = dir.join("archives");
        let res = archive_items(&items, &adir).unwrap();
        assert!(
            res.archive_path.is_empty(),
            "nothing accepted -> no container"
        );
        assert_eq!(res.outcomes.len(), 3);
        assert!(!res.outcomes[0].ok);
        assert!(!res.outcomes[1].ok);
        assert_eq!(
            res.outcomes[2].error.as_deref(),
            Some("archive supports files only (folder not archived)")
        );
        assert!(f.exists(), "originals untouched");

        let _ = fs::remove_dir_all(&dir);
    }
}
