//! Full-flow integration test without a webview:
//! run_scan_job -> throttled JSON progress -> complete -> binary view bytes.
//!
//! Scans the reusable benchmark volume %TEMP%\bytemap-bench-100k (created by
//! `cargo run --release -p bytemap-scan --bin bench-walk`). If it does not
//! exist, the test skips LOUDLY (never silently) — build it with the bench
//! command and re-run.

use std::sync::{Arc, Mutex};

use bytemap_lib::commands::run_scan_job;
use bytemap_lib::state::SharedSession;
use bytemap_lib::view::view_bytes;
use bytemap_scan::ScanOptions;
use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody};

#[test]
fn full_flow_scan_progress_complete_view() {
    let root = std::env::temp_dir().join("bytemap-bench-100k");
    if !root.is_dir() {
        eprintln!(
            "SKIPPED LOUDLY: {} does not exist. Run `cargo run --release -p bytemap-scan --bin bench-walk` first.",
            root.display()
        );
        return;
    }

    let shared: SharedSession = Arc::new(Mutex::new(bytemap_lib::state::Session::default()));
    let messages: Arc<Mutex<Vec<InvokeResponseBody>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&messages);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        collector.lock().unwrap().push(body);
        Ok(())
    });

    let options = ScanOptions::default();
    run_scan_job(root, options, Arc::clone(&shared), channel);

    // --- every message parses as JSON ---
    let raw = messages.lock().unwrap();
    assert!(
        raw.len() >= 2,
        "expected progress + complete, got {}",
        raw.len()
    );
    let parsed: Vec<Value> = raw
        .iter()
        .map(|body| match body {
            InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(s)
                .unwrap_or_else(|e| panic!("message is not valid JSON: {e}")),
            InvokeResponseBody::Raw(_) => panic!("unexpected raw (non-JSON) channel message"),
        })
        .collect();
    drop(raw);

    // --- progress throttled to <= 20/s (50ms window) ---
    let elapsed_total_ms = parsed.last().unwrap()["elapsedMs"].as_u64().unwrap_or(0);
    let progress_count = parsed.iter().filter(|m| m["type"] == "progress").count() as u64;
    // Window 50ms => <= elapsed/50 + slack(1 for the first immediate emission).
    let max_allowed = elapsed_total_ms / 50 + 1;
    assert!(
        progress_count <= max_allowed,
        "throttling broken: {progress_count} progress messages in {elapsed_total_ms} ms (max {max_allowed})"
    );

    // --- last message is complete with a real rootId ---
    let complete = parsed.last().unwrap();
    assert_eq!(
        complete["type"], "complete",
        "last message must be complete"
    );
    let root_id = complete["rootId"]
        .as_u64()
        .expect("complete carries rootId") as u32;

    // --- the session tree survived, and view_bytes decodes in our format ---
    let s = shared.lock().unwrap();
    assert!(!s.running, "session must be idle after complete");
    let bytes = view_bytes(&s.tree, &s.cats, root_id, 200);
    drop(s);
    let mut d = ViewDecoder::new(&bytes);
    assert_eq!(d.u16(), 2, "view version (v2: cat + tier per item)");
    let ancestors = d.u16();
    let requested = d.u32();
    assert_eq!(requested, root_id, "view is for the scanned root");
    for _ in 0..ancestors {
        d.u32(); // ancestor id
        d.str(); // ancestor name
    }
    let total_children = d.u32();
    let item_count = d.u32();
    assert_eq!(
        item_count,
        total_children.min(200),
        "top-200 view of the root"
    );
    for _ in 0..item_count {
        d.u32(); // id
        d.u64(); // logical
        d.u64(); // on_disk
        d.u16(); // flags
        d.u8(); // category (v2)
        d.u8(); // tier (v2)
        d.str(); // name
    }
    assert!(d.at_end(), "view buffer fully consumed");
}

/// Minimal little-endian reader for the bytemap-core view format.
struct ViewDecoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ViewDecoder<'a> {
    fn new(buf: &'a [u8]) -> Self {
        ViewDecoder { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        assert!(self.pos + n <= self.buf.len(), "view buffer truncated");
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    fn str(&mut self) -> String {
        let len = self.u16() as usize;
        String::from_utf8(self.take(len).to_vec()).unwrap()
    }
    fn at_end(&self) -> bool {
        self.pos == self.buf.len()
    }
}
