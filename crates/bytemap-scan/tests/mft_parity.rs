//! Tier A vs Tier B parity (P6.T3b): a fixture scanned by the walker and
//! rebuilt from synthetic MFT records must produce the same tree — same
//! entry counts, names, flags, mtimes, logical sizes and size on disk.
//!
//! The records are synthesized by walking the fixture with std::fs in the
//! shape the helper sends (root record first, one record per name,
//! parent-numbered refs, cluster rounding left to the app side), so this
//! tests the tree builder end-to-end against the reference scanner. Real
//! MFT wire output is verified by the helper's own tests + the T4 gate.

#![allow(dead_code)] // fixtures.rs helpers this file does not use

use std::collections::VecDeque;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bytemap_core::tree::{flags, Tree};
use bytemap_helper::proto::Record;
use bytemap_scan::metadata as m;
use bytemap_scan::mft_tree::build_tree_from_records;
use bytemap_scan::{walker, ScanOptions};

mod fixtures;
use fixtures::Fixture;

/// Synthetic record numbers start here (above the reserved NTFS range).
const FIRST_RECORD: u64 = 100_000;

/// Walk `root` and emit the record stream the helper would send: the root
/// record first, then one record per entry, parents before children, no
/// descent into reparse directories.
fn synthesize_records(root: &Path) -> Vec<Record> {
    let mut records = Vec::new();
    let mut next = FIRST_RECORD;
    let root_number = next;
    next += 1;
    records.push(Record {
        file_ref: root_number,
        parent_ref: root_number,
        logical_size: 0,
        allocated_size: 0,
        attributes: m::FILE_ATTRIBUTE_DIRECTORY,
        mtime: 0,
        name: String::new(), // helper sends the on-disk name; app ignores it
    });

    let mut queue: VecDeque<(u64, PathBuf)> = VecDeque::new();
    queue.push_back((root_number, root.to_path_buf()));
    while let Some((parent_number, dir)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue, // parity fixture is clean; no denied dirs
        };
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let attrs = meta.file_attributes();
            let is_dir = attrs & m::FILE_ATTRIBUTE_DIRECTORY != 0;
            let number = next;
            next += 1;
            records.push(Record {
                file_ref: number,
                parent_ref: parent_number,
                logical_size: if is_dir { 0 } else { meta.len() },
                allocated_size: 0, // the app computes size on disk
                attributes: attrs,
                mtime: meta.modified().map(m::filetime).unwrap_or(0),
                name: entry.file_name().to_string_lossy().into_owned(),
            });
            if is_dir && attrs & m::FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                queue.push_back((number, entry.path()));
            }
        }
    }
    records
}

/// Compare two trees structurally: children sorted by name (insertion
/// order differs between tiers), comparing name, flags, mtime and the
/// rolled-up aggregates.
fn assert_trees_equal(left: &Tree, right: &Tree) {
    let mut stack: Vec<(u32, u32)> = Vec::new();
    stack.push((left.root().unwrap(), right.root().unwrap()));
    while let Some((a, b)) = stack.pop() {
        assert_eq!(left.name(a), right.name(b), "node name");
        assert_eq!(left.node(a).flags(), right.node(b).flags(), "flags of {a}");
        assert_eq!(left.node(a).mtime(), right.node(b).mtime(), "mtime of {a}");
        assert_eq!(
            left.node(a).agg_logical(),
            right.node(b).agg_logical(),
            "logical of {}",
            left.name(a)
        );
        assert_eq!(
            left.node(a).agg_on_disk(),
            right.node(b).agg_on_disk(),
            "on-disk of {}",
            left.name(a)
        );

        let mut ka: Vec<u32> = left.children(a).collect();
        let mut kb: Vec<u32> = right.children(b).collect();
        assert_eq!(ka.len(), kb.len(), "child count of {}", left.name(a));
        let key = |t: &Tree, id: u32| (t.name(id).to_lowercase(), id);
        ka.sort_by_cached_key(|&id| key(left, id));
        kb.sort_by_cached_key(|&id| key(right, id));
        for (&ca, &cb) in ka.iter().zip(kb.iter()) {
            stack.push((ca, cb));
        }
    }
}

#[test]
fn mft_tree_matches_the_walker_tree() {
    let fixture = Fixture::empty("mft-parity");
    fixture.file("big.zip", 100_000);
    fixture.file("small.txt", 17);
    fixture.file("nested/inner/data.bin", 4_096);
    fixture.file("empty-target/nothing", 0);
    assert!(fixture.junction("link", "empty-target"), "mklink /J");
    assert!(fixture.hardlink("big.zip", "nested/hardlink-of-big.zip"));

    // Tier B: the reference scan.
    let mut walker_tree = Tree::new();
    let walker_stats = walker::scan_folder(
        fixture.path(),
        &mut walker_tree,
        &ScanOptions::default(),
        None,
    )
    .expect("walker scan");

    // Tier A: same fixture rebuilt from synthetic records.
    let records = synthesize_records(fixture.path());
    let (mft_tree, mft_stats) =
        build_tree_from_records(fixture.path(), records).expect("mft tree build");

    assert_eq!(
        walker_stats.files, mft_stats.files,
        "file count (hardlinks counted per name, dedup off)"
    );
    assert_eq!(walker_stats.dirs, mft_stats.dirs, "dir count");
    assert_eq!(walker_stats.denied, mft_stats.denied, "denied count");
    assert_trees_equal(&walker_tree, &mft_tree);

    // Spot-check the interesting nodes directly.
    let wroot = walker_tree.root().unwrap();
    let mroot = mft_tree.root().unwrap();
    let wdocs = walker_tree
        .find_child(wroot, "nested")
        .and_then(|n| walker_tree.find_child(n, "inner"))
        .expect("walker nested/inner");
    let mdocs = mft_tree
        .find_child(mroot, "nested")
        .and_then(|n| mft_tree.find_child(n, "inner"))
        .expect("mft nested/inner");
    let mnested = mft_tree.find_child(mroot, "nested").expect("mft nested");
    assert_eq!(
        walker_tree.node(wdocs).agg_logical(),
        mft_tree.node(mdocs).agg_logical()
    );
    // The hardlink of big.zip lives in nested/: full size counted twice.
    assert_eq!(mft_tree.node(mnested).agg_logical(), 100_000 + 4_096);
    // Root flags: both tiers mark it a plain directory.
    assert_eq!(walker_tree.node(wroot).flags() & flags::DIR, flags::DIR);
    assert_eq!(mft_tree.node(mroot).flags() & flags::DIR, flags::DIR);
}
