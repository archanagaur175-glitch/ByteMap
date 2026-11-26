//! P8.T5 reopen gate: a synthetic 1M-node tree must save and reload in
//! under one second in RELEASE mode.
//!
//! The timed test is `#[ignore]` on purpose: an unoptimized zstd + tree pass
//! measures the compiler, not the format, so it must not slow down the normal
//! `cargo test --workspace` gate. The documented command is
//!
//! ```text
//! cargo test --release -p bytemap-core --test reopen_1m -- --ignored --nocapture
//! ```
//!
//! and the printed `REOPEN-1M ...` line is the measured number recorded in
//! docs/agent/SESSION_LOG.md. A small, always-on round-trip test guards the
//! same code path in every build.

use std::time::Instant;

use bytemap_core::categorizer::{categorize, Rules};
use bytemap_core::tree::flags;
use bytemap_core::{
    load_snapshot, recompute_rollups, save_snapshot, ScanTier, Snapshot, SnapshotMeta, Tree,
};

/// Directories under the root (each one holds `FILES_PER_DIR` files).
const DIRS: usize = 1_000;
/// Files per directory: root + DIRS + DIRS*FILES_PER_DIR = 1,001,001 nodes.
const FILES_PER_DIR: usize = 1_000;

/// Build the synthetic volume: root -> 1,000 folders -> 1,000 files each.
fn build_tree() -> Tree {
    let mut tree = Tree::new();
    let root = tree.insert_root(r"C:\");
    for d in 0..DIRS {
        let dir = tree.insert_child(root, &format!("folder-{d:04}"), flags::DIR, 0, 0, 0);
        for f in 0..FILES_PER_DIR {
            // Sizes vary so compression has real work to do.
            let logical = ((d * 7 + f * 13) % 100_000 + 1) as u64;
            let on_disk = logical.div_ceil(4096) * 4096;
            tree.insert_child(
                dir,
                &format!("file-{f:04}.bin"),
                0,
                133_000_000_000_000_000 + (d * 1_000 + f) as i64,
                logical,
                on_disk,
            );
        }
    }
    tree
}

fn snapshot(tree: Tree, root_name: &str) -> Snapshot {
    let cats = categorize(&tree, &Rules::default_rules());
    Snapshot {
        meta: SnapshotMeta {
            created_unix: 1_700_000_000,
            scan_ms: 1_234,
            tier: ScanTier::Walker,
            root_name: root_name.to_string(),
            app_version: "reopen-gate".to_string(),
            usn: None,
        },
        tree,
        cats,
    }
}

/// Always-on sanity: the same save -> load path on a small tree, every build.
#[test]
fn a_small_tree_round_trips() {
    let mut tree = Tree::new();
    let root = tree.insert_root(r"C:\tiny");
    let dir = tree.insert_child(root, "sub", flags::DIR, 0, 0, 0);
    tree.insert_child(dir, "a.bin", 0, 1, 42, 4096);
    recompute_rollups(&mut tree);
    let built = tree.len();

    let bytes = save_snapshot(&snapshot(tree, r"C:\tiny")).expect("save");
    let loaded = load_snapshot(&bytes).expect("load");
    assert_eq!(loaded.tree.len(), built);
    assert_eq!(loaded.cats.cat.len(), built);
    assert_eq!(loaded.meta.root_name, r"C:\tiny");
    assert_eq!(loaded.meta.usn, None);
}

/// The gate. Run in release mode with `--ignored --nocapture`.
#[test]
#[ignore = "1M-node release gate: cargo test --release -p bytemap-core --test reopen_1m -- --ignored --nocapture"]
fn one_million_nodes_save_and_reopen_under_one_second() {
    let started = Instant::now();
    let mut tree = build_tree();
    recompute_rollups(&mut tree);
    let nodes = tree.len();
    assert_eq!(nodes, 1 + DIRS + DIRS * FILES_PER_DIR);
    let build_ms = started.elapsed().as_millis();

    let save_started = Instant::now();
    let bytes = save_snapshot(&snapshot(tree, r"C:\")).expect("save 1M nodes");
    let save_ms = save_started.elapsed().as_millis();
    assert!(bytes.len() > 1_000_000, "a 1M-node body must not be tiny");

    // Write + read the real file: the gate is "reopen", not "decompress".
    let path = std::env::temp_dir().join(format!("bytemap-reopen-1m-{}.bms", std::process::id()));
    std::fs::write(&path, &bytes).expect("write snapshot file");

    let reopen_started = Instant::now();
    let on_disk = std::fs::read(&path).expect("read snapshot file");
    let loaded = load_snapshot(&on_disk).expect("load 1M nodes");
    let reopen_ms = reopen_started.elapsed().as_millis();
    let _ = std::fs::remove_file(&path);

    assert_eq!(loaded.tree.len(), nodes, "every node must come back");
    assert_eq!(loaded.cats.cat.len(), nodes);
    assert_eq!(loaded.meta.created_unix, 1_700_000_000);
    // Spot-check that sizes survived (not just the node count).
    let root = loaded.tree.root().expect("root");
    let first_dir = loaded.tree.children(root).next().expect("first folder");
    assert!(
        loaded.tree.node(first_dir).agg_logical() > 0,
        "first_dir={first_dir} flags=0x{:x} agg={} root_agg={} kids={}",
        loaded.tree.node(first_dir).flags(),
        loaded.tree.node(first_dir).agg_logical(),
        loaded.tree.node(root).agg_logical(),
        loaded.tree.children(first_dir).count()
    );

    println!(
        "REOPEN-1M nodes={nodes} bytes={} build_ms={build_ms} save_ms={save_ms} reopen_ms={reopen_ms}",
        bytes.len()
    );
    assert!(
        reopen_ms < 1_000,
        "reopen gate FAILED: {reopen_ms} ms for {nodes} nodes (target < 1000 ms)"
    );
}
