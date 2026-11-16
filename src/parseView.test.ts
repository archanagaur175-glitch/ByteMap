import { describe, expect, it } from "vitest";
import {
  parseView,
  FOLDED_ID,
  formatBytes,
  FLAG_FOLDED,
  CAT_DOCUMENTS,
  TIER_REVIEW,
  TIER_PROTECTED,
} from "./parseView";

// Builds the same little-endian format the Rust side emits
// (mirrors crates/bytemap-core/src/view.rs::serialize_view, v2).
function encode(view: {
  requested: number;
  ancestors: Array<[number, string]>;
  totalChildren: number;
  items: Array<{ id: number; logical: number; onDisk: number; flags: number; cat: number; tier: number; name: string }>;
}): ArrayBuffer {
  const chunks: number[] = [];
  const u16 = (v: number) => chunks.push(v & 0xff, (v >> 8) & 0xff);
  const u8 = (v: number) => chunks.push(v & 0xff);
  const u32 = (v: number) => chunks.push(v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >>> 24) & 0xff);
  const u64 = (v: number) => {
    u32(v >>> 0);
    u32(Math.floor(v / 0x100000000) >>> 0);
  };
  const str = (s: string) => {
    const bytes = new TextEncoder().encode(s);
    u16(bytes.length);
    for (const b of bytes) chunks.push(b);
  };
  u16(2);
  u16(view.ancestors.length);
  u32(view.requested);
  for (const [id, name] of view.ancestors) {
    u32(id);
    str(name);
  }
  u32(view.totalChildren);
  u32(view.items.length);
  for (const it of view.items) {
    u32(it.id);
    u64(it.logical);
    u64(it.onDisk);
    u16(it.flags);
    u8(it.cat);
    u8(it.tier);
    str(it.name);
  }
  return Uint8Array.from(chunks).buffer;
}

describe("parseView", () => {
  it("decodes a view with ancestors and a folded item", () => {
    const buf = encode({
      requested: 7,
      ancestors: [
        [1, "C:"],
        [3, "Users"],
      ],
      totalChildren: 4,
      items: [
        { id: 11, logical: 1073741824, onDisk: 1073741824, flags: 1, cat: CAT_DOCUMENTS, tier: TIER_REVIEW, name: "big" },
        { id: FOLDED_ID, logical: 16, onDisk: 12288, flags: FLAG_FOLDED | 1, cat: 6, tier: 1, name: "3 small items" },
      ],
    });
    const view = parseView(buf);
    expect(view.requested).toBe(7);
    expect(view.ancestors.map((a) => a.name)).toEqual(["C:", "Users"]);
    expect(view.totalChildren).toBe(4);
    expect(view.items[0].name).toBe("big");
    expect(view.items[0].logical).toBe(1073741824);
    expect(view.items[0].cat).toBe(CAT_DOCUMENTS);
    expect(view.items[0].tier).toBe(TIER_REVIEW);
    expect(view.items[1].isFolded).toBe(true);
    expect(view.items[1].logical).toBe(16);
    expect(view.items[1].onDisk).toBe(12288);
  });

  it("rejects unknown versions", () => {
    const bad = encode({
      requested: 0,
      ancestors: [],
      totalChildren: 0,
      items: [],
    });
    new DataView(bad).setUint16(0, 99, true);
    expect(() => parseView(bad)).toThrow(/unsupported view version/);
  });

  it("round-trips tier ids including protected", () => {
    const buf = encode({
      requested: 1,
      ancestors: [],
      totalChildren: 1,
      items: [
        { id: 2, logical: 10, onDisk: 10, flags: 0, cat: 6, tier: TIER_PROTECTED, name: "pagefile.sys" },
      ],
    });
    const view = parseView(buf);
    expect(view.items[0].tier).toBe(TIER_PROTECTED);
  });
});

describe("formatBytes", () => {
  it("formats sizes readably", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1073741824)).toBe("1.0 GB");
  });
});
