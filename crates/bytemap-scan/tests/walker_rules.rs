//! Walker scanner rules, exercised on temporary fixture trees:
//! reparse points not followed, junctions not double-counted, hardlinks
//! counted per name until P1.T3b dedup lands, long paths, denied entries.

mod fixtures;

use bytemap_core::tree::{flags, Tree};
use bytemap_scan::{scan_folder, ScanError, ScanOptions, ScanStats};
use fixtures::Fixture;

fn scan(fx: &Fixture) -> Result<(Tree, ScanStats), ScanError> {
    let mut tree = Tree::new();
    let stats = scan_folder(fx.path(), &mut tree, &ScanOptions::default())?;
    Ok((tree, stats))
}

fn find_by_name(tree: &Tree, name: &str) -> Option<u32> {
    (0..tree.len() as u32).find(|&id| tree.name(id) == name)
}

#[test]
fn junction_is_listed_never_followed_never_double_counted() {
    let fx = Fixture::empty("junction");
    fx.file("sub/video.mkv", 2048);
    fx.file("alpha.txt", 1234);
    assert!(fx.junction("link_to_sub", "sub"), "mklink /J failed (junction creation should not need admin)");

    let (tree, stats) = scan(&fx).expect("scan should not fail on junctions");
    let link = find_by_name(&tree, "link_to_sub").expect("junction must appear as a node");
    assert_eq!(tree.node(link).flags() & flags::REPARSE, flags::REPARSE, "junction flagged REPARSE");
    assert_eq!(tree.node(link).child_count(), 0, "junction children never enumerated");

    // sub counted exactly once: root aggregate = video + alpha only.
    let root = tree.root().unwrap();
    assert_eq!(tree.node(root).agg_logical(), 1234 + 2048);
    assert_eq!(stats.files, 2);
    // dirs: root, sub, link_to_sub
    assert_eq!(stats.dirs, 3);
}

#[test]
fn self_referential_junction_does_not_hang_or_explode() {
    let fx = Fixture::empty("junction-loop");
    fx.file("f.txt", 10);
    assert!(fx.junction("loop", "."), "mklink /J failed");

    let (tree, _stats) = scan(&fx).expect("loop junction must not break the scan");
    let root = tree.root().unwrap();
    assert_eq!(tree.node(root).agg_logical(), 10, "loop target contents must not be counted");
}

#[test]
fn hardlinks_counted_per_name_before_dedup() {
    // P1.T3b note: with dedup enabled the second name must be flagged
    // DUPLICATE_HARDLINK and not add its size; until then both count.
    let fx = Fixture::empty("hardlink");
    fx.file("hard.txt", 7);
    assert!(fx.hardlink("hard.txt", "hard2.txt"), "CreateHardLinkW failed");

    let (tree, stats) = scan(&fx).expect("scan");
    assert_eq!(stats.files, 2);
    let root = tree.root().unwrap();
    assert_eq!(tree.node(root).agg_logical(), 14, "both names counted without dedup");
    let h2 = find_by_name(&tree, "hard2.txt").expect("hardlink name present");
    assert_eq!(
        tree.node(h2).flags() & flags::DUPLICATE_HARDLINK,
        0,
        "dedup flag not set before P1.T3b"
    );
}

#[test]
fn long_paths_scan_ok() {
    let fx = Fixture::empty("longpath");
    let deep = fx.deep();
    let file = deep.join("eeeeeeeeeeeeeeeeeeeeeeee.txt");
    std::fs::write(&file, vec![b'y'; 99]).expect("write deep file");

    let (tree, _stats) = scan(&fx).expect("long path scan");
    let f = find_by_name(&tree, "eeeeeeeeeeeeeeeeeeeeeeee.txt").expect("deep file found");
    assert_eq!(tree.node(f).agg_logical(), 99);
    assert!(tree.depth(f) >= 5);
}

#[test]
fn denied_entries_counted_not_fatal() {
    let fx = Fixture::empty("denied");
    fx.file("visible.txt", 55);
    let setup_ok = fx.deny_read("secret");
    if !setup_ok {
        // Loud skip with a stated reason (rule: never skip silently).
        eprintln!("SKIPPED denied_entries_counted_not_fatal: icacls /deny setup failed on this machine; verify manually");
        return;
    }

    let (tree, stats) = scan(&fx).expect("denied subdir must not abort the scan");
    assert!(stats.denied >= 1, "denied counter must increment");
    let root = tree.root().unwrap();
    // visible.txt must still be counted; secret contributes nothing known.
    assert_eq!(tree.node(root).agg_logical(), 55);
    if let Some(secret) = find_by_name(&tree, "secret") {
        assert_eq!(tree.node(secret).child_count(), 0, "denied dir must have no children");
    }
}

#[test]
fn sizes_and_flags_on_plain_files() {
    let fx = Fixture::empty("plain");
    fx.file("a.bin", 4096); // exactly one cluster on most volumes
    fx.file("b.bin", 1); // rounds up to a full cluster

    let (tree, _stats) = scan(&fx).expect("scan");
    let root = tree.root().unwrap();
    assert_eq!(tree.node(root).agg_logical(), 4097, "logical sizes exact");
    // Size on disk = logical rounded up to cluster size; 4096 + 4096 = 8192
    // on an NTFS volume with 4096-byte clusters (test box assumption).
    let disk = tree.node(root).agg_on_disk();
    assert!(disk >= 8192, "size on disk must round up to cluster granularity (got {disk})");
    let b = find_by_name(&tree, "b.bin").unwrap();
    assert_eq!(tree.node(b).flags() & flags::PLACEHOLDER, 0, "local files are not placeholders");
}
