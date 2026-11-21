// Decoder for the packed binary snapshot diff produced by
// bytemap-core::diff and sent via tauri::ipc::Response (little-endian).
//
// Layout (version 1):
//   u16 version
//   u32 returned_entry_count
//   u32 total_changed
//   u32 added_count, u32 removed_count, u32 grew_count, u32 shrank_count
//   u64 added_bytes, u64 removed_bytes, u64 grew_bytes, u64 shrank_bytes
//   u64 before_total, u64 after_total
//   per entry: u8 kind, u8 is_dir, u8 cat, u8 tier,
//              u64 before, u64 after, u32 path_len, path bytes (UTF-8)
//
// Deltas are `after - before`; `net` is `afterTotal - beforeTotal`.

import { Reader } from "./parseView";

export const DIFF_VERSION = 1;

export const KIND_ADDED = 0;
export const KIND_REMOVED = 1;
export const KIND_GREW = 2;
export const KIND_SHRANK = 3;

export interface DiffEntry {
  /** KIND_* id. */
  kind: number;
  /** Absolute path from the snapshot the entry exists on. */
  path: string;
  isDir: boolean;
  /** Logical size in the old snapshot (0 for added). */
  before: number;
  /** Logical size in the new snapshot (0 for removed). */
  after: number;
  /** after - before (negative for removed/shrank). */
  delta: number;
  /** Category id (CAT_*). */
  cat: number;
  /** Safety tier id (TIER_*). */
  tier: number;
}

export interface DiffSummary {
  /** Changes in the whole diff (may exceed entries.length). */
  totalChanged: number;
  addedCount: number;
  removedCount: number;
  grewCount: number;
  shrankCount: number;
  addedBytes: number;
  removedBytes: number;
  grewBytes: number;
  shrankBytes: number;
  beforeTotal: number;
  afterTotal: number;
  /** afterTotal - beforeTotal (signed). */
  net: number;
}

export interface Diff {
  summary: DiffSummary;
  /** Top movers: ranked by |delta|, capped by Rust. */
  entries: DiffEntry[];
  /** True when the ranked list was cut (the summary is complete). */
  truncated: boolean;
}

export function parseDiff(buf: ArrayBuffer): Diff {
  const r = new Reader(buf);
  const version = r.u16();
  if (version !== DIFF_VERSION) {
    throw new Error(`unsupported diff version ${version}`);
  }
  const returned = r.u32();
  const totalChanged = r.u32();
  const addedCount = r.u32();
  const removedCount = r.u32();
  const grewCount = r.u32();
  const shrankCount = r.u32();
  const addedBytes = r.u64();
  const removedBytes = r.u64();
  const grewBytes = r.u64();
  const shrankBytes = r.u64();
  const beforeTotal = r.u64();
  const afterTotal = r.u64();

  const entries: DiffEntry[] = [];
  for (let i = 0; i < returned; i++) {
    const kind = r.u8();
    const isDir = r.u8() !== 0;
    const cat = r.u8();
    const tier = r.u8();
    const before = r.u64();
    const after = r.u64();
    const path = r.str32();
    entries.push({ kind, path, isDir, before, after, delta: after - before, cat, tier });
  }
  if (!r.atEnd()) {
    throw new Error("trailing bytes in diff buffer");
  }

  return {
    summary: {
      totalChanged,
      addedCount,
      removedCount,
      grewCount,
      shrankCount,
      addedBytes,
      removedBytes,
      grewBytes,
      shrankBytes,
      beforeTotal,
      afterTotal,
      net: afterTotal - beforeTotal,
    },
    entries,
    truncated: totalChanged > entries.length,
  };
}

/** "+1.2 GB" / "-300 MB" (zero gets a plain "0 B"). */
export function formatDelta(delta: number, formatBytes: (n: number) => string): string {
  if (delta === 0) return "0 B";
  return `${delta > 0 ? "+" : "-"}${formatBytes(Math.abs(delta))}`;
}
