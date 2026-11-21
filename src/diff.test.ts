import { describe, expect, it } from "vitest";
import {
  parseDiff, formatDelta, KIND_ADDED, KIND_SHRANK, DIFF_VERSION,
  type DiffEntry,
} from "./diff";
import { formatBytes } from "./parseView";

/** Little-endian writer mirroring bytemap-core::diff (test only). */
class Writer {
  private bytes: number[] = [];

  u8(v: number): void {
    this.bytes.push(v & 0xff);
  }

  u16(v: number): void {
    this.u8(v);
    this.u8(v >>> 8);
  }

  u32(v: number): void {
    this.u16(v & 0xffff);
    this.u16(Math.floor(v / 0x10000));
  }

  u64(v: number): void {
    this.u32(v % 0x100000000);
    this.u32(Math.floor(v / 0x100000000));
  }

  str32(s: string): void {
    const encoded = new TextEncoder().encode(s);
    this.u32(encoded.length);
    for (const b of encoded) this.bytes.push(b);
  }

  done(): ArrayBuffer {
    return new Uint8Array(this.bytes).buffer;
  }
}

interface Options {
  version?: number;
  totalChanged?: number;
  counts?: [number, number, number, number];
  bytes?: [number, number, number, number];
  totals?: [number, number];
  entries?: Array<Partial<DiffEntry>>;
  trailing?: boolean;
}

function build(opts: Options = {}): ArrayBuffer {
  const w = new Writer();
  w.u16(opts.version ?? DIFF_VERSION);
  const entries = opts.entries ?? [];
  w.u32(entries.length);
  w.u32(opts.totalChanged ?? entries.length);
  const [added, removed, grew, shrank] = opts.counts ?? [0, 0, 0, 0];
  w.u32(added);
  w.u32(removed);
  w.u32(grew);
  w.u32(shrank);
  const [addedB, removedB, grewB, shrankB] = opts.bytes ?? [0, 0, 0, 0];
  w.u64(addedB);
  w.u64(removedB);
  w.u64(grewB);
  w.u64(shrankB);
  const [beforeTotal, afterTotal] = opts.totals ?? [0, 0];
  w.u64(beforeTotal);
  w.u64(afterTotal);
  for (const e of entries) {
    w.u8(e.kind ?? 0);
    w.u8(e.isDir ? 1 : 0);
    w.u8(e.cat ?? 0);
    w.u8(e.tier ?? 0);
    w.u64(e.before ?? 0);
    w.u64(e.after ?? 0);
    w.str32(e.path ?? "");
  }
  if (opts.trailing) w.u8(0);
  return w.done();
}

describe("parseDiff", () => {
  it("decodes summary, entries, deltas and net", () => {
    const buf = build({
      totalChanged: 2,
      counts: [1, 0, 0, 1],
      bytes: [1234, 0, 0, 100],
      totals: [5000, 6134],
      entries: [
        {
          kind: KIND_ADDED, path: "C:\\fixture\\new — file.bin", isDir: false,
          before: 0, after: 1234, cat: 2, tier: 0,
        },
        {
          kind: KIND_SHRANK, path: "C:\\fixture\\old.log", isDir: false,
          before: 300, after: 200, cat: 4, tier: 1,
        },
      ],
    });
    const diff = parseDiff(buf);
    expect(diff.summary).toEqual({
      totalChanged: 2,
      addedCount: 1,
      removedCount: 0,
      grewCount: 0,
      shrankCount: 1,
      addedBytes: 1234,
      removedBytes: 0,
      grewBytes: 0,
      shrankBytes: 100,
      beforeTotal: 5000,
      afterTotal: 6134,
      net: 1134,
    });
    expect(diff.entries).toHaveLength(2);
    expect(diff.entries[0]).toMatchObject({
      kind: KIND_ADDED, isDir: false, before: 0, after: 1234, delta: 1234, cat: 2, tier: 0,
    });
    expect(diff.entries[0].path).toBe("C:\\fixture\\new — file.bin");
    expect(diff.entries[1].delta).toBe(-100);
    expect(diff.truncated).toBe(false);
  });

  it("marks the list truncated when the summary counted more changes", () => {
    const buf = build({
      totalChanged: 7,
      counts: [0, 0, 7, 0],
      entries: [
        { kind: 2, path: "C:\\a", isDir: false, before: 1, after: 2 },
      ],
    });
    const diff = parseDiff(buf);
    expect(diff.entries).toHaveLength(1);
    expect(diff.summary.totalChanged).toBe(7);
    expect(diff.truncated).toBe(true);
  });

  it("decodes an empty diff with a negative net", () => {
    const buf = build({ totals: [900, 400] });
    const diff = parseDiff(buf);
    expect(diff.entries).toHaveLength(0);
    expect(diff.summary.net).toBe(-500);
    expect(diff.truncated).toBe(false);
  });

  it("refuses an unknown version", () => {
    expect(() => parseDiff(build({ version: 2 }))).toThrow(/unsupported diff version 2/);
  });

  it("refuses a truncated buffer", () => {
    const bytes = new Uint8Array(build({
      entries: [{ path: "C:\\fixture\\a.bin", after: 10 }],
    }));
    expect(() => parseDiff(bytes.slice(0, bytes.length - 4).buffer)).toThrow(/truncated/);
  });

  it("refuses trailing bytes", () => {
    expect(() => parseDiff(build({ trailing: true }))).toThrow(/trailing bytes/);
  });
});

describe("formatDelta", () => {
  it("signs non-zero deltas and leaves zero plain", () => {
    expect(formatDelta(1536, formatBytes)).toBe("+1.5 KB");
    expect(formatDelta(-1536, formatBytes)).toBe("-1.5 KB");
    expect(formatDelta(0, formatBytes)).toBe("0 B");
  });
});
