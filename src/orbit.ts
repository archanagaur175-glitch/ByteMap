// Decoder + geometry for the packed binary orbit (sunburst) format
// produced by bytemap-core::orbit and sent via tauri::ipc::Response (LE).
//
// Layout:
//   u16 version (= 1)
//   u32 center_id
//   u64 center_logical, u64 center_on_disk
//   u16 ring_count
//   u32 arc_count
//   per arc: u32 id, u16 depth, u16 start, u16 sweep, u64 logical,
//            u64 on_disk, u16 flags, u16 name_len, name bytes
//
// Angles travel as fixed point (65536 = one full turn) and are converted
// to turns (0..1 floats) here; the renderer turns them into radians.

export const ORBIT_VERSION = 2;

export const FLAG_DIR = 1;

// Category ids (wire format v2; mirrors bytemap-core categorizer).
export const CAT_CODE = 0;
export const CAT_SYSTEM = 1;
export const CAT_ARCHIVES = 2;
export const CAT_MEDIA = 3;
export const CAT_DOCUMENTS = 4;
export const CAT_APPS = 5;
export const CAT_OTHER = 6;

// Safety tier ids (wire format v2).
export const TIER_SAFE = 0;
export const TIER_REVIEW = 1;
export const TIER_PROTECTED = 2;

export interface OrbitArc {
  id: number;
  /** 1 = child of the center, 2 = grandchild, ... */
  depth: number;
  /** Angular position in turns (0..1). */
  start: number;
  /** Angular width in turns. */
  sweep: number;
  logical: number;
  onDisk: number;
  flags: number;
  /** Category id (CAT_*); u8 from wire format v2. */
  cat: number;
  /** Safety tier id (TIER_*); u8 from wire format v2. */
  tier: number;
  name: string;
}

export interface Orbit {
  center: number;
  centerLogical: number;
  centerOnDisk: number;
  rings: number;
  arcs: OrbitArc[];
}

class Reader {
  private view: DataView;
  private pos = 0;
  private utf8 = new TextDecoder();

  constructor(buf: ArrayBuffer) {
    this.view = new DataView(buf);
  }

  private take(n: number): Uint8Array {
    if (this.pos + n > this.view.byteLength) {
      throw new Error("orbit buffer truncated");
    }
    const bytes = new Uint8Array(this.view.buffer, this.pos, n);
    this.pos += n;
    return bytes;
  }

  u16(): number {
    const v = this.view.getUint16(this.pos, true);
    this.pos += 2;
    return v;
  }

  u8(): number {
    const v = this.view.getUint8(this.pos);
    this.pos += 1;
    return v;
  }

  u32(): number {
    const v = this.view.getUint32(this.pos, true);
    this.pos += 4;
    return v;
  }

  u64(): number {
    const lo = BigInt(this.view.getUint32(this.pos, true));
    const hi = BigInt(this.view.getUint32(this.pos + 4, true));
    this.pos += 8;
    return Number((hi << 32n) | lo);
  }

  str(): string {
    const len = this.u16();
    return this.utf8.decode(this.take(len));
  }

  atEnd(): boolean {
    return this.pos === this.view.byteLength;
  }
}

export function parseOrbit(buf: ArrayBuffer): Orbit {
  const r = new Reader(buf);
  const version = r.u16();
  if (version !== ORBIT_VERSION) {
    throw new Error(`unsupported orbit version ${version}`);
  }
  const center = r.u32();
  const centerLogical = r.u64();
  const centerOnDisk = r.u64();
  const rings = r.u16();
  const arcCount = r.u32();
  const arcs: OrbitArc[] = [];
  for (let i = 0; i < arcCount; i++) {
    const id = r.u32();
    const depth = r.u16();
    const start = r.u16() / 65536;
    const sweep = r.u16() / 65536;
    const logical = r.u64();
    const onDisk = r.u64();
    const flags = r.u16();
    const cat = r.u8();
    const tier = r.u8();
    const name = r.str();
    arcs.push({ id, depth, start, sweep, logical, onDisk, flags, cat, tier, name });
  }
  if (!r.atEnd()) {
    throw new Error("trailing bytes in orbit buffer");
  }
  return { center, centerLogical, centerOnDisk, rings, arcs };
}

/** Canvas geometry of the sunburst (set by the renderer, used by hit-test). */
export interface OrbitGeometry {
  cx: number;
  cy: number;
  innerR: number;
  ringWidth: number;
  rings: number;
}

/**
 * Returns the arc under canvas point (x, y), or null for the center disc
 * and empty space. Angles: 0 turns = +x axis, increasing clockwise on
 * screen (canvas y is down) — must match the renderer's arc direction.
 */
export function hitTest(
  orbit: Orbit,
  geom: OrbitGeometry,
  x: number,
  y: number,
): OrbitArc | null {
  const dx = x - geom.cx;
  const dy = y - geom.cy;
  const r = Math.hypot(dx, dy);
  if (r < geom.innerR) return null; // center disc, not clickable
  const ring = Math.floor((r - geom.innerR) / geom.ringWidth) + 1;
  if (ring > Math.min(geom.rings, orbit.rings)) return null;
  let angle = Math.atan2(dy, dx);
  if (angle < 0) angle += Math.PI * 2;
  const turn = angle / (Math.PI * 2);
  for (const arc of orbit.arcs) {
    if (arc.depth !== ring) continue;
    const end = arc.start + arc.sweep;
    const wrapped = end > 1;
    if (wrapped ? turn >= arc.start || turn < end - 1 : turn >= arc.start && turn < end) {
      return arc;
    }
  }
  return null;
}
