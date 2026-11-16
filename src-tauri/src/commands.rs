//! IPC commands. Progress travels via Channel batched to at most 20/s.
//! The frontend sends node IDs only; paths are resolved in Rust.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

use bytemap_actions::{AddError, BasketItem};
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
    basket_add_ids(session.inner(), &node_ids, include_documents.unwrap_or(false))
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
