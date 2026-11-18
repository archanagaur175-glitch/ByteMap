import { describe, expect, it } from "vitest";
import { CANVAS, STRATA_VERSION, hitTestStrata, parseStrata } from "./strata";
import type { Strata, StrataRect } from "./strata";

// Mirrors crates/bytemap-core/src/strata.rs::serialize_strata.
function encode(s: Omit<Strata, "maxDepth" | "centerLogical" | "centerOnDisk"> & {
  centerLogical: number;
  centerOnDisk: number;
  maxDepth: number;
}): ArrayBuffer {
  const chunks: number[] = [];
  const u16 = (v: number) => chunks.push(v & 0xff, (v >> 8) & 0xff);
  const u8 = (v: number) => chunks.push(v & 0xff);
  const u32 = (v: number) =>
    chunks.push(v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >>> 24) & 0xff);
  const u64 = (v: number) => {
    u32(v >>> 0);
    u32(Math.floor(v / 0x100000000) >>> 0);
  };
  const str = (v: string) => {
    const bytes = new TextEncoder().encode(v);
    u16(bytes.length);
    for (const b of bytes) chunks.push(b);
  };
  u16(STRATA_VERSION);
  u32(s.center);
  u64(s.centerLogical);
  u64(s.centerOnDisk);
  u16(s.maxDepth);
  u32(s.rects.length);
  for (const r of s.rects) {
    u32(r.id);
    u16(r.depth);
    u32(r.x);
    u32(r.y);
    u32(r.w);
    u32(r.h);
    u64(r.logical);
    u64(r.onDisk);
    u16(r.flags);
    u8(r.cat);
    u8(r.tier);
    str(r.name);
  }
  return Uint8Array.from(chunks).buffer;
}

// Two layer-1 tiles (a on the left half, b on the right half) and one
// layer-2 tile inside a's left quarter.
const rects: StrataRect[] = [
  { id: 2, depth: 1, x: 0, y: 0, w: 5000, h: 10000, logical: 500, onDisk: 500, flags: 1, cat: 0, tier: 0, name: "a" },
  { id: 3, depth: 1, x: 5000, y: 0, w: 5000, h: 10000, logical: 300, onDisk: 300, flags: 1, cat: 5, tier: 1, name: "b" },
  { id: 4, depth: 2, x: 0, y: 0, w: 5000, h: 5000, logical: 250, onDisk: 250, flags: 0, cat: 3, tier: 2, name: "a1" },
];

const sample: Strata = {
  center: 1,
  centerLogical: 800,
  centerOnDisk: 800,
  maxDepth: 2,
  rects,
};

const buf = encode(sample);

describe("parseStrata", () => {
  it("decodes every field of the packed layout", () => {
    const s = parseStrata(buf);
    expect(s.center).toBe(1);
    expect(s.centerLogical).toBe(800);
    expect(s.centerOnDisk).toBe(800);
    expect(s.maxDepth).toBe(2);
    expect(s.rects).toHaveLength(3);
    expect(s.rects[0]).toEqual(rects[0]);
    expect(s.rects[2].name).toBe("a1");
    expect(s.rects[2].tier).toBe(2);
  });

  it("rejects a wrong version", () => {
    const bytes = new Uint8Array(buf.slice(0));
    bytes[0] = 9;
    expect(() => parseStrata(bytes.buffer)).toThrow(/unsupported strata version/);
  });

  it("rejects trailing bytes", () => {
    const bytes = new Uint8Array(buf.byteLength + 1);
    bytes.set(new Uint8Array(buf));
    expect(() => parseStrata(bytes.buffer)).toThrow(/trailing bytes/);
  });

  it("rejects a truncated buffer", () => {
    expect(() => parseStrata(buf.slice(0, 12))).toThrow(/truncated/);
  });
});

describe("hitTestStrata", () => {
  const strata = parseStrata(buf);

  it("returns the deepest tile under the point", () => {
    // (0.1, 0.1) of the canvas: inside a (layer 1) and a1 (layer 2).
    expect(hitTestStrata(strata, 100, 100, 1000, 1000)?.name).toBe("a1");
    // (0.1, 0.9): inside a only.
    expect(hitTestStrata(strata, 100, 900, 1000, 1000)?.name).toBe("a");
    // (0.9, 0.5): inside b only.
    expect(hitTestStrata(strata, 900, 500, 1000, 1000)?.name).toBe("b");
  });

  it("returns null outside every tile and for a zero-size canvas", () => {
    expect(hitTestStrata(strata, 1000, 500, 1000, 1000)).toBeNull();
    expect(hitTestStrata(strata, -1, 100, 1000, 1000)).toBeNull();
    expect(hitTestStrata(strata, 100, 100, 0, 1000)).toBeNull();
  });

  it("scales with a non-square canvas", () => {
    // Half-width 400px, so x=350 is inside a's 5000/10000 span.
    expect(hitTestStrata(strata, 350, 60, 800, 600)?.name).toBe("a1");
    expect(hitTestStrata(strata, 600, 300, 800, 600)?.name).toBe("b");
  });

  it("hits the whole canvas for a lone layer-1 tile", () => {
    const solo: Strata = {
      center: 1,
      centerLogical: 10,
      centerOnDisk: 10,
      maxDepth: 1,
      rects: [{ ...rects[0], x: 0, y: 0, w: CANVAS, h: CANVAS }],
    };
    expect(hitTestStrata(solo, 1, 1, 100, 100)?.name).toBe("a");
    expect(hitTestStrata(solo, 99, 99, 100, 100)?.name).toBe("a");
  });
});
