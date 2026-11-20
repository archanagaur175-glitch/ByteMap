//! Snapshot file format (Phase 8): one completed scan persisted as a single
//! versioned file, so a big scan can be reopened without walking the disk.
//!
//! Codec decision Q5 (owner, 2026-11-19): hand-rolled packed little-endian,
//! no serializer dependency. `bincode` is not used (3.0.0 is a tombstone
//! release, 2.0.1 is abandoned). The body is a zstd frame with a content
//! checksum, so a flipped byte anywhere in the body fails the load.
//!
//! Format (all integers little-endian):
//!
//! ```text
//! header (uncompressed):
//!   magic "BYTEMAPS" (8 bytes)
//!   u16 format_version = 1     structure of this file
//!   u16 codec_version  = 1     encoding of the body (zstd frame)
//!   u64 created_unix
//!   u8  scan_tier (0 = walker, 1 = mft), u8[3] reserved (0)
//!   u64 node_count, u64 file_count, u64 dir_count, u64 denied_count
//!   u64 total_logical, u64 total_on_disk, u64 scan_ms
//!   u32 + bytes  root_name    (display name of the scan root)
//!   u32 + bytes  app_version  (writer, for the snapshot list)
//!   u64 body_uncompressed_len
//!   u64 body_compressed_len
//! body (zstd frame, checksum enabled):
//!   u32 node_count (repeat of the header; mismatch = corrupt)
//!   u32 strings_len + string arena bytes
//!   u32 cat_len + category bytes   (cat_len == node_count)
//!   u32 tier_len + tier bytes      (tier_len == node_count)
//!   per node, 38 bytes:
//!     u32 parent (u32::MAX for the root)
//!     u32 name_off, u64 agg_logical, u64 agg_on_disk, i64 mtime,
//!     u16 name_len, u16 depth, u16 flags
//! ```
//!
//! The child links (`first_child` / `next_sibling` / `child_count`) are NOT
//! stored: they are rebuilt from the parent column in arena order, which is
//! insertion order, so the restored tree iterates children identically.
//! Header counts are computed from the tree at save time, never taken from
//! the caller, and the loader re-checks every structural invariant (root at
//! index 0, parent precedes child, exact depth, in-bounds UTF-8 names,
//! known category/tier ids), so a hand-edited file cannot build a broken
//! tree.

use std::fmt;
use std::io::{Read, Write};

use crate::categorizer::{category, tier, CategoryMap};
use crate::tree::{flags, Node, NodeId, Tree, NONE};

/// File magic. Eight bytes so a hex dump identifies the file immediately.
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"BYTEMAPS";

/// v1: the Phase 8 snapshot (format structure, not the app version).
pub const SNAPSHOT_FORMAT_VERSION: u16 = 1;

/// v1: body = zstd frame with content checksum.
pub const SNAPSHOT_CODEC_VERSION: u16 = 1;

/// Fixed header bytes up to and including `scan_ms`.
const HEADER_FIXED_LEN: usize = 80;

/// Bytes per node record in the body.
const NODE_RECORD_LEN: usize = 38;

/// A Windows name is at most 255 UTF-16 units, and one unit is at most
/// 3 UTF-8 bytes, so this bounds every legal name.
const MAX_NAME_BYTES: usize = 765;

/// A scan root (including long paths) is well below this.
const MAX_ROOT_NAME_LEN: usize = 32 * 1024;

/// App version strings are `CARGO_PKG_VERSION`, never near this.
const MAX_APP_VERSION_LEN: usize = 128;

/// Largest possible header, so a caller can read one bounded prefix from a
/// file and call [`read_header`] without loading the body.
pub const MAX_HEADER_LEN: usize =
    HEADER_FIXED_LEN + 4 + MAX_ROOT_NAME_LEN + 4 + MAX_APP_VERSION_LEN + 16;

/// Which scanner produced the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTier {
    Walker,
    Mft,
}

impl ScanTier {
    /// Wire id used in the header.
    pub const fn id(self) -> u8 {
        match self {
            ScanTier::Walker => 0,
            ScanTier::Mft => 1,
        }
    }

    pub fn from_id(id: u8) -> Result<Self, SnapshotError> {
        match id {
            0 => Ok(ScanTier::Walker),
            1 => Ok(ScanTier::Mft),
            other => Err(SnapshotError::BadTier(other)),
        }
    }

    /// The same labels the scan `complete` event uses.
    pub const fn as_str(self) -> &'static str {
        match self {
            ScanTier::Walker => "walker",
            ScanTier::Mft => "mft",
        }
    }
}

/// Descriptive metadata stored with every snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// Seconds since the Unix epoch, captured by the caller at save time.
    pub created_unix: u64,
    /// How long the original scan took (0 = unknown).
    pub scan_ms: u64,
    pub tier: ScanTier,
    /// Display name of the scan root, e.g. `C:` or the folder path.
    pub root_name: String,
    /// App version that wrote the file.
    pub app_version: String,
}

/// Totals shown in the snapshot list without loading the tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotCounts {
    pub nodes: u64,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub total_logical: u64,
    pub total_on_disk: u64,
}

/// Everything the snapshot list needs, without decompressing the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub meta: SnapshotMeta,
    pub counts: SnapshotCounts,
}

/// A complete saved scan: the tree plus the categorization made at scan time.
#[derive(Debug)]
pub struct Snapshot {
    pub meta: SnapshotMeta,
    pub tree: Tree,
    pub cats: CategoryMap,
}

/// Everything that can go wrong saving or loading a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// A tree with no root is not a scan; nothing is written.
    EmptyTree,
    /// Category vectors must have exactly one entry per node.
    CategoryLength {
        nodes: usize,
        cats: usize,
        tiers: usize,
    },
    BadMagic,
    UnsupportedFormat(u16),
    UnsupportedCodec(u16),
    BadTier(u8),
    /// Header-level corruption (before the body is touched).
    BadHeader(&'static str),
    /// Body structure violates a tree invariant.
    BadTree(&'static str),
    /// The category map holds an id the wire format does not define.
    BadCategoryMap,
    /// The declared bytes are not all there.
    Truncated,
    /// Length fields disagree with the actual payload.
    SizeMismatch,
    /// More nodes than the arena can index.
    NodeLimit(u64),
    /// zstd or I/O failure, with the underlying message.
    Codec(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::EmptyTree => f.write_str("cannot save a snapshot without a tree"),
            SnapshotError::CategoryLength { nodes, cats, tiers } => write!(
                f,
                "category map has {cats} categories and {tiers} tiers for {nodes} nodes"
            ),
            SnapshotError::BadMagic => f.write_str("not a ByteMap snapshot (bad magic)"),
            SnapshotError::UnsupportedFormat(v) => {
                write!(f, "unsupported snapshot format version {v}")
            }
            SnapshotError::UnsupportedCodec(v) => {
                write!(f, "unsupported snapshot body codec {v}")
            }
            SnapshotError::BadTier(v) => write!(f, "unknown scan tier id {v}"),
            SnapshotError::BadHeader(why) => write!(f, "bad snapshot header: {why}"),
            SnapshotError::BadTree(why) => write!(f, "bad snapshot tree: {why}"),
            SnapshotError::BadCategoryMap => {
                f.write_str("category map contains an id the format does not define")
            }
            SnapshotError::Truncated => f.write_str("snapshot file is truncated"),
            SnapshotError::SizeMismatch => {
                f.write_str("snapshot body length does not match its header")
            }
            SnapshotError::NodeLimit(n) => {
                write!(f, "snapshot claims {n} nodes, more than u32::MAX")
            }
            SnapshotError::Codec(msg) => write!(f, "snapshot codec: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Counts derived from the tree itself (files vs dirs, denied, totals).
pub fn tree_counts(tree: &Tree) -> SnapshotCounts {
    let mut counts = SnapshotCounts::default();
    for node in tree.node_slice() {
        counts.nodes += 1;
        if node.flags & flags::DIR != 0 {
            counts.dirs += 1;
        } else {
            counts.files += 1;
        }
        if node.flags & flags::DENIED != 0 {
            counts.denied += 1;
        }
    }
    if let Some(root) = tree.root() {
        counts.total_logical = tree.node(root).agg_logical();
        counts.total_on_disk = tree.node(root).agg_on_disk();
    }
    counts
}

/// Serialize a snapshot to the bytes of one `.bms` file.
pub fn save_snapshot(snapshot: &Snapshot) -> Result<Vec<u8>, SnapshotError> {
    save_snapshot_parts(&snapshot.tree, &snapshot.cats, &snapshot.meta)
}

/// Serialize borrowed parts. The app uses this to save the session tree
/// without cloning it; the format is identical to [`save_snapshot`].
pub fn save_snapshot_parts(
    tree: &Tree,
    cats: &CategoryMap,
    meta: &SnapshotMeta,
) -> Result<Vec<u8>, SnapshotError> {
    let n = tree.len();
    if n == 0 || tree.root().is_none() {
        return Err(SnapshotError::EmptyTree);
    }
    if n > u32::MAX as usize {
        return Err(SnapshotError::NodeLimit(n as u64));
    }
    if cats.cat.len() != n || cats.tier.len() != n {
        return Err(SnapshotError::CategoryLength {
            nodes: n,
            cats: cats.cat.len(),
            tiers: cats.tier.len(),
        });
    }

    let counts = tree_counts(tree);
    let strings = tree.string_bytes();
    if strings.len() > u32::MAX as usize {
        return Err(SnapshotError::BadHeader("string arena is too large"));
    }
    // Save must never write a file the loader would reject: category ids are
    // fixed by the wire format (0..=6 categories, 0..=2 tiers).
    if cats.cat.iter().any(|&c| c > category::OTHER)
        || cats.tier.iter().any(|&t| t > tier::PROTECTED)
    {
        return Err(SnapshotError::BadCategoryMap);
    }
    let uncompressed_len = body_len(n, strings.len());

    // Body: write straight into the encoder, so the uncompressed bytes never
    // exist as a second buffer.
    let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3)
        .map_err(|e| SnapshotError::Codec(e.to_string()))?;
    enc.include_checksum(true)
        .map_err(|e| SnapshotError::Codec(e.to_string()))?;
    write_body(&mut enc, tree, cats, n, strings)?;
    let body = enc
        .finish()
        .map_err(|e| SnapshotError::Codec(e.to_string()))?;

    // Header + body.
    let mut out = Vec::with_capacity(HEADER_FIXED_LEN + 128 + body.len());
    out.extend_from_slice(&SNAPSHOT_MAGIC);
    put_u16(&mut out, SNAPSHOT_FORMAT_VERSION);
    put_u16(&mut out, SNAPSHOT_CODEC_VERSION);
    put_u64(&mut out, meta.created_unix);
    out.push(meta.tier.id());
    out.extend_from_slice(&[0, 0, 0]); // reserved
    put_u64(&mut out, counts.nodes);
    put_u64(&mut out, counts.files);
    put_u64(&mut out, counts.dirs);
    put_u64(&mut out, counts.denied);
    put_u64(&mut out, counts.total_logical);
    put_u64(&mut out, counts.total_on_disk);
    put_u64(&mut out, meta.scan_ms);
    put_string32(&mut out, &meta.root_name)?;
    put_string32(&mut out, &meta.app_version)?;
    put_u64(&mut out, uncompressed_len as u64);
    put_u64(&mut out, body.len() as u64);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Read the header of a snapshot without decompressing (cheap list entry).
/// Works on a prefix of the file: only the header bytes are required.
pub fn read_header(bytes: &[u8]) -> Result<SnapshotHeader, SnapshotError> {
    Ok(parse_header(bytes)?.header)
}

/// Load a whole snapshot from the bytes of one `.bms` file.
pub fn load_snapshot(bytes: &[u8]) -> Result<Snapshot, SnapshotError> {
    let parsed = parse_header(bytes)?;
    let body_start = parsed.body_start;
    let uncompressed_len = parsed.uncompressed_len;
    let compressed_len = parsed.compressed_len;

    let compressed = usize::try_from(compressed_len).map_err(|_| SnapshotError::Truncated)?;
    let end = body_start
        .checked_add(compressed)
        .ok_or(SnapshotError::Truncated)?;
    if bytes.len() < end {
        return Err(SnapshotError::Truncated);
    }
    if bytes.len() > end {
        return Err(SnapshotError::SizeMismatch);
    }

    let body = &bytes[body_start..end];
    let mut dec =
        zstd::stream::read::Decoder::new(body).map_err(|e| SnapshotError::Codec(e.to_string()))?;
    let (tree, cats, read) = decode_body(&mut dec, parsed.header.counts.nodes)?;

    // Reading past the last body byte drains the frame, which is when zstd
    // verifies the content checksum. A clean frame returns 0 bytes.
    let mut extra = [0u8; 1];
    if dec
        .read(&mut extra)
        .map_err(|e| SnapshotError::Codec(e.to_string()))?
        != 0
    {
        return Err(SnapshotError::BadTree("trailing bytes after the tree"));
    }
    if read != uncompressed_len {
        return Err(SnapshotError::SizeMismatch);
    }

    Ok(Snapshot {
        meta: parsed.header.meta,
        tree,
        cats,
    })
}

/// Uncompressed body size for a tree of `n` nodes and `strings_len` bytes.
fn body_len(n: usize, strings_len: usize) -> usize {
    4 + 4 + strings_len + 4 + n + 4 + n + n * NODE_RECORD_LEN
}

fn write_body<W: Write>(
    w: &mut W,
    tree: &Tree,
    cats: &CategoryMap,
    n: usize,
    strings: &[u8],
) -> Result<(), SnapshotError> {
    let write = |w: &mut W, bytes: &[u8]| -> Result<(), SnapshotError> {
        w.write_all(bytes)
            .map_err(|e| SnapshotError::Codec(e.to_string()))
    };

    let mut head = Vec::with_capacity(4);
    put_u32(&mut head, n as u32);
    write(w, &head)?;

    let mut head = Vec::with_capacity(4);
    put_u32(&mut head, strings.len() as u32);
    write(w, &head)?;
    write(w, strings)?;

    let mut head = Vec::with_capacity(4);
    put_u32(&mut head, n as u32);
    write(w, &head)?;
    write(w, &cats.cat)?;

    let mut head = Vec::with_capacity(4);
    put_u32(&mut head, n as u32);
    write(w, &head)?;
    write(w, &cats.tier)?;

    // One fixed 38-byte record per node, in arena order.
    let mut rec = [0u8; NODE_RECORD_LEN];
    for node in tree.node_slice() {
        rec[0..4].copy_from_slice(&node.parent.to_le_bytes());
        rec[4..8].copy_from_slice(&node.name_off.to_le_bytes());
        rec[8..16].copy_from_slice(&node.agg_logical.to_le_bytes());
        rec[16..24].copy_from_slice(&node.agg_on_disk.to_le_bytes());
        rec[24..32].copy_from_slice(&node.mtime.to_le_bytes());
        rec[32..34].copy_from_slice(&node.name_len.to_le_bytes());
        rec[34..36].copy_from_slice(&node.depth.to_le_bytes());
        rec[36..38].copy_from_slice(&node.flags.to_le_bytes());
        write(w, &rec)?;
    }
    Ok(())
}

/// Decode the body and rebuild the tree. Returns the bytes consumed.
fn decode_body<R: Read>(
    reader: &mut R,
    header_nodes: u64,
) -> Result<(Tree, CategoryMap, u64), SnapshotError> {
    if header_nodes == 0 {
        return Err(SnapshotError::BadHeader("node count is zero"));
    }
    let mut rd = BodyReader::new(reader);

    let n = rd.u32()? as usize;
    if n == 0 || n as u64 != header_nodes {
        return Err(SnapshotError::BadTree(
            "node count does not match the header",
        ));
    }

    let strings_len = rd.u32()? as usize;
    let cap = (n as u64)
        .checked_mul(MAX_NAME_BYTES as u64)
        .and_then(|b| b.checked_add(8))
        .ok_or(SnapshotError::BadTree("string arena is too large"))?;
    if strings_len as u64 > cap {
        return Err(SnapshotError::BadTree("string arena is too large"));
    }
    let strings = rd.bytes(strings_len)?;

    let cat_len = rd.u32()? as usize;
    if cat_len != n {
        return Err(SnapshotError::BadTree("category length does not match"));
    }
    let cats = rd.bytes(n)?;
    for &c in &cats {
        if c > category::OTHER {
            return Err(SnapshotError::BadTree("unknown category id"));
        }
    }
    let tier_len = rd.u32()? as usize;
    if tier_len != n {
        return Err(SnapshotError::BadTree("tier length does not match"));
    }
    let tiers = rd.bytes(n)?;
    for &t in &tiers {
        if t > tier::PROTECTED {
            return Err(SnapshotError::BadTree("unknown tier id"));
        }
    }

    let mut nodes: Vec<Node> = Vec::with_capacity(n.min(1 << 20));
    for id in 0..n {
        let parent = rd.u32()?;
        let name_off = rd.u32()?;
        let agg_logical = rd.u64()?;
        let agg_on_disk = rd.u64()?;
        let mtime = rd.u64()? as i64;
        let name_len = rd.u16()?;
        let depth = rd.u16()?;
        let node_flags = rd.u16()?;

        // Structural invariants: one root at index 0, parents first, depth
        // exactly parent + 1, name inside the arena and valid UTF-8.
        if id == 0 {
            if parent != NONE {
                return Err(SnapshotError::BadTree("root has a parent"));
            }
            if depth != 0 {
                return Err(SnapshotError::BadTree("root depth is not zero"));
            }
        } else if parent == NONE || parent as usize >= id {
            return Err(SnapshotError::BadTree("parent does not precede the child"));
        } else {
            let want = nodes[parent as usize]
                .depth
                .checked_add(1)
                .ok_or(SnapshotError::BadTree("depth does not match the parent"))?;
            if depth != want {
                return Err(SnapshotError::BadTree("depth does not match the parent"));
            }
        }

        if name_len as usize > MAX_NAME_BYTES {
            return Err(SnapshotError::BadTree("name is too long"));
        }
        let start = name_off as usize;
        let end = start
            .checked_add(name_len as usize)
            .ok_or(SnapshotError::BadTree("name is outside the string arena"))?;
        if end > strings.len() {
            return Err(SnapshotError::BadTree("name is outside the string arena"));
        }
        if std::str::from_utf8(&strings[start..end]).is_err() {
            return Err(SnapshotError::BadTree("name is not UTF-8"));
        }

        let mut node = Node {
            parent,
            first_child: NONE,
            next_sibling: NONE,
            child_count: 0,
            agg_logical,
            agg_on_disk,
            name_off,
            name_len,
            depth,
            flags: node_flags,
            mtime,
        };
        if parent != NONE {
            // Same intrusive-list order as Tree::insert_child: newest first.
            let p = parent as usize;
            node.next_sibling = nodes[p].first_child;
            nodes[p].first_child = id as NodeId;
            nodes[p].child_count += 1;
        }
        nodes.push(node);
    }

    if rd.try_byte()?.is_some() {
        return Err(SnapshotError::BadTree("trailing bytes after the tree"));
    }

    let tree = Tree::from_parts(nodes, strings, 0);
    let cats = CategoryMap {
        cat: cats,
        tier: tiers,
    };
    Ok((tree, cats, rd.read))
}

/// Parsed header plus where the compressed body starts.
struct ParsedHeader {
    header: SnapshotHeader,
    body_start: usize,
    uncompressed_len: u64,
    compressed_len: u64,
}

fn parse_header(bytes: &[u8]) -> Result<ParsedHeader, SnapshotError> {
    let mut r = SliceReader::new(bytes);
    if r.take(8)? != SNAPSHOT_MAGIC {
        return Err(SnapshotError::BadMagic);
    }
    let format = r.u16()?;
    if format != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotError::UnsupportedFormat(format));
    }
    let codec = r.u16()?;
    if codec != SNAPSHOT_CODEC_VERSION {
        return Err(SnapshotError::UnsupportedCodec(codec));
    }
    let created_unix = r.u64()?;
    let tier = ScanTier::from_id(r.u8()?)?;
    if r.take(3)? != [0, 0, 0] {
        return Err(SnapshotError::BadHeader("reserved bytes are not zero"));
    }
    let counts = SnapshotCounts {
        nodes: r.u64()?,
        files: r.u64()?,
        dirs: r.u64()?,
        denied: r.u64()?,
        total_logical: r.u64()?,
        total_on_disk: r.u64()?,
    };
    let scan_ms = r.u64()?;
    let root_name = r.string32(MAX_ROOT_NAME_LEN)?;
    let app_version = r.string32(MAX_APP_VERSION_LEN)?;
    let uncompressed_len = r.u64()?;
    let compressed_len = r.u64()?;
    let body_start = r.pos;

    if counts.nodes == 0 {
        return Err(SnapshotError::BadHeader("node count is zero"));
    }
    if counts.nodes > u32::MAX as u64 {
        return Err(SnapshotError::NodeLimit(counts.nodes));
    }
    if counts.files + counts.dirs != counts.nodes {
        return Err(SnapshotError::BadHeader(
            "file and dir counts do not add up",
        ));
    }
    if counts.denied > counts.nodes {
        return Err(SnapshotError::BadHeader("denied count exceeds node count"));
    }
    if compressed_len == 0 {
        return Err(SnapshotError::BadHeader("body is empty"));
    }

    Ok(ParsedHeader {
        header: SnapshotHeader {
            meta: SnapshotMeta {
                created_unix,
                scan_ms,
                tier,
                root_name,
                app_version,
            },
            counts,
        },
        body_start,
        uncompressed_len,
        compressed_len,
    })
}

/// Reader over the (already loaded) file header slice.
struct SliceReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SliceReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        SliceReader { buf, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.pos.checked_add(len).ok_or(SnapshotError::Truncated)?;
        if end > self.buf.len() {
            return Err(SnapshotError::Truncated);
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string32(&mut self, max: usize) -> Result<String, SnapshotError> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        if len > max {
            return Err(SnapshotError::BadHeader("string is too long"));
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| SnapshotError::BadHeader("string is not UTF-8"))
    }
}

/// Reader over the decompressed body that counts bytes consumed.
struct BodyReader<'a, R: Read> {
    r: &'a mut R,
    read: u64,
}

impl<'a, R: Read> BodyReader<'a, R> {
    fn new(r: &'a mut R) -> Self {
        BodyReader { r, read: 0 }
    }

    fn fill(&mut self, buf: &mut [u8]) -> Result<(), SnapshotError> {
        self.r.read_exact(buf).map_err(io_error)?;
        self.read += buf.len() as u64;
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, SnapshotError> {
        let mut b = [0u8; 4];
        self.fill(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn u16(&mut self) -> Result<u16, SnapshotError> {
        let mut b = [0u8; 2];
        self.fill(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        let mut b = [0u8; 8];
        self.fill(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    /// Read exactly `len` bytes, growing as the bytes arrive (never trusting
    /// the length field for a single big allocation).
    fn bytes(&mut self, len: usize) -> Result<Vec<u8>, SnapshotError> {
        let mut out = Vec::new();
        let got = self
            .r
            .by_ref()
            .take(len as u64)
            .read_to_end(&mut out)
            .map_err(io_error)?;
        self.read += got as u64;
        if got != len {
            return Err(SnapshotError::Truncated);
        }
        Ok(out)
    }

    /// One byte, or `None` at the end of the body.
    fn try_byte(&mut self) -> Result<Option<u8>, SnapshotError> {
        let mut b = [0u8; 1];
        match self.r.read(&mut b) {
            Ok(0) => Ok(None),
            Ok(_) => {
                self.read += 1;
                Ok(Some(b[0]))
            }
            Err(e) => Err(io_error(e)),
        }
    }
}

fn io_error(e: std::io::Error) -> SnapshotError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        SnapshotError::Truncated
    } else {
        SnapshotError::Codec(e.to_string())
    }
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_string32(out: &mut Vec<u8>, s: &str) -> Result<(), SnapshotError> {
    let len = u32::try_from(s.len()).map_err(|_| SnapshotError::BadHeader("string is too long"))?;
    put_u32(out, len);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categorizer::{categorize, Rules};
    use crate::rollup::recompute_rollups;

    fn meta() -> SnapshotMeta {
        SnapshotMeta {
            created_unix: 1_763_500_000,
            scan_ms: 1_234,
            tier: ScanTier::Mft,
            root_name: "C:".to_string(),
            app_version: "0.1.0".to_string(),
        }
    }

    /// root 900: a 400 (dir) -> a1 200 (dir) -> a1x 200; a2 200; b 300; c 200
    /// plus a denied directory and a reparse point, to cover every flag.
    fn build() -> Snapshot {
        let mut t = Tree::new();
        let root = t.insert_root("C:");
        let a = t.insert_child(root, "a", flags::DIR, 1_700_000_000_000_000_000, 0, 0);
        let a1 = t.insert_child(a, "a1", flags::DIR, 0, 0, 0);
        t.insert_child(a1, "a1x", 0, 5, 200, 200);
        t.insert_child(a, "a2", 0, 0, 200, 400);
        t.insert_child(root, "b", 0, 0, 300, 300);
        t.insert_child(root, "c", 0, 0, 200, 200);
        t.insert_child(root, "denied", flags::DIR | flags::DENIED, 0, 0, 0);
        t.insert_child(root, "link", flags::REPARSE, 0, 0, 0);
        let big_name = "x".repeat(MAX_NAME_BYTES);
        t.insert_child(root, &big_name, 0, 0, 1, 1);
        recompute_rollups(&mut t);
        let cats = categorize(&t, &Rules::default_rules());
        Snapshot {
            meta: meta(),
            tree: t,
            cats,
        }
    }

    fn assert_same_tree(a: &Tree, b: &Tree) {
        assert_eq!(a.len(), b.len());
        assert_eq!(a.root(), b.root());
        for id in 0..a.len() as u32 {
            let (x, y) = (a.node(id), b.node(id));
            assert_eq!(a.name(id), b.name(id), "name {id}");
            assert_eq!(x.parent, y.parent, "parent {id}");
            assert_eq!(x.first_child, y.first_child, "first_child {id}");
            assert_eq!(x.next_sibling, y.next_sibling, "next_sibling {id}");
            assert_eq!(x.child_count, y.child_count, "child_count {id}");
            assert_eq!(x.agg_logical, y.agg_logical, "logical {id}");
            assert_eq!(x.agg_on_disk, y.agg_on_disk, "on_disk {id}");
            assert_eq!(x.name_off, y.name_off, "name_off {id}");
            assert_eq!(x.name_len, y.name_len, "name_len {id}");
            assert_eq!(x.depth, y.depth, "depth {id}");
            assert_eq!(x.flags, y.flags, "flags {id}");
            assert_eq!(x.mtime, y.mtime, "mtime {id}");
        }
        // Child iteration order is part of the contract (newest first).
        for id in 0..a.len() as u32 {
            let left: Vec<NodeId> = a.children(id).collect();
            let right: Vec<NodeId> = b.children(id).collect();
            assert_eq!(left, right, "children of {id}");
        }
    }

    #[test]
    fn roundtrip_preserves_tree_categories_and_meta() {
        let snap = build();
        let bytes = save_snapshot(&snap).unwrap();
        let loaded = load_snapshot(&bytes).unwrap();

        assert_eq!(loaded.meta, snap.meta);
        assert_same_tree(&snap.tree, &loaded.tree);
        assert_eq!(loaded.cats.cat, snap.cats.cat);
        assert_eq!(loaded.cats.tier, snap.cats.tier);

        // Counts are recomputed from the tree, not trusted from the file.
        let counts = tree_counts(&loaded.tree);
        assert_eq!(counts.nodes, 10);
        assert_eq!(counts.dirs, 4); // a, a1, denied, root
        assert_eq!(counts.files, 6);
        assert_eq!(counts.denied, 1);
        assert_eq!(counts.total_logical, 901);
        assert_eq!(counts.total_on_disk, 1_101);

        let header = read_header(&bytes).unwrap();
        assert_eq!(header.meta, snap.meta);
        assert_eq!(header.counts, counts);
    }

    #[test]
    fn header_can_be_read_from_a_prefix() {
        let snap = build();
        let strings_len = snap.tree.string_bytes().len();
        let bytes = save_snapshot(&snap).unwrap();
        // Find the body start the same way the loader does, then prove a
        // header-only prefix parses and reports the real sizes.
        let parsed = parse_header(&bytes).unwrap();
        assert!(parsed.body_start < bytes.len());
        // MAX_HEADER_LEN is the contract the snapshot-store UI relies on.
        assert!(parsed.body_start <= MAX_HEADER_LEN);
        let prefix = &bytes[..parsed.body_start];
        let header = read_header(prefix).unwrap();
        assert_eq!(header.meta.root_name, "C:");
        assert_eq!(header.counts.nodes, 10);
        assert_eq!(header.counts.total_logical, 901);
        assert_eq!(
            parsed.compressed_len as usize,
            bytes.len() - parsed.body_start
        );
        assert_eq!(parsed.uncompressed_len as usize, body_len(10, strings_len));
    }

    #[test]
    fn save_is_deterministic() {
        let snap = build();
        let a = save_snapshot(&snap).unwrap();
        let b = save_snapshot(&build()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_tree_and_bad_category_length_are_refused() {
        let empty = Snapshot {
            meta: meta(),
            tree: Tree::new(),
            cats: CategoryMap {
                cat: Vec::new(),
                tier: Vec::new(),
            },
        };
        assert_eq!(save_snapshot(&empty), Err(SnapshotError::EmptyTree));

        let mut snap = build();
        snap.cats.cat.pop();
        assert!(matches!(
            save_snapshot(&snap),
            Err(SnapshotError::CategoryLength {
                nodes: 10,
                cats: 9,
                tiers: 10
            })
        ));

        // A map with an id the wire format does not define is refused at
        // save time, so the app can never write a file it cannot reload.
        let mut snap = build();
        snap.cats.cat[0] = 99;
        assert_eq!(save_snapshot(&snap), Err(SnapshotError::BadCategoryMap));
    }

    #[test]
    fn magic_and_versions_are_gated() {
        let bytes = save_snapshot(&build()).unwrap();

        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(load_snapshot(&bad).unwrap_err(), SnapshotError::BadMagic);

        let mut bad = bytes.clone();
        bad[8..10].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            load_snapshot(&bad).unwrap_err(),
            SnapshotError::UnsupportedFormat(2)
        );

        let mut bad = bytes.clone();
        bad[10..12].copy_from_slice(&9u16.to_le_bytes());
        assert_eq!(
            load_snapshot(&bad).unwrap_err(),
            SnapshotError::UnsupportedCodec(9)
        );

        let mut bad = bytes.clone();
        bad[20] = 7; // scan tier
        assert_eq!(load_snapshot(&bad).unwrap_err(), SnapshotError::BadTier(7));
    }

    #[test]
    fn corruption_in_the_body_is_detected() {
        let bytes = save_snapshot(&build()).unwrap();
        let parsed = parse_header(&bytes).unwrap();
        // Flip a byte in the middle of the compressed body: zstd's content
        // checksum must refuse it.
        let mut bad = bytes.clone();
        let at = parsed.body_start + parsed.compressed_len as usize / 2;
        bad[at] ^= 0xFF;
        let err = load_snapshot(&bad).unwrap_err();
        assert!(
            matches!(
                err,
                SnapshotError::Codec(_)
                    | SnapshotError::BadTree(_)
                    | SnapshotError::Truncated
                    | SnapshotError::SizeMismatch
            ),
            "flipped body byte must fail, got {err:?}"
        );
    }

    #[test]
    fn truncation_is_detected_at_every_boundary() {
        let bytes = save_snapshot(&build()).unwrap();
        let parsed = parse_header(&bytes).unwrap();
        // Header cut short.
        for cut in [0, 4, 20, parsed.body_start - 1] {
            assert_eq!(
                load_snapshot(&bytes[..cut]).unwrap_err(),
                SnapshotError::Truncated
            );
        }
        // Body cut short (declared lengths no longer fit).
        for cut in [parsed.body_start, bytes.len() - 1] {
            assert_eq!(
                load_snapshot(&bytes[..cut]).unwrap_err(),
                SnapshotError::Truncated
            );
        }
        // Trailing garbage is refused, not ignored.
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(
            load_snapshot(&extra).unwrap_err(),
            SnapshotError::SizeMismatch
        );
    }

    /// Build a valid file around a hand-crafted (possibly invalid) body.
    fn craft(counts: SnapshotCounts, body: &[u8]) -> Vec<u8> {
        let raw_len = body.len() as u64;
        let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        enc.include_checksum(true).unwrap();
        enc.write_all(body).unwrap();
        let compressed = enc.finish().unwrap();

        let mut out = Vec::new();
        out.extend_from_slice(&SNAPSHOT_MAGIC);
        put_u16(&mut out, SNAPSHOT_FORMAT_VERSION);
        put_u16(&mut out, SNAPSHOT_CODEC_VERSION);
        put_u64(&mut out, 1);
        out.push(ScanTier::Walker.id());
        out.extend_from_slice(&[0, 0, 0]);
        put_u64(&mut out, counts.nodes);
        put_u64(&mut out, counts.files);
        put_u64(&mut out, counts.dirs);
        put_u64(&mut out, counts.denied);
        put_u64(&mut out, counts.total_logical);
        put_u64(&mut out, counts.total_on_disk);
        put_u64(&mut out, 0);
        put_string32(&mut out, "C:").unwrap();
        put_string32(&mut out, "test").unwrap();
        put_u64(&mut out, raw_len);
        put_u64(&mut out, compressed.len() as u64);
        out.extend_from_slice(&compressed);
        out
    }

    /// Body with two nodes: root (0) then `parent` at index 1.
    fn two_node_body(parent: u32, depth: u16, name_len: u16, cat: u8) -> Vec<u8> {
        let mut body = Vec::new();
        put_u32(&mut body, 2);
        let strings = b"C:ok";
        put_u32(&mut body, strings.len() as u32);
        body.extend_from_slice(strings);
        put_u32(&mut body, 2);
        body.extend_from_slice(&[cat, category::OTHER]);
        put_u32(&mut body, 2);
        body.extend_from_slice(&[tier::SAFE, tier::SAFE]);
        for (id, p, d, off, len) in [
            (0usize, NONE, 0u16, 0u32, 2u16),
            (1, parent, depth, 2, name_len),
        ] {
            put_u32(&mut body, p);
            put_u32(&mut body, off);
            put_u64(&mut body, if id == 0 { 10 } else { 0 });
            put_u64(&mut body, 0);
            put_u64(&mut body, 0);
            put_u16(&mut body, len);
            put_u16(&mut body, d);
            put_u16(&mut body, 0);
        }
        body
    }

    fn counts_for(n: u64, dirs: u64) -> SnapshotCounts {
        SnapshotCounts {
            nodes: n,
            files: n - dirs,
            dirs,
            denied: 0,
            total_logical: 0,
            total_on_disk: 0,
        }
    }

    #[test]
    fn structural_violations_are_refused() {
        // A valid control: child of root, depth 1.
        let good = craft(counts_for(2, 1), &two_node_body(0, 1, 2, category::CODE));
        assert!(load_snapshot(&good).is_ok());

        // Parent must precede the child (here it equals the child id).
        let body = two_node_body(1, 1, 2, category::CODE);
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("parent does not precede the child")
        );

        // Depth must be parent depth + 1.
        let body = two_node_body(0, 3, 2, category::CODE);
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("depth does not match the parent")
        );

        // Name must lie inside the arena.
        let body = two_node_body(0, 1, 9, category::CODE);
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("name is outside the string arena")
        );

        // Unknown category ids are refused (wire ids 0..=6 only).
        let body = two_node_body(0, 1, 2, 99);
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("unknown category id")
        );

        // Header counts must add up; a body claiming a different node count
        // than the header is refused before the tree is built.
        let body = two_node_body(0, 1, 2, category::CODE);
        assert_eq!(
            load_snapshot(&craft(counts_for(3, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("node count does not match the header")
        );
    }

    #[test]
    fn non_utf8_name_is_refused() {
        let mut body = two_node_body(0, 1, 2, category::CODE);
        // The arena starts after: u32 count + u32 strings_len = 8 bytes, and
        // the second name ("ok") starts 2 bytes into it. Make it invalid.
        body[8 + 2] = 0xFF;
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("name is not UTF-8")
        );
    }

    #[test]
    fn body_node_count_repeat_is_checked() {
        let mut body = two_node_body(0, 1, 2, category::CODE);
        body[0..4].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(
            load_snapshot(&craft(counts_for(2, 1), &body)).unwrap_err(),
            SnapshotError::BadTree("node count does not match the header")
        );
    }
}
