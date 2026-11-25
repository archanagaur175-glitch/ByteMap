//! USN journal refresh (P8.T3): the elevated half of change tracking.
//!
//! The app sends [`Frame::UsnRequest`] with the volume root and the journal
//! checkpoint stored in a snapshot. This module validates that the journal
//! still covers the checkpoint, then resolves every journal entry after it
//! to a live path and streams one [`Frame::UsnChange`] per distinct path.
//!
//! Fail-closed rules:
//! - a different `journal_id`, or a checkpoint older than the journal's
//!   `lowest_valid_usn` (the journal wrapped), is refused: the app must run
//!   a full scan rather than apply a wrong delta;
//! - a *folder* rename is refused: snapshots store paths, not file ids, so
//!   the app cannot move a subtree safely from a path delta;
//! - a path that cannot be stat'ed for a reason other than "not found" is
//!   counted as unresolved and left untouched (never guessed deleted);
//! - more than [`MAX_USN_CHANGES`] distinct paths is refused.
//!
//! The USN crate needs Administrator; this module only runs inside the
//! elevated helper (see docs/agent/DECISIONS.md).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::fs::MetadataExt;

use usn_journal_rs::journal::JournalIterOptions;
use usn_journal_rs::{Usn, UsnReason};

use crate::proto::{self, Frame, MAX_USN_CHANGES};

/// Journal checkpoint for a whole-volume scan, or `None` when the volume
/// has no active journal. A scan never fails just because it cannot be
/// refreshed later.
pub(super) fn checkpoint_for(root: &str) -> Option<(u64, i64)> {
    let letter = drive_letter(root)?;
    let volume = usn_journal_rs::volume::Volume::from_drive_letter(letter).ok()?;
    let data = volume.journal().query().ok()?;
    Some((data.journal_id, data.next_usn.get()))
}

/// Entry point: every failure becomes an `Error` frame, which ends the
/// session normally on the wire (same contract as `mft`).
pub(super) fn handle_usn_request<S: Read + Write>(
    stream: &mut S,
    path: &str,
    journal_id: u64,
    since_usn: i64,
) {
    if let Err(message) = run_usn(stream, path, journal_id, since_usn) {
        let _ = proto::write_frame(stream, &Frame::Error { message });
    }
}

/// One distinct changed path, ready for the wire.
struct PendingChange {
    path: String,
    exists: bool,
    attributes: u32,
    logical: u64,
    mtime: i64,
}

fn run_usn<S: Read + Write>(
    stream: &mut S,
    path: &str,
    journal_id: u64,
    since_usn: i64,
) -> Result<(), String> {
    let letter = drive_letter(path)
        .ok_or_else(|| format!("USN refresh needs a whole volume, not {path}"))?;
    let volume = usn_journal_rs::volume::Volume::from_drive_letter(letter)
        .map_err(|e| format!("cannot open volume {letter}: {e}"))?;
    let journal = volume.journal();
    let data = journal
        .query()
        .map_err(|e| format!("cannot query the USN journal on {letter}: {e}"))?;

    if data.journal_id != journal_id {
        return Err(
            "the USN journal was recreated since this snapshot; run a full scan".to_owned(),
        );
    }
    let since = Usn::new(since_usn);
    if since > data.next_usn {
        return Err(
            "the snapshot's USN cursor is ahead of the journal; run a full scan".to_owned(),
        );
    }
    if since < data.lowest_valid_usn || since < data.first_usn {
        return Err("the USN journal has wrapped since this snapshot; run a full scan".to_owned());
    }

    // The cursor the refreshed snapshot will store: everything after it is
    // captured by this (or a later) refresh.
    let checkpoint = data.next_usn.get();
    proto::write_frame(
        stream,
        &Frame::UsnCheckpoint {
            journal_id: data.journal_id,
            next_usn: checkpoint,
        },
    )
    .map_err(|e| e.to_string())?;

    let options = JournalIterOptions::builder()
        .start_usn(since)
        .reason_mask(UsnReason::ALL)
        .wait_for_more(false)
        .build()
        .map_err(|e| format!("cannot build journal options: {e}"))?;
    let entries = journal
        .try_iter_with_options(options)
        .map_err(|e| format!("cannot read the USN journal on {letter}: {e}"))?;

    // Paths, not file ids: resolve each entry against the live directory
    // tree, then stat the result. Caching is deliberately OFF: a rename
    // between resolution and stat would poison a directory cache.
    let resolver = volume.path_resolver();
    let mut changes: BTreeMap<String, PendingChange> = BTreeMap::new();
    let mut unresolved = 0u64;

    for entry in entries {
        let entry = entry.map_err(|e| format!("USN journal read failed: {e}"))?;
        // A renamed folder would need a subtree move, which a path delta
        // cannot express safely.
        if entry.is_dir() && entry.reason.is_rename() {
            return Err(
                "a folder was renamed or moved since this snapshot; run a full scan".to_owned(),
            );
        }
        let Some(resolved) = resolver.resolve_path(&entry) else {
            // Parent gone, hardlink unreadable, ... : the deletion of the
            // parent (or the entry itself) covers this path.
            unresolved += 1;
            continue;
        };
        let path_text = resolved.to_string_lossy().into_owned();
        let key = path_text.to_lowercase();
        if !changes.contains_key(&key) && changes.len() >= MAX_USN_CHANGES {
            return Err(format!(
                "more than {MAX_USN_CHANGES} changed paths; run a full scan"
            ));
        }
        match stat_change(path_text, entry.file_attributes.bits()) {
            Some(change) => {
                changes.insert(key, change);
            }
            // Metadata unreadable for a reason other than "gone": leave the
            // snapshot's node alone rather than guess a size of zero.
            None => unresolved += 1,
        }
    }

    let count = changes.len() as u64;
    for change in changes.into_values() {
        proto::write_frame(
            stream,
            &Frame::UsnChange {
                path: change.path,
                exists: change.exists,
                attributes: change.attributes,
                logical: change.logical,
                mtime: change.mtime,
            },
        )
        .map_err(|e| e.to_string())?;
    }
    proto::write_frame(
        stream,
        &Frame::UsnDone {
            changes: count,
            unresolved,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Current metadata for `path`. `None` means the entry must be skipped
/// (stat failed for a reason other than "gone"), so the app never guesses a
/// size; a missing path is a deletion and carries the journal attributes.
fn stat_change(path: String, journal_attributes: u32) -> Option<PendingChange> {
    match fs::symlink_metadata(&path) {
        Ok(md) => {
            let attributes = md.file_attributes();
            let is_dir = attributes & 0x10 != 0; // FILE_ATTRIBUTE_DIRECTORY
            Some(PendingChange {
                path,
                exists: true,
                attributes,
                logical: if is_dir { 0 } else { md.len() },
                mtime: md.last_write_time() as i64,
            })
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Some(PendingChange {
            path,
            exists: false,
            attributes: journal_attributes,
            logical: 0,
            mtime: 0,
        }),
        Err(_) => None,
    }
}

/// Drive letter of a whole-volume root: `C:\`, `C:` or `\\?\C:\`. A path
/// below the root (`C:\Users`) is not a volume and is refused.
fn drive_letter(path: &str) -> Option<char> {
    let trimmed = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix("//?/"))
        .unwrap_or(path);
    let mut chars = trimmed.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    if chars.next() != Some(':') {
        return None;
    }
    match chars.as_str() {
        "" | "\\" | "/" => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letters_are_recognized() {
        assert_eq!(drive_letter(r"C:\"), Some('C'));
        assert_eq!(drive_letter("c:"), Some('C'));
        assert_eq!(drive_letter(r"\\?\D:\"), Some('D'));
        assert_eq!(drive_letter(r"//?/e:/"), Some('E'));
    }

    #[test]
    fn folders_are_not_volumes() {
        assert_eq!(drive_letter(r"C:\Users"), None);
        assert_eq!(drive_letter(r"\\server\share"), None);
        assert_eq!(drive_letter("not a path"), None);
        assert_eq!(drive_letter(""), None);
    }

    #[test]
    fn missing_paths_are_deletions() {
        let missing = std::env::temp_dir().join("bytemap-usn-does-not-exist-12345");
        let change = stat_change(missing.to_string_lossy().into_owned(), 0x20)
            .expect("a missing path is a fact, not a skip");
        assert!(!change.exists);
        assert_eq!(change.logical, 0);
    }
}
