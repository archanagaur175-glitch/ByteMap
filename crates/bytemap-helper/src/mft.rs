//! Real MFT streaming (P6.T3a): the elevated half of the scan.
//!
//! Two request shapes (P6.T4d):
//!
//! - **Whole volume** (`C:\`): ONE pass over `$MFT`, the fast path. The
//!   root record is synthesized first, then every live entry is streamed
//!   as it is read — no index, no second pass. This is the shape the
//!   speed gate measures (~527k entries/s on the P1.T7 spike machine)
//!   and the only shape the product uses: the app routes folder scans to
//!   the walker, which is faster on a subtree anyway.
//! - **Subtree** (`C:\Users\...`, parity harness): TWO passes, because
//!   `$MFT` offers no subtree shortcut and the root first has to be
//!   resolved by name:
//!
//! 1. **Directory index**: stream every live directory and remember
//!    `(own number, best name, parent number, reparse)` grouped by parent.
//!    That is the whole state kept — the app never receives anything but
//!    its requested subtree, and the helper never holds file data.
//! 2. **Record stream**: resolve the requested root by a case-insensitive
//!    walk from the volume root (record [`ROOT_RECORD`]), breadth-first
//!    collect the subtree's directory numbers (junctions/symlinks stop the
//!    descent, like the walker), then stream a [`Record`] frame for every
//!    live entry whose parent is inside the subtree, followed by `Done`.
//!
//! Hard links: one `Record` per name inside the subtree (the walker sees
//! each link too); DOS 8.3 aliases are skipped. `allocated_size` is always
//! 0 on the wire — the app computes size-on-disk from the root's cluster
//! size, matching the walker's default rule.
//!
//! Everything that can fail ends as an on-the-wire `Error` frame: the
//! client must see why a scan failed, and the failure never takes the
//! server down. Elevation is required to read a volume; an unelevated
//! caller gets the OS error from `Volume::new` in that message.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};

use ntfs_reader::{MftScan, NtfsFile, Volume};

use crate::proto::{self, Frame, Record};

/// The scan root as the helper understands it.
pub(crate) struct ScanTarget {
    /// Win32 device path, e.g. `\\.\C:`.
    device: String,
    /// Components below the drive root, e.g. `["Users", "archa"]`.
    components: Vec<String>,
}

impl ScanTarget {
    /// A drive root (`C:\`): the single-pass whole-volume path applies.
    fn is_whole_volume(&self) -> bool {
        self.components.is_empty()
    }
}

/// Parse a user-facing scan root into a volume device + components.
/// Only local drive paths (`X:\...`, either separator) are accepted:
/// UNC paths and relative paths are rejected — the MFT tier is local
/// NTFS only. `..` components are rejected (no escaping the drive).
pub(crate) fn volume_device(path: &str) -> Option<ScanTarget> {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    if bytes[2] != b'\\' && bytes[2] != b'/' {
        return None;
    }
    let device = format!(r"\\.\{}:", (bytes[0] as char).to_ascii_uppercase());
    let mut components = Vec::new();
    for component in path[3..].split(['\\', '/']) {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return None;
        }
        components.push(component.to_owned());
    }
    Some(ScanTarget { device, components })
}

/// One live directory, indexed by its parent's record number.
struct DirChild {
    /// The directory's own record number (masked, no sequence).
    number: u64,
    /// Best (Win32-preferred) name, lossy UTF-8.
    name: String,
    /// A junction/symlink/reparse directory: recorded, never descended.
    reparse: bool,
}

/// The directory index from pass 1: live directories grouped by parent.
struct DirIndex {
    by_parent: HashMap<u64, Vec<DirChild>>,
}

/// Pass 1: stream the whole volume once and index every live directory.
fn collect_dirs(scan: &mut MftScan) -> Result<DirIndex, String> {
    let mut by_parent: HashMap<u64, Vec<DirChild>> = HashMap::new();
    while let Some(chunk) = scan.next_chunk().map_err(|e| e.to_string())? {
        for file in chunk.files() {
            if !file.is_directory() {
                continue;
            }
            let Some(name) = file.best_name() else {
                continue; // unnamed (corrupt): unresolvable, skip with its subtree
            };
            let reparse = name.is_reparse_point();
            by_parent
                .entry(name.parent_number())
                .or_default()
                .push(DirChild {
                    number: file.number(),
                    name: name.to_os_string().to_string_lossy().into_owned(),
                    reparse,
                });
        }
    }
    Ok(DirIndex { by_parent })
}

/// Resolve the requested root: case-insensitive walk of `components` from
/// the volume root (record 5), through non-reparse directories only (a
/// junctioned root path is refused and the caller falls back to the
/// walker). Empty components select the drive root itself (empty name).
fn resolve_root(index: &DirIndex, components: &[String]) -> Option<(u64, String)> {
    let mut current = ntfs_reader::ROOT_RECORD;
    let mut name = String::new();
    for component in components {
        let want = component.to_lowercase();
        let children = index.by_parent.get(&current)?;
        let next = children
            .iter()
            .find(|child| !child.reparse && child.name.to_lowercase() == want)?;
        current = next.number;
        name = next.name.clone();
    }
    Some((current, name))
}

/// Breadth-first collect the subtree's directory numbers. Reparse
/// directories stop the descent (the walker records them but never
/// follows); files are keyed by parent and need no entry here.
fn collect_subtree(index: &DirIndex, root: u64) -> HashSet<u64> {
    let mut subtree = HashSet::new();
    subtree.insert(root);
    let mut queue = VecDeque::new();
    queue.push_back(root);
    while let Some(current) = queue.pop_front() {
        if let Some(children) = index.by_parent.get(&current) {
            for child in children {
                if !child.reparse && subtree.insert(child.number) {
                    queue.push_back(child.number);
                }
            }
        }
    }
    subtree
}

/// Entry point of a scan session: every failure becomes an `Error` frame,
/// which ends the session normally on the wire.
pub(super) fn handle_scan_request<S: Read + Write>(stream: &mut S, path: &str) {
    if let Err(message) = run_scan(stream, path) {
        // Best effort: if the client hung up mid-scan, this write fails
        // too and the session ends either way.
        let _ = proto::write_frame(stream, &Frame::Error { message });
    }
}

fn run_scan<S: Read + Write>(stream: &mut S, path: &str) -> Result<(), String> {
    let target = volume_device(path).ok_or_else(|| format!("not a local drive path: {path}"))?;
    if target.is_whole_volume() {
        return run_volume_scan(stream, &target, path);
    }
    run_subtree_scan(stream, &target, path)
}

/// The fast path (P6.T4d): one `$MFT` pass, every live record streamed.
///
/// No directory index is needed because the root is the volume root
/// itself, and no membership set is needed because every entry is inside
/// the request. The root record frame comes first for the same contract
/// as the subtree path: the app learns the root's record number and
/// attaches children by masked parent number.
fn run_volume_scan<S: Read + Write>(
    stream: &mut S,
    target: &ScanTarget,
    path: &str,
) -> Result<(), String> {
    let volume =
        Volume::new(&target.device).map_err(|e| format!("cannot open {}: {e}", target.device))?;
    let mut scan =
        MftScan::new(volume).map_err(|e| format!("cannot read {}: {e}", target.device))?;

    // Capture the journal cursor before streaming: everything after it is a
    // change the app can later apply to this snapshot. Best effort — a
    // volume without an active journal still scans, just not refreshably.
    if let Some((journal_id, next_usn)) = crate::usn::checkpoint_for(path) {
        proto::write_frame(
            stream,
            &Frame::UsnCheckpoint {
                journal_id,
                next_usn,
            },
        )
        .map_err(|e| e.to_string())?;
    }

    proto::write_frame(
        stream,
        &Frame::Record(Record {
            file_ref: ntfs_reader::ROOT_RECORD,
            parent_ref: ntfs_reader::ROOT_RECORD,
            logical_size: 0,
            allocated_size: 0,
            attributes: FILE_ATTRIBUTE_DIRECTORY,
            mtime: 0,
            // Drive root: the app mirrors the walker's naming rule.
            name: String::new(),
        }),
    )
    .map_err(|e| e.to_string())?;

    let mut records = 0u64;
    let mut logical_bytes = 0u64;
    while let Some(chunk) = scan.next_chunk().map_err(|e| e.to_string())? {
        for file in chunk.files() {
            let fields = entry_fields(&file);
            let file_ref = file.reference();
            for name in file.names() {
                if name.is_dos_alias() {
                    continue; // 8.3 alias of a sibling Win32 name, not a link
                }
                let record = Record {
                    file_ref,
                    parent_ref: name.parent_reference(),
                    logical_size: fields.logical,
                    allocated_size: 0, // app computes from the cluster size
                    attributes: fields.attrs,
                    mtime: fields.mtime,
                    name: name.to_os_string().to_string_lossy().into_owned(),
                };
                logical_bytes += fields.logical;
                records += 1;
                proto::write_frame(stream, &Frame::Record(record)).map_err(|e| e.to_string())?;
            }
        }
    }

    proto::write_frame(
        stream,
        &Frame::Done {
            entries: records,
            logical_bytes,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The parity path (P6.T3): resolve a folder root by name, then stream its
/// subtree across two passes. Used by the parity benchmark; the product
/// sends folder scans to the walker, which needs no elevation and is
/// faster on a subtree (a subtree costs whole-volume passes here).
fn run_subtree_scan<S: Read + Write>(
    stream: &mut S,
    target: &ScanTarget,
    path: &str,
) -> Result<(), String> {
    // Pass 1: the directory index.
    let volume =
        Volume::new(&target.device).map_err(|e| format!("cannot open {}: {e}", target.device))?;
    let index = collect_dirs(
        &mut MftScan::new(volume).map_err(|e| format!("cannot read {}: {e}", target.device))?,
    )?;

    let (root_number, root_name) = resolve_root(&index, &target.components)
        .ok_or_else(|| format!("root not found on {}: {path}", target.device))?;
    let subtree = collect_subtree(&index, root_number);

    // The requested root goes first so the app learns its record number
    // (children attach to it by masked parent number). Its mtime/attrs
    // are the app's business: it stats the path itself, unelevated.
    proto::write_frame(
        stream,
        &Frame::Record(Record {
            file_ref: root_number,
            parent_ref: root_number,
            logical_size: 0,
            allocated_size: 0,
            attributes: FILE_ATTRIBUTE_DIRECTORY,
            mtime: 0,
            name: root_name,
        }),
    )
    .map_err(|e| e.to_string())?;

    // Pass 2: stream the subtree's records.
    let volume =
        Volume::new(&target.device).map_err(|e| format!("cannot reopen {}: {e}", target.device))?;
    let tally = stream_records(
        &mut MftScan::new(volume).map_err(|e| format!("cannot reread {}: {e}", target.device))?,
        &subtree,
        stream,
    )?;

    proto::write_frame(
        stream,
        &Frame::Done {
            entries: tally.0,
            logical_bytes: tally.1,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// (records sent, logical bytes of files sent)
type Tally = (u64, u64);

/// Per-record facts shared by both streaming paths: the walker-equivalent
/// logical size, the Win32 attributes, and the FILETIME mtime.
struct EntryFields {
    logical: u64,
    attrs: u32,
    mtime: i64,
}

fn entry_fields(file: &NtfsFile) -> EntryFields {
    let is_dir = file.is_directory();
    // Size of the default (unnamed) data stream: the walker's logical
    // size for a file. Split across extension records, the later extents
    // carry 0, so summing is exact.
    let logical = if is_dir {
        0
    } else {
        file.data_streams()
            .filter(|s| s.name.is_none())
            .map(|s| s.size)
            .sum()
    };
    // Attributes and mtime once per record. The directory bit is forced
    // on for dirs: the app decides "is a directory" from it, and
    // $STANDARD_INFORMATION does not guarantee it is set.
    let (attrs, mtime) = match file.standard_information() {
        Some(si) => {
            let mut attrs = si.file_attributes();
            if is_dir {
                attrs |= FILE_ATTRIBUTE_DIRECTORY;
            }
            // FILETIME ticks (100 ns since 1601) from the modification
            // time — the same encoding and pre-1970 rule (collapse to 0)
            // as the walker's `metadata::filetime`. No new dependency: the
            // type is ntfs-reader's, converted inline.
            let mtime = {
                const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
                let nanos = si.modified().unix_timestamp_nanos();
                if nanos < 0 {
                    0
                } else {
                    i64::try_from(nanos / 100)
                        .unwrap_or(i64::MAX)
                        .saturating_add(EPOCH_DIFF)
                }
            };
            (attrs, mtime)
        }
        None => (if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { 0 }, 0),
    };
    EntryFields {
        logical,
        attrs,
        mtime,
    }
}

/// Pass 2: stream one `Record` per name inside the subtree, then hand the
/// caller back the tally for the `Done` frame.
fn stream_records<S: Read + Write>(
    scan: &mut MftScan,
    subtree: &HashSet<u64>,
    stream: &mut S,
) -> Result<Tally, String> {
    let mut records = 0u64;
    let mut logical_bytes = 0u64;
    while let Some(chunk) = scan.next_chunk().map_err(|e| e.to_string())? {
        for file in chunk.files() {
            let fields = entry_fields(&file);
            let file_ref = file.reference();
            for name in file.names() {
                if name.is_dos_alias() {
                    continue; // 8.3 alias of a sibling Win32 name, not a link
                }
                let parent = name.parent_number();
                if !subtree.contains(&parent) {
                    continue;
                }
                let record = Record {
                    file_ref,
                    parent_ref: name.parent_reference(),
                    logical_size: fields.logical,
                    allocated_size: 0, // app computes from the cluster size
                    attributes: fields.attrs,
                    mtime: fields.mtime,
                    name: name.to_os_string().to_string_lossy().into_owned(),
                };
                logical_bytes += fields.logical;
                records += 1;
                proto::write_frame(stream, &Frame::Record(record)).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok((records, logical_bytes))
}

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::proto::OP_ERROR;

    #[test]
    fn volume_device_accepts_drive_paths() {
        let target = volume_device(r"C:\Users\archa").unwrap();
        assert_eq!(target.device, r"\\.\C:");
        assert_eq!(target.components, ["Users", "archa"]);

        // Either separator, case-insensitive drive letter, trailing or
        // doubled separators, "." noise.
        let target = volume_device("d:/temp/./x/").unwrap();
        assert_eq!(target.device, r"\\.\D:");
        assert_eq!(target.components, ["temp", "x"]);

        // Drive root: empty components.
        let target = volume_device(r"C:\").unwrap();
        assert_eq!(target.components, Vec::<String>::new());
    }

    #[test]
    fn volume_device_rejects_everything_else() {
        assert!(volume_device(r"\\server\share\dir").is_none(), "UNC");
        assert!(volume_device(r"relative\path").is_none());
        assert!(volume_device("C:relative").is_none(), "no separator");
        assert!(volume_device(r"C:\a\..\b").is_none(), "no escaping");
        assert!(volume_device("").is_none());
        assert!(volume_device(r"1:\x").is_none());
    }

    /// Synthetic index: root(5) -> Users(100) -> archa(200), with a
    /// junction "link"(300) under archa leading to secret(400), and a
    /// normal dir projects(500) under archa.
    fn fixture_index() -> DirIndex {
        let mut by_parent = HashMap::new();
        let push = |by_parent: &mut HashMap<u64, Vec<DirChild>>,
                    parent: u64,
                    number: u64,
                    name: &str,
                    reparse: bool| {
            by_parent.entry(parent).or_default().push(DirChild {
                number,
                name: name.to_owned(),
                reparse,
            });
        };
        push(&mut by_parent, 5, 100, "Users", false);
        push(&mut by_parent, 100, 200, "archa", false);
        push(&mut by_parent, 200, 300, "link", true); // junction
        push(&mut by_parent, 300, 400, "secret", false);
        push(&mut by_parent, 200, 500, "projects", false);
        DirIndex { by_parent }
    }

    #[test]
    fn resolve_root_walks_case_insensitively() {
        let index = fixture_index();
        let (number, name) = resolve_root(&index, &["users".to_owned(), "ARCHA".to_owned()])
            .expect("case-insensitive resolution");
        assert_eq!(number, 200);
        assert_eq!(name, "archa");

        // Drive root: record 5, empty name.
        let (number, name) = resolve_root(&index, &[]).unwrap();
        assert_eq!(number, 5);
        assert_eq!(name, "");

        assert!(resolve_root(&index, &["Users", "nobody"].map(String::from)).is_none());
        assert!(
            resolve_root(&index, &["Users", "archa", "link"].map(String::from)).is_none(),
            "a junctioned root path is refused"
        );
    }

    #[test]
    fn subtree_stops_at_reparse_directories() {
        let index = fixture_index();
        let (root, _) = resolve_root(&index, &["Users", "archa"].map(String::from)).unwrap();
        let subtree = collect_subtree(&index, root);
        assert!(subtree.contains(&200), "root itself");
        assert!(subtree.contains(&500), "ordinary child dir");
        assert!(!subtree.contains(&400), "beyond the junction");
        // The junction dir itself is not a descent root, but its record
        // still attaches to a subtree parent (emitted by the parent rule).
        assert!(index
            .by_parent
            .get(&200)
            .unwrap()
            .iter()
            .any(|c| c.number == 300 && c.reparse));
    }

    #[test]
    fn drive_roots_take_the_single_pass_path() {
        assert!(volume_device(r"C:\").unwrap().is_whole_volume());
        assert!(volume_device("D:/").unwrap().is_whole_volume());
        assert!(!volume_device(r"C:\Users").unwrap().is_whole_volume());
    }

    #[test]
    fn session_replies_with_an_error_frame_for_a_bad_volume() {
        // Q: is assumed to be a nonexistent drive on this machine (any
        // letter the owner's box does not use works; update if needed).
        let mut stream = scan_session(r"Q:\fixture");
        super::super::pipe::run_session(&mut stream).expect("session ends normally");
        assert_error_frame(&stream.outgoing, "cannot open");
    }

    /// The whole-volume fast path fails the same way on the wire.
    #[test]
    fn volume_scan_replies_with_an_error_frame_for_a_bad_volume() {
        let mut stream = scan_session(r"Q:\");
        super::super::pipe::run_session(&mut stream).expect("session ends normally");
        assert_error_frame(&stream.outgoing, "cannot open");
    }

    /// Handshake + one scan request for `path`, ready for `run_session`.
    fn scan_session(path: &str) -> DuplexStream {
        let mut incoming = Vec::new();
        incoming.extend_from_slice(&crate::PROTOCOL_MAGIC);
        incoming.extend_from_slice(&crate::PROTOCOL_VERSION.to_le_bytes());
        let mut request = Vec::new();
        request.push(crate::proto::OP_SCAN_REQUEST);
        request.extend_from_slice(&(path.len() as u16).to_le_bytes());
        request.extend_from_slice(path.as_bytes());
        let wire_len = (request.len() as u32).to_le_bytes();
        incoming.extend_from_slice(&wire_len);
        incoming.extend_from_slice(&request);
        DuplexStream {
            incoming: Cursor::new(incoming),
            outgoing: Vec::new(),
        }
    }

    /// Outgoing must be: handshake reply (7 bytes) + one terminal error
    /// frame whose message contains `needle`.
    fn assert_error_frame(outgoing: &[u8], needle: &str) {
        assert_eq!(&outgoing[0..4], &crate::PROTOCOL_MAGIC);
        assert_eq!(outgoing[6], 0, "handshake accepted");
        let frame_len = u32::from_le_bytes(outgoing[7..11].try_into().unwrap()) as usize;
        let payload = &outgoing[11..11 + frame_len];
        assert_eq!(payload[0], OP_ERROR, "terminal error frame");
        let message = String::from_utf8_lossy(
            &payload[3..3 + u16::from_le_bytes([payload[1], payload[2]]) as usize],
        );
        assert!(
            message.contains(needle),
            "error must contain {needle:?}: {message}"
        );
    }

    /// One-shot duplex buffer for session tests: reads drain `incoming`,
    /// writes append to `outgoing`.
    struct DuplexStream {
        incoming: Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl Read for DuplexStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.incoming.read(buf)
        }
    }

    impl Write for DuplexStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.outgoing.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
