//! IPC commands. Progress travels via Channel batched to at most 20/s.
//! The frontend sends node IDs only; paths are resolved in Rust.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

use bytemap_actions::dry_run;
#[cfg(windows)]
use bytemap_actions::inuse;
use bytemap_actions::journal::{Journal, JournalEntry, JournalItem};
use bytemap_actions::recycle::{permanent_delete_items, recycle_items, ActionOutcome};
use bytemap_actions::{
    archive_items, restore_archive, undo_recycle, AddError, BasketItem, StaleStatus,
};
use bytemap_core::categorizer::category;
use bytemap_scan::{scan_folder, ScanError, ScanOptions, ScanProgress};

use crate::pathres;
use crate::state::SharedSession;
use crate::view;

/// Batches progress updates: at most one emission per `min_interval`.
/// Pure with respect to the injected `now`, so it is unit-testable.
pub struct Throttle {
    min_interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(min_interval: Duration) -> Self {
        Throttle {
            min_interval,
            last: None,
        }
    }

    pub fn ready(&mut self, now: Instant) -> bool {
        let emit = match self.last {
            None => true,
            Some(last) => now.duration_since(last) >= self.min_interval,
        };
        if emit {
            self.last = Some(now);
        }
        emit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn throttle_limits_to_20_per_second() {
        let base = Instant::now();
        let mut t = Throttle::new(Duration::from_millis(50));
        assert!(t.ready(base), "first emission always allowed");
        assert!(!t.ready(base + Duration::from_millis(10)));
        assert!(!t.ready(base + Duration::from_millis(49)));
        assert!(
            t.ready(base + Duration::from_millis(50)),
            "next allowed after 50ms"
        );
        assert!(!t.ready(base + Duration::from_millis(99)));
        assert!(t.ready(base + Duration::from_millis(100)));
        // 3 emissions in 100ms window => max 20/s respected.
    }

    /// Full execute path on a real fixture: basket -> dry run (one item
    /// dropped as changed) -> recycle -> journal -> basket cleared.
    /// Writes one small file to the Recycle Bin (harmless, fixture only).
    #[test]
    fn execute_recycle_drops_changed_and_journals() {
        // Isolate the undo journal for this test run.
        std::env::set_var("BYTEMAP_UNDO_DIR", temp_dir().join("bytemap-exec-journal"));

        let dir = temp_dir().join("bytemap-exec-fixture");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("gone.dat");
        let stay = dir.join("stay.dat");
        std::fs::write(&gone, vec![0u8; 10]).unwrap();
        std::fs::write(&stay, vec![0u8; 10]).unwrap();

        let shared: SharedSession = Arc::new(Mutex::new(crate::state::Session::default()));
        {
            let mut s = shared.lock().unwrap();
            s.root_path = dir.clone();
            for (id, p, mtime) in [
                (1u32, gone.clone(), real_mtime(&gone)),
                (2, stay.clone(), 0), // stale mtime -> must be dropped
            ] {
                s.basket
                    .add(BasketItem {
                        node_id: id,
                        path: p.to_string_lossy().into_owned(),
                        name: p.file_name().unwrap().to_string_lossy().into_owned(),
                        logical: 10,
                        on_disk: 10,
                        mtime,
                        tier: 0, // SAFE
                        cat: 6,
                    })
                    .unwrap();
            }
        }

        let report = execute_action_core(&shared, "recycle", None);
        assert_eq!(
            report["executed"].as_array().unwrap().len(),
            1,
            "report: {report}"
        );
        assert_eq!(report["dropped"].as_array().unwrap().len(), 1);
        assert_eq!(
            report["dropped"][0]["reason"], "changed",
            "report: {report}"
        );
        assert!(report["journaled"].as_bool().unwrap());
        assert!(!gone.exists(), "recycled -> original gone");
        assert!(stay.exists(), "changed item must NOT be touched");

        // Basket: executed item left, dropped item stays for the UI to show.
        let s = shared.lock().unwrap();
        assert_eq!(s.basket.len(), 1);
        assert_eq!(s.basket.items()[0].node_id, 2);
        drop(s);

        // Journal recorded the recycle.
        let j = Journal::open().unwrap();
        let last = j.last().unwrap().expect("journal line");
        assert_eq!(last.action, "recycle");
        assert_eq!(last.items[0].path, gone.to_string_lossy());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn real_mtime(p: &std::path::Path) -> i64 {
        let modified = std::fs::metadata(p).ok().and_then(|md| md.modified().ok());
        bytemap_actions::dryrun::system_time_to_filetime(&modified)
            .expect("fixture file has an mtime")
    }

    /// Archive end-to-end: execute_action_core(mode "archive") writes a
    /// verified container, removes the original, journals the container
    /// location — and undo_last_core restores the file byte-for-byte.
    #[test]
    fn archive_execute_then_undo_restores() {
        std::env::set_var("BYTEMAP_UNDO_DIR", temp_dir().join("bytemap-arch-journal"));
        std::env::set_var(
            "BYTEMAP_ARCHIVE_DIR",
            temp_dir().join("bytemap-arch-containers"),
        );

        let dir = temp_dir().join("bytemap-arch-fixture");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("data.bin");
        let payload: Vec<u8> = (0..50_000u32).map(|i| (i % 249) as u8).collect();
        std::fs::write(&f, &payload).unwrap();

        let shared: SharedSession = Arc::new(Mutex::new(crate::state::Session::default()));
        {
            let mut s = shared.lock().unwrap();
            s.root_path = dir.clone();
            s.basket
                .add(BasketItem {
                    node_id: 1,
                    path: f.to_string_lossy().into_owned(),
                    name: "data.bin".to_string(),
                    logical: payload.len() as u64,
                    on_disk: payload.len() as u64,
                    mtime: real_mtime(&f),
                    tier: 0,
                    cat: 6,
                })
                .unwrap();
        }

        let report = execute_action_core(&shared, "archive", None);
        assert_eq!(
            report["executed"].as_array().unwrap().len(),
            1,
            "report: {report}"
        );
        assert!(report["journaled"].as_bool().unwrap(), "report: {report}");
        assert!(!f.exists(), "original removed after verified archive");
        assert_eq!(shared.lock().unwrap().basket.len(), 0);

        // Journal records the archive + container location.
        let j = Journal::open().unwrap();
        let last = j.last().unwrap().expect("journal line");
        assert_eq!(last.action, "archive");
        let container = last.archive.clone().expect("container recorded");
        assert!(std::path::Path::new(&container).exists());

        // Undo: restores byte-for-byte.
        let rep = undo_last_core();
        assert_eq!(rep["undoable"], true, "rep: {rep}");
        assert_eq!(rep["restored"].as_array().unwrap().len(), 1, "rep: {rep}");
        assert!(f.exists(), "undo restored the file");
        assert_eq!(std::fs::read(&f).unwrap(), payload, "byte-for-byte");

        // Redundant belt-and-braces: direct restore is idempotent-check via
        // conflicts (file now exists again -> would be a conflict).
        let direct = bytemap_actions::restore_archive(std::path::Path::new(&container)).unwrap();
        assert_eq!(direct.restored.len(), 0);
        assert_eq!(direct.conflicts.len(), 1, "never overwrites: {direct:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Channel message helper: progress is throttled upstream (here), never
/// emitted per file (PROJECT_BRIEF rule).
fn progress_event(p: ScanProgress) -> Value {
    json!({
        "type": "progress",
        "files": p.files,
        "dirs": p.dirs,
        "denied": p.denied,
        "elapsedMs": p.elapsed_ms as u64,
    })
}

#[tauri::command]
pub fn start_scan(
    root: String,
    session: State<'_, SharedSession>,
    on_event: Channel,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&root);
    if !path.is_dir() {
        return Err(format!("not a directory: {root}"));
    }

    let shared: SharedSession = session.inner().clone();
    {
        let mut s = shared.lock().unwrap();
        if s.running {
            return Err("a scan is already running".to_string());
        }
        s.running = true;
        s.cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        s.tree = bytemap_core::Tree::new();
    }
    let cancel = shared.lock().unwrap().cancel.clone();
    let options = ScanOptions {
        cancel: Some(cancel),
        ..Default::default()
    };

    std::thread::spawn(move || {
        run_scan_job(path, options, shared, on_event);
    });
    Ok(())
}
/// The scan job itself, separated from the command so integration tests can
/// drive the whole flow (start -> throttled progress -> complete -> view)
/// without a webview.
pub fn run_scan_job(
    path: std::path::PathBuf,
    options: ScanOptions,
    shared: SharedSession,
    on_event: Channel,
) {
    // New scan: the root anchors node-id -> path resolution, and node ids
    // from any previous scan are invalid, so the basket must be emptied.
    {
        let mut s = shared.lock().unwrap();
        s.root_path = path.clone();
        s.basket.clear();
    }
    let throttle = std::sync::Mutex::new(Throttle::new(Duration::from_millis(50)));
    let progress_channel = on_event.clone();
    let progress: &dyn Fn(ScanProgress) = &move |p| {
        if throttle.lock().unwrap().ready(Instant::now()) {
            let body = InvokeResponseBody::Json(progress_event(p).to_string());
            let _ = progress_channel.send(body);
        }
    };
    let mut tree = bytemap_core::Tree::new();
    let result = scan_folder(&path, &mut tree, &options, Some(progress));

    let event = match result {
        Ok(stats) => {
            let root_id = tree.root();
            // Categories/tiers computed once per completed scan (Phase 4).
            let cats = bytemap_core::categorize(&tree, &bytemap_core::Rules::default_rules());
            let mut s = shared.lock().unwrap();
            s.tree = tree;
            s.cats = cats;
            s.running = false;
            json!({
                "type": "complete",
                "rootId": root_id,
                "files": stats.files,
                "dirs": stats.dirs,
                "denied": stats.denied,
                "elapsedMs": stats.elapsed_ms as u64,
            })
        }
        Err(ScanError::Cancelled) => {
            shared.lock().unwrap().running = false;
            json!({ "type": "cancelled" })
        }
        Err(e) => {
            shared.lock().unwrap().running = false;
            json!({ "type": "error", "message": e.to_string() })
        }
    };
    let body = InvokeResponseBody::Json(event.to_string());
    let _ = on_event.send(body);
}

#[tauri::command]
pub fn cancel_scan(session: State<'_, SharedSession>) {
    session
        .lock()
        .unwrap()
        .cancel
        .store(true, Ordering::Relaxed);
}

/// Owner/agent-visible perf report: prints the bench result line to the
/// dev process stdout (visible in the terminal running `tauri dev`).
#[tauri::command]
pub fn bench_report(line: String) {
    println!("{line}");
}

/// Binary view: ancestor chain (breadcrumb) + top-N children by size with
/// the remainder folded into one "N small items" entry. Travels as packed
/// little-endian bytes via tauri::ipc::Response (PROJECT_BRIEF rule).
#[tauri::command]
pub fn node_view(
    node_id: u32,
    limit: Option<u32>,
    session: State<'_, SharedSession>,
) -> tauri::ipc::Response {
    let limit = limit.unwrap_or(200).clamp(1, 2000);
    let bytes = {
        let s = session.lock().unwrap();
        view::view_bytes(&s.tree, &s.cats, node_id, limit)
    };
    tauri::ipc::Response::new(tauri::ipc::InvokeResponseBody::Raw(bytes))
}

/// Orbit (sunburst) layout: center = current folder, rings = depth,
/// angle = size. Travels as packed little-endian bytes via ipc::Response.
#[tauri::command]
pub fn node_orbit(
    node_id: u32,
    rings: Option<u16>,
    max_arcs: Option<u32>,
    session: State<'_, SharedSession>,
) -> tauri::ipc::Response {
    let rings = rings.unwrap_or(4).clamp(1, 8);
    let bytes = {
        let s = session.lock().unwrap();
        view::orbit_bytes(&s.tree, &s.cats, node_id, rings, max_arcs.unwrap_or(0))
    };
    tauri::ipc::Response::new(tauri::ipc::InvokeResponseBody::Raw(bytes))
}

// --- Action Hub: basket (Phase 5) ---

fn add_error_reason(e: AddError) -> &'static str {
    match e {
        AddError::Protected => "protected",
        AddError::UnknownTier => "unknown-tier",
        AddError::Duplicate => "duplicate",
    }
}

/// Stages node ids into the basket (testable core; tauri wrapper below).
/// Documents are excluded from BULK adds by default (brief: "Documents are
/// excluded from bulk actions by default"); a single-item add is not bulk,
/// so it is always allowed. `include_documents` overrides for bulk adds.
/// Node ids must belong to the most recent scan; anything else is refused.
pub fn basket_add_ids(shared: &SharedSession, node_ids: &[u32], include_documents: bool) -> Value {
    let mut s = shared.lock().unwrap();
    let bulk = node_ids.len() > 1;
    let mut added: Vec<u32> = Vec::new();
    let mut refused: Vec<Value> = Vec::new();

    for &id in node_ids {
        if id as usize >= s.tree.len() || s.root_path.as_os_str().is_empty() {
            refused.push(json!({ "nodeId": id, "reason": "invalid-node" }));
            continue;
        }
        let n = s.tree.node(id);
        let item_path = match pathres::node_path(&s.tree, &s.root_path, id) {
            Some(p) => p,
            None => {
                refused.push(json!({ "nodeId": id, "reason": "invalid-node" }));
                continue;
            }
        };
        let tier = s.cats.tier.get(id as usize).copied().unwrap_or(255);
        let cat = s.cats.cat.get(id as usize).copied().unwrap_or(6);

        if bulk && cat == category::DOCUMENTS && !include_documents {
            refused.push(json!({ "nodeId": id, "reason": "documents-default-excluded" }));
            continue;
        }

        let item = BasketItem {
            node_id: id,
            path: item_path.to_string_lossy().into_owned(),
            name: s.tree.name(id).to_string(),
            logical: n.agg_logical(),
            on_disk: n.agg_on_disk(),
            mtime: n.mtime(),
            tier,
            cat,
        };
        match s.basket.add(item) {
            Ok(()) => added.push(id),
            Err(e) => refused.push(json!({ "nodeId": id, "reason": add_error_reason(e) })),
        }
    }

    json!({
        "added": added,
        "refused": refused,
        "count": s.basket.len(),
    })
}

#[tauri::command]
pub fn basket_add(
    node_ids: Vec<u32>,
    include_documents: Option<bool>,
    session: State<'_, SharedSession>,
) -> Value {
    basket_add_ids(
        session.inner(),
        &node_ids,
        include_documents.unwrap_or(false),
    )
}

#[tauri::command]
pub fn basket_remove(node_id: u32, session: State<'_, SharedSession>) -> bool {
    session.lock().unwrap().basket.remove(node_id)
}

#[tauri::command]
pub fn basket_clear(session: State<'_, SharedSession>) {
    session.lock().unwrap().basket.clear();
}

#[tauri::command]
pub fn basket_list(session: State<'_, SharedSession>) -> Value {
    let s = session.lock().unwrap();
    let items: Vec<Value> = s
        .basket
        .items()
        .iter()
        .map(|i| {
            json!({
                "nodeId": i.node_id,
                "path": i.path,
                "name": i.name,
                "logical": i.logical,
                "onDisk": i.on_disk,
                "tier": i.tier,
                "cat": i.cat,
            })
        })
        .collect();
    json!({ "items": items, "count": items.len() })
}

// --- Action Hub: execute (Phase 5) ---

/// Real Restart Manager probe for production use: a probe failure is
/// surfaced as (false, Some(error)) — never silently treated as "free".
#[cfg(windows)]
fn rm_probe(p: &str) -> (bool, Option<String>) {
    match inuse::file_in_use(p) {
        Ok(b) => (b, None),
        Err(e) => (false, Some(e)),
    }
}

/// Non-Windows fallback (not a product target): probe unavailable.
#[cfg(not(windows))]
fn rm_probe(_p: &str) -> (bool, Option<String>) {
    (false, Some("in-use probe unavailable".to_string()))
}

/// Testable execution core: dry run -> drop changed/missing -> tier gate ->
/// execute -> append-only journal -> clear executed items from the basket.
/// `mode` is "recycle" (default), "permanent" (requires the exact typed
/// confirmation string; recycle.rs enforces it again) or "archive"
/// (compress + verify + remove originals; archive.rs enforces the gate).
pub fn execute_action_core(
    shared: &SharedSession,
    mode: &str,
    confirmation: Option<&str>,
) -> Value {
    let mut s = shared.lock().unwrap();
    if !matches!(mode, "recycle" | "permanent" | "archive") {
        return json!({ "error": format!("unknown mode {mode}") });
    }
    let items: Vec<BasketItem> = s.basket.items().to_vec();
    if items.is_empty() {
        return json!({ "executed": [], "dropped": [], "failed": [], "count": 0 });
    }

    // 1. stale-scan guard: re-check size + mtime; drop non-Ok items.
    let entries = dry_run(&items, &rm_probe);
    let mut live: Vec<BasketItem> = Vec::new();
    let mut dropped: Vec<Value> = Vec::new();
    for (entry, item) in entries.into_iter().zip(&items) {
        match entry.status {
            StaleStatus::Ok => live.push(item.clone()),
            StaleStatus::Changed => dropped.push(json!({
                "nodeId": item.node_id, "path": item.path, "reason": "changed"
            })),
            StaleStatus::Missing => dropped.push(json!({
                "nodeId": item.node_id, "path": item.path, "reason": "missing"
            })),
        }
    }

    // 2. execute (each backend re-gates tiers per item).
    // Archive: one container for the whole batch; a verify failure fails
    // the entire archive (Err) and keeps every original.
    let archive_result = if mode == "archive" {
        match archive_items(&live, &archive_dir()) {
            Ok(r) => Some(r),
            Err(e) => {
                return json!({ "error": format!("archive: {e}"), "executed": [], "dropped": dropped })
            }
        }
    } else {
        None
    };
    let outcomes = match (&archive_result, mode) {
        (Some(r), _) => r.outcomes.clone(),
        (None, "recycle") => recycle_items(&live),
        _ => permanent_delete_items(&live, confirmation),
    };

    // 3. append-only journal for what actually happened.
    let mut journaled = false;
    let ok: Vec<&ActionOutcome> = outcomes.iter().filter(|o| o.ok).collect();
    if !ok.is_empty() {
        if let Ok(journal) = Journal::open() {
            let entry = JournalEntry {
                ts_ms: now_ms(),
                action: mode.to_string(),
                items: ok
                    .iter()
                    .map(|o| JournalItem {
                        path: o.path.clone(),
                        kind: "file".to_string(),
                        logical: 0,
                    })
                    .collect(),
                // Archive undo needs the container location.
                archive: archive_result.as_ref().map(|r| r.archive_path.clone()),
            };
            journaled = journal.append(&entry).is_ok();
        }
    }

    // 4. executed items leave the basket.
    for o in &outcomes {
        if o.ok {
            s.basket.remove(o.node_id);
        }
    }

    let failed: Vec<Value> = outcomes
        .iter()
        .filter(|o| !o.ok)
        .map(|o| json!({ "nodeId": o.node_id, "path": o.path, "error": o.error }))
        .collect();
    json!({
        "executed": ok.iter().map(|o| json!({ "nodeId": o.node_id, "path": o.path })).collect::<Vec<_>>(),
        "dropped": dropped,
        "failed": failed,
        "journaled": journaled,
        "count": s.basket.len(),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Archive containers live under %LOCALAPPDATA%\ByteMap\archives (local-only
/// product); BYTEMAP_ARCHIVE_DIR overrides for tests.
fn archive_dir() -> std::path::PathBuf {
    std::env::var_os("BYTEMAP_ARCHIVE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("ByteMap")
                .join("archives")
        })
}

#[tauri::command]
pub fn execute_recycle(session: State<'_, SharedSession>) -> Value {
    execute_action_core(session.inner(), "recycle", None)
}

#[tauri::command]
pub fn execute_permanent(confirmation: String, session: State<'_, SharedSession>) -> Value {
    execute_action_core(session.inner(), "permanent", Some(&confirmation))
}

#[tauri::command]
pub fn execute_archive(session: State<'_, SharedSession>) -> Value {
    execute_action_core(session.inner(), "archive", None)
}

// --- Action Hub: undo (Phase 5) ---

/// Undo the most recent journaled action (testable core; no session state).
/// Recycle actions are undone by restoring from the Recycle Bin (matched by
/// the journaled original paths). Permanent deletes cannot be undone —
/// reported as such. Undo does NOT modify the journal (it is append-only;
/// an undo is its own action conceptually, but nothing destructive).
pub fn undo_last_core() -> Value {
    let journal = match Journal::open() {
        Ok(j) => j,
        Err(e) => {
            return json!({ "undoable": false, "reason": format!("journal unavailable: {e}") })
        }
    };
    let last = match journal.last() {
        Ok(Some(e)) => e,
        _ => return json!({ "undoable": false, "reason": "no journaled actions yet" }),
    };
    let paths: Vec<String> = last.items.iter().map(|i| i.path.clone()).collect();

    match last.action.as_str() {
        "recycle" => match undo_recycle(&paths) {
            Ok(rep) => json!({
                "undoable": true,
                "action": "recycle",
                "restored": rep.restored,
                "notFound": rep.not_found,
            }),
            Err(e) => json!({ "undoable": true, "action": "recycle", "error": e }),
        },
        "permanent" => json!({
            "undoable": false,
            "action": "permanent",
            "reason": "permanent deletes cannot be undone",
        }),
        "archive" => match last.archive.as_deref() {
            Some(container) => match restore_archive(std::path::Path::new(container)) {
                Ok(rep) => json!({
                    "undoable": true,
                    "action": "archive",
                    "restored": rep.restored,
                    "conflicts": rep.conflicts,
                }),
                Err(e) => json!({ "undoable": true, "action": "archive", "error": e }),
            },
            None => json!({
                "undoable": false,
                "action": "archive",
                "reason": "journal entry has no archive location",
            }),
        },
        other => json!({ "undoable": false, "reason": format!("unknown journal action {other}") }),
    }
}

#[tauri::command]
pub fn undo_last() -> Value {
    undo_last_core()
}
