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
use bytemap_core::Tree;
use bytemap_scan::{scan_folder, ScanError, ScanOptions, ScanProgress, ScanStats};

#[cfg(windows)]
use bytemap_scan::mft_client::{is_volume_root, MftClient, Record, WireFrame, DEFAULT_PIPE_NAME};
#[cfg(windows)]
use bytemap_scan::mft_tree::build_tree_from_records;

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
    use_mft: Option<bool>,
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
    // Tier A (elevated MFT helper) is opt-in per scan; the frontend sends
    // `useMft`. Default off until the helper is spawned by the app itself
    // (P6.T4): a manually started helper still enables the fast tier.
    let use_mft = use_mft.unwrap_or(false);

    std::thread::spawn(move || {
        run_scan_job(path, options, use_mft, shared, on_event);
    });
    Ok(())
}
/// The scan job itself, separated from the command so integration tests can
/// drive the whole flow (start -> throttled progress -> complete -> view)
/// without a webview.
///
/// `use_mft` requests the Tier A scan through the elevated helper. ANY MFT
/// failure (helper not running, protocol/stream error, tree build error)
/// automatically falls back to the walker; the complete event then carries
/// `"tier": "walker"` plus a `"fallbackReason"` string.
pub fn run_scan_job(
    path: std::path::PathBuf,
    options: ScanOptions,
    use_mft: bool,
    shared: SharedSession,
    on_event: Channel,
) {
    // New scan: the root anchors node-id -> path resolution, and node ids
    // from any previous scan are invalid, so the basket must be emptied.
    {
        let mut s = shared.lock().unwrap();
        s.root_path = path.clone();
        s.basket.clear();
        // A live scan replaces any loaded snapshot: clear the banner and
        // stale scan metadata until this run completes.
        s.loaded_snapshot = None;
        s.scan_tier = None;
        s.scan_ms = 0;
    }
    let throttle = std::sync::Mutex::new(Throttle::new(Duration::from_millis(50)));
    let progress_channel = on_event.clone();
    let progress: &dyn Fn(ScanProgress) = &move |p| {
        if throttle.lock().unwrap().ready(Instant::now()) {
            let body = InvokeResponseBody::Json(progress_event(p).to_string());
            let _ = progress_channel.send(body);
        }
    };

    let outcome = scan_with_fallback(&path, &options, use_mft, &shared, Some(progress));

    let event = match outcome {
        ScanOutcome::Complete(tree, stats, tier, fallback) => {
            let root_id = tree.root();
            // Categories/tiers computed once per completed scan (Phase 4).
            let cats = bytemap_core::categorize(&tree, &bytemap_core::Rules::default_rules());
            let mut s = shared.lock().unwrap();
            s.tree = tree;
            s.cats = cats;
            s.running = false;
            // Snapshot metadata: which tier produced this tree and how long
            // it took. Saving later reads these, not the complete event.
            s.scan_tier = Some(if tier == "mft" {
                bytemap_core::ScanTier::Mft
            } else {
                bytemap_core::ScanTier::Walker
            });
            s.scan_ms = stats.elapsed_ms as u64;
            s.loaded_snapshot = None;
            let mut event = json!({
                "type": "complete",
                "tier": tier,
                "rootId": root_id,
                "files": stats.files,
                "dirs": stats.dirs,
                "denied": stats.denied,
                "elapsedMs": stats.elapsed_ms as u64,
            });
            if let Some(reason) = fallback {
                event["fallbackReason"] = json!(reason);
            }
            event
        }
        ScanOutcome::Cancelled => {
            shared.lock().unwrap().running = false;
            json!({ "type": "cancelled" })
        }
        ScanOutcome::Error(message) => {
            shared.lock().unwrap().running = false;
            json!({ "type": "error", "message": message })
        }
    };
    let body = InvokeResponseBody::Json(event.to_string());
    let _ = on_event.send(body);
}

/// Result of one scan attempt, already tier-tagged.
enum ScanOutcome {
    Complete(Tree, ScanStats, &'static str, Option<String>),
    Cancelled,
    Error(String),
}

/// Tier A attempt, walker fallback: the front door of `run_scan_job`.
fn scan_with_fallback(
    path: &std::path::Path,
    options: &ScanOptions,
    use_mft: bool,
    shared: &SharedSession,
    progress: Option<&dyn Fn(ScanProgress)>,
) -> ScanOutcome {
    #[cfg(windows)]
    if use_mft {
        // The MFT tier pays off only on a whole volume (P6.T4d): a folder
        // subtree costs a whole-$MFT pass either way, so the walker wins
        // there and this path never prompts for UAC.
        if !is_volume_root(&path.to_string_lossy()) {
            return match walker_scan(path, options, progress) {
                Ok((tree, stats)) => ScanOutcome::Complete(
                    tree,
                    stats,
                    "walker",
                    Some(
                        "the MFT tier scans whole volumes only; folder scans use the walker"
                            .to_owned(),
                    ),
                ),
                Err(ScanError::Cancelled) => ScanOutcome::Cancelled,
                Err(e) => ScanOutcome::Error(e.to_string()),
            };
        }
        // Tier A needs its helper: connect-or-spawn (UAC on first use).
        // Failure to start it is just a fallback reason, not a scan error.
        if let Err(spawn_error) = bytemap_scan::spawn::ensure_helper() {
            return match walker_scan(path, options, progress) {
                Ok((tree, stats)) => {
                    ScanOutcome::Complete(tree, stats, "walker", Some(spawn_error))
                }
                Err(ScanError::Cancelled) => ScanOutcome::Cancelled,
                Err(e) => ScanOutcome::Error(e.to_string()),
            };
        }
        match mft_scan(path, shared, progress) {
            Ok((tree, stats)) => return ScanOutcome::Complete(tree, stats, "mft", None),
            Err(MftFailure::Cancelled) => return ScanOutcome::Cancelled,
            Err(MftFailure::Failed(reason)) => {
                // The walker takes over; the reason travels to the UI.
                return match walker_scan(path, options, progress) {
                    Ok((tree, stats)) => ScanOutcome::Complete(tree, stats, "walker", Some(reason)),
                    Err(ScanError::Cancelled) => ScanOutcome::Cancelled,
                    Err(e) => ScanOutcome::Error(e.to_string()),
                };
            }
        }
    }
    let _ = (use_mft, shared); // non-Windows: the MFT tier does not exist
    match walker_scan(path, options, progress) {
        Ok((tree, stats)) => ScanOutcome::Complete(tree, stats, "walker", None),
        Err(ScanError::Cancelled) => ScanOutcome::Cancelled,
        Err(e) => ScanOutcome::Error(e.to_string()),
    }
}

/// Tier B attempt: the reference scanner (jwalk + rayon).
fn walker_scan(
    path: &std::path::Path,
    options: &ScanOptions,
    progress: Option<&dyn Fn(ScanProgress)>,
) -> Result<(Tree, ScanStats), ScanError> {
    let mut tree = Tree::new();
    let stats = scan_folder(path, &mut tree, options, progress)?;
    Ok((tree, stats))
}

/// Why the MFT tier gave up; the walker takes over with this reason.
#[cfg(windows)]
enum MftFailure {
    Cancelled,
    Failed(String),
}

/// Tier A attempt: connect to the elevated helper, stream its Record
/// frames, build the arena tree. Any failure becomes `MftFailure` — the
/// helper being absent is the normal case, not a panic path.
#[cfg(windows)]
fn mft_scan(
    path: &std::path::Path,
    shared: &SharedSession,
    progress: Option<&dyn Fn(ScanProgress)>,
) -> Result<(Tree, ScanStats), MftFailure> {
    let started = Instant::now();
    let cancel = shared.lock().unwrap().cancel.clone();

    // 1. Connect + request. Unreachable helper = the everyday fallback.
    let mut client =
        MftClient::connect(DEFAULT_PIPE_NAME).map_err(|e| MftFailure::Failed(e.to_string()))?;
    let root_text = path.to_string_lossy().into_owned();
    client
        .request_scan(&root_text)
        .map_err(|e| MftFailure::Failed(format!("helper pipe write failed: {e}")))?;

    // 2. Stream + build: the builder consumes the frame iterator directly,
    //    so records are neither buffered nor visited twice. Progress and
    //    the cooperative cancel flag are checked inside the iterator.
    let mut records = RecordStream {
        client,
        cancel,
        error: None,
        cancelled: false,
        done: false,
        emitted: 0,
        since_progress: 0,
        stream_start: Instant::now(),
        progress,
    };
    let built = build_tree_from_records(path, &mut records);
    if records.cancelled {
        return Err(MftFailure::Cancelled);
    }
    if let Some(error) = records.error.take() {
        return Err(MftFailure::Failed(error));
    }
    let (tree, mut stats) = built.map_err(|e| MftFailure::Failed(e.to_string()))?;
    stats.elapsed_ms = started.elapsed().as_millis();
    Ok((tree, stats))
}

/// Iterator over the helper's Record frames. Stops on `Done`, `Error`,
/// an unexpected frame, an IO error or the cancel flag; the reason is
/// stored in `error` (None after a clean `Done`, `cancelled` set when the
/// caller asked to cancel). Emits throttled-caller progress every 8192
/// records.
#[cfg(windows)]
struct RecordStream<'a> {
    client: MftClient,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    error: Option<String>,
    cancelled: bool,
    done: bool,
    emitted: u64,
    since_progress: u64,
    stream_start: Instant,
    progress: Option<&'a dyn Fn(ScanProgress)>,
}

#[cfg(windows)]
impl Iterator for RecordStream<'_> {
    type Item = Record;

    fn next(&mut self) -> Option<Record> {
        if self.done {
            return None;
        }
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            self.cancelled = true;
            self.done = true;
            return None;
        }
        match self.client.next_frame() {
            Ok(WireFrame::Record(record)) => {
                self.emitted += 1;
                self.since_progress += 1;
                if self.since_progress >= 8192 {
                    self.since_progress = 0;
                    if let Some(progress) = self.progress {
                        progress(ScanProgress {
                            files: self.emitted,
                            dirs: 0,
                            denied: 0,
                            elapsed_ms: self.stream_start.elapsed().as_millis(),
                        });
                    }
                }
                Some(record)
            }
            Ok(WireFrame::Done { .. }) => {
                self.done = true;
                None
            }
            Ok(WireFrame::Error { message }) => {
                self.error = Some(message);
                self.done = true;
                None
            }
            Ok(other) => {
                self.error = Some(format!("unexpected frame from helper: {other:?}"));
                self.done = true;
                None
            }
            Err(e) => {
                self.error = Some(format!("helper pipe read failed: {e}"));
                self.done = true;
                None
            }
        }
    }
}

#[tauri::command]
pub fn cancel_scan(session: State<'_, SharedSession>) {
    session
        .lock()
        .unwrap()
        .cancel
        .store(true, Ordering::Relaxed);
}

/// Make sure the elevated helper is serving (P6.T4b): connect first, and
/// if no helper answers, start one with the `runas` verb (one UAC prompt
/// per session). `start_scan` with `useMft` calls the same function.
#[tauri::command]
pub fn ensure_helper() -> Result<Value, String> {
    #[cfg(windows)]
    {
        let status = bytemap_scan::spawn::ensure_helper()?;
        Ok(json!({ "status": status.as_str() }))
    }
    #[cfg(not(windows))]
    {
        Err("the MFT helper is Windows-only".to_owned())
    }
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

/// Strata (layered treemap) layout for `node_id`: rectangle area = share
/// of the parent's aggregate size, nested `depth` layers (default 3).
/// Travels as packed little-endian bytes via ipc::Response.
#[tauri::command]
pub fn node_strata(
    node_id: u32,
    depth: Option<u16>,
    max_rects: Option<u32>,
    session: State<'_, SharedSession>,
) -> tauri::ipc::Response {
    let depth = depth.unwrap_or(3).clamp(1, 4);
    let bytes = {
        let s = session.lock().unwrap();
        view::strata_bytes(&s.tree, &s.cats, node_id, depth, max_rects.unwrap_or(0))
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

/// Dry run of the current basket: re-checks every path (size + mtime vs
/// the scan snapshot, in-use via Restart Manager) and reports status.
/// NEVER mutates anything. UI shows this before the user executes.
pub fn dry_run_core(shared: &SharedSession) -> Value {
    let s = shared.lock().unwrap();
    let items: Vec<BasketItem> = s.basket.items().to_vec();
    if items.is_empty() {
        return json!({ "entries": [], "count": 0 });
    }
    let entries = dry_run(&items, &rm_probe);
    let json_entries: Vec<Value> = entries
        .into_iter()
        .map(|e| {
            json!({
                "nodeId": e.node_id,
                "path": e.path,
                "logicalScan": e.logical_scan,
                "sizeNow": e.size_now,
                "mtimeScan": e.mtime_scan,
                "mtimeNow": e.mtime_now,
                "tier": e.tier,
                "cat": e.cat,
                "inUse": e.in_use,
                "probeError": e.probe_error,
                "status": match e.status {
                    StaleStatus::Ok => "ok",
                    StaleStatus::Changed => "changed",
                    StaleStatus::Missing => "missing",
                },
            })
        })
        .collect();
    json!({ "entries": json_entries, "count": json_entries.len() })
}

#[tauri::command]
pub fn dry_run_basket(session: State<'_, SharedSession>) -> Value {
    dry_run_core(session.inner())
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
