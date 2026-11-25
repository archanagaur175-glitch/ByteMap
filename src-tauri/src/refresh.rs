//! Snapshot refresh through the USN journal (P8.T3).
//!
//! One explicit user action: load a saved snapshot, ask the elevated helper
//! for every journal entry after its stored cursor, apply the delta to the
//! snapshot tree, save the result as a NEW snapshot, and load it into the
//! session. The old file is untouched, so a bad refresh never destroys the
//! last good snapshot.
//!
//! Fail-closed: a journal gap, a recreated journal, a renamed folder, too
//! many changes, or a delta the tree cannot place all become an error that
//! tells the user to run a full scan. Nothing is half-applied.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bytemap_core::categorizer::Rules;
use bytemap_core::snapshot::SnapshotMeta;
use bytemap_core::{apply_changes, ChangeFact};
use serde_json::json;
use tauri::State;

use crate::snapshot_store::{apply_snapshot, load_in, save_in, snapshot_dir};
use crate::state::SharedSession;

/// Refresh a saved snapshot with the USN journal and load the result.
///
/// Returns the `snapshot_load` payload plus a `refresh` object with the
/// delta counts, so the frontend can reuse its snapshot-state update.
#[tauri::command]
pub fn snapshot_refresh(
    name: String,
    session: State<'_, SharedSession>,
) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        refresh_snapshot(&snapshot_dir(), &name, session.inner())
    }
    #[cfg(not(windows))]
    {
        let _ = (name, session);
        Err("USN refresh is Windows-only".to_string())
    }
}

#[cfg(windows)]
fn refresh_snapshot(
    dir: &Path,
    name: &str,
    shared: &SharedSession,
) -> Result<serde_json::Value, String> {
    let snapshot = load_in(dir, name)?;
    let checkpoint = snapshot.meta.usn.ok_or_else(|| {
        format!("{name} has no USN checkpoint; run a full MFT scan and save again")
    })?;

    // Never prompt for UAC from a background thread; this is a direct click.
    bytemap_scan::spawn::ensure_helper()?;
    let mut client =
        bytemap_scan::mft_client::MftClient::connect(bytemap_scan::mft_client::DEFAULT_PIPE_NAME)
            .map_err(|e| e.to_string())?;
    client
        .request_usn_refresh(
            &snapshot.meta.root_name,
            checkpoint.journal_id,
            checkpoint.next_usn,
        )
        .map_err(|e| format!("helper pipe write failed: {e}"))?;

    let mut next: Option<bytemap_core::UsnCheckpoint> = None;
    let mut wire_changes: Vec<bytemap_scan::mft_client::WireFrame> = Vec::new();
    // Read until the helper ends the session. `changes` frames are kept as
    // frames only to stay independent of bytemap-helper types here.
    let (helper_changes, helper_unresolved) = loop {
        match client.next_frame() {
            Ok(bytemap_scan::mft_client::WireFrame::UsnCheckpoint {
                journal_id,
                next_usn,
            }) => {
                next = Some(bytemap_core::UsnCheckpoint {
                    journal_id,
                    next_usn,
                });
            }
            Ok(frame @ bytemap_scan::mft_client::WireFrame::UsnChange { .. }) => {
                if wire_changes.len() >= bytemap_core::MAX_REFRESH_CHANGES {
                    return Err("too many changes; run a full scan".to_string());
                }
                wire_changes.push(frame);
            }
            Ok(bytemap_scan::mft_client::WireFrame::UsnDone {
                changes,
                unresolved,
            }) => break (changes, unresolved),
            Ok(bytemap_scan::mft_client::WireFrame::Error { message }) => return Err(message),
            Ok(other) => return Err(format!("unexpected frame from helper: {other:?}")),
            Err(e) => return Err(format!("helper pipe read failed: {e}")),
        }
    };
    if helper_changes != wire_changes.len() as u64 {
        return Err(
            "the helper's change count does not match its stream; run a full scan".to_string(),
        );
    }
    let next = next.ok_or_else(|| "the helper sent no USN checkpoint".to_string())?;

    let cluster = bytemap_scan::metadata::cluster_size(Path::new(&snapshot.meta.root_name));
    let mut facts = Vec::with_capacity(wire_changes.len());
    for frame in wire_changes {
        let bytemap_scan::mft_client::WireFrame::UsnChange {
            path,
            exists,
            attributes,
            logical,
            mtime,
        } = frame
        else {
            unreachable!("only change frames reach here");
        };
        let is_dir = attributes & bytemap_scan::metadata::FILE_ATTRIBUTE_DIRECTORY != 0;
        let on_disk = if exists && !is_dir {
            bytemap_scan::metadata::size_on_disk(Path::new(&path), logical, attributes, cluster)
        } else {
            0
        };
        facts.push(ChangeFact {
            path,
            exists,
            flags: fact_flags(attributes, is_dir),
            logical: if is_dir { 0 } else { logical },
            on_disk,
            mtime,
        });
    }

    let (tree, stats) = apply_changes(&snapshot.tree, &snapshot.meta.root_name, &facts)
        .map_err(|e| e.to_string())?;
    if stats.unresolved > 0 {
        return Err(format!(
            "{} changes could not be placed in the snapshot; run a full scan",
            stats.unresolved
        ));
    }
    let cats = bytemap_core::categorize(&tree, &Rules::default_rules());
    let meta = SnapshotMeta {
        created_unix: now_unix(),
        scan_ms: 0,
        tier: snapshot.meta.tier,
        root_name: snapshot.meta.root_name.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        usn: Some(next),
    };
    let entry = save_in(dir, &tree, &cats, &meta)?;
    let refreshed = bytemap_core::Snapshot { meta, tree, cats };
    let mut applied = apply_snapshot(shared, &entry.name, refreshed)?;
    applied["refresh"] = json!({
        "facts": stats.facts,
        "added": stats.added,
        "removed": stats.removed,
        "updated": stats.updated,
        "unresolved": helper_unresolved,
        "nodesBefore": stats.nodes_before,
        "nodesAfter": stats.nodes_after,
    });
    Ok(applied)
}

/// Core tree flags from Win32 attributes. The helper sends the current
/// metadata for existing entries, so only the interesting bits are mapped.
#[cfg(windows)]
fn fact_flags(attributes: u32, is_dir: bool) -> u16 {
    use bytemap_core::tree::flags;
    use bytemap_scan::metadata as m;

    let mut out = 0;
    if is_dir {
        out |= flags::DIR;
    }
    if attributes & m::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        out |= flags::REPARSE;
    }
    if attributes & (m::FILE_ATTRIBUTE_RECALL_ON_OPEN | m::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
    {
        out |= flags::PLACEHOLDER;
    }
    out
}

#[cfg(windows)]
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
