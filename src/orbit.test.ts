import { describe, expect, it } from "vitest";
import { parseOrbit, hitTest, ORBIT_VERSION } from "./orbit";
import type { Orbit } from "./orbit";
import { mix } from "./renderer";

// Mirrors crates/bytemap-core/src/orbit.rs::serialize_orbit.
function encode(o: {
  center: number;
  centerLogical: number;
  centerOnDisk: number;
  rings: number;
  arcs: Array<{ id: number; depth: number; start: number; sweep: number; logical: number; onDisk: number; flags: number; cat: number; tier: number; name: string }>;
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
  const str = (s: string) => {
    const bytes = new TextEncoder().encode(s);
    u16(bytes.length);
    for (const b of bytes) chunks.push(b);
  };
  u16(ORBIT_VERSION);
  u32(o.center);
  u64(o.centerLogical);
  u64(o.centerOnDisk);
  u16(o.rings);
  u32(o.arcs.length);
  for (const a of o.arcs) {
    u32(a.id);
    u16(a.depth);
    u16(Math.round(a.start * 65536));
    u16(Math.round(a.sweep * 65536));
    u64(a.logical);
    u64(a.onDisk);
    u16(a.flags);
    u8(a.cat);
    u8(a.tier);
    str(a.name);
  }
  return Uint8Array.from(chunks).buffer;
}

const sample: Orbit = {
  center: 1,
  centerLogical: 1000,
  centerOnDisk: 1000,
  rings: 2,
  arcs: [
    { id: 2, depth: 1, start: 0, sweep: 0.5, logical: 500, onDisk: 500, flags: 1, cat: 0, tier: 0, name: "a" },
    { id: 3, depth: 1, start: 0.5, sweep: 0.3, logical: 300, onDisk: 300, flags: 1, cat: 5, tier: 1, name: "b" },
    { id: 4, depth: 2, start: 0.25, sweep: 0.25, logical: 250, onDisk: 250, flags: 0, cat: 3, tier: 2, name: "a1" },
  ],
};

describe("parseOrbit", () => {
  it("decodes the packed layout", () => {
    const orbit = parseOrbit(encode(sample));
    expect(orbit.center).toBe(1);
    expect(orbit.centerLogical).toBe(1000);
    expect(orbit.rings).toBe(2);
    expect(orbit.arcs).toHaveLength(3);
    expect(orbit.arcs[0]).toMatchObject({
      id: 2,
      depth: 1,
      start: 0,
      sweep: 0.5,
      name: "a",
    });
    // v2 fields survive the round trip.
    expect(orbit.arcs[1].cat).toBe(5);
    expect(orbit.arcs[2].tier).toBe(2);
  });

  it("rejects unknown versions", () => {
    const buf = encode(sample);
    new DataView(buf).setUint16(0, 99, true);
    expect(() => parseOrbit(buf)).toThrow(/unsupported orbit version/);
  });
});

describe("hitTest", () => {
  const geom = { cx: 100, cy: 100, innerR: 20, ringWidth: 15, rings: 2 };

  it("returns null on the center disc and outside the outer ring", () => {
    expect(hitTest(sample, geom, 100, 100)).toBeNull();
    expect(hitTest(sample, geom, 195, 100)).toBeNull(); // r=95 > 50
  });

  it("finds arcs by ring and angle", () => {
    // r=25 (ring 1), angle 0 (turn 0) -> a.
    expect(hitTest(sample, geom, 125, 100)?.name).toBe("a");
    // r=25, angle 0.6 turn (216 deg): y down is clockwise -> dy positive.
    expect(hitTest(sample, geom, 100 + 25 * Math.cos(0.6 * 2 * Math.PI), 100 + 25 * Math.sin(0.6 * 2 * Math.PI))?.name).toBe("b");
    // r=40 (ring 2), angle 0.3 turn -> a1.
    expect(hitTest(sample, geom, 100 + 40 * Math.cos(0.3 * 2 * Math.PI), 100 + 40 * Math.sin(0.3 * 2 * Math.PI))?.name).toBe("a1");
    // Ring 2 at 0.6 turn: nothing there -> null.
    expect(hitTest(sample, geom, 100 + 40 * Math.cos(0.6 * 2 * Math.PI), 100 + 40 * Math.sin(0.6 * 2 * Math.PI))).toBeNull();
  });

  it("handles arcs that wrap past turn 1", () => {
    const wrapped: Orbit = {
      ...sample,
      arcs: [{ id: 9, depth: 1, start: 0.9, sweep: 0.2, logical: 1, onDisk: 1, flags: 0, cat: 6, tier: 1, name: "wrap" }],
    };
    // 0.95 turn (inside the wrapped part).
    const p = (t: number) => [100 + 25 * Math.cos(t * 2 * Math.PI), 100 + 25 * Math.sin(t * 2 * Math.PI)];
    const [x, y] = p(0.95);
    expect(hitTest(wrapped, geom, x, y)?.name).toBe("wrap");
    const [x2, y2] = p(0.5);
    expect(hitTest(wrapped, geom, x2, y2)).toBeNull();
  });
});

describe("mix", () => {
  it("blends channel-wise and clamps to hex output", () => {
    expect(mix("#000000", "#ffffff", 0.5)).toBe("#808080");
    expect(mix("#6ea8ff", "#6ea8ff", 0.9)).toBe("#6ea8ff");
  });
});
