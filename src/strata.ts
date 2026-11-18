// Decoder + hit-test for the packed binary Strata (layered treemap) format
// produced by bytemap-core::strata and sent via tauri::ipc::Response (LE).
//
// Layout (STRATA_VERSION 1):
//   u16 version
//   u32 center_id
//   u64 center_logical, u64 center_on_disk
//   u16 max_depth
//   u32 rect_count
//   per rect: u32 id, u16 depth, u32 x, u32 y, u32 w, u32 h,
//             u64 logical, u64 on_disk, u16 flags, u8 cat, u8 tier,
//             u16 name_len, name bytes
//
// Coordinates are canvas units of a CANVAS x CANVAS square; the renderer
// scales them to pixels, so a window resize never needs a new layout.

export const STRATA_VERSION = 1;

/** Must match bytemap-core::strata::CANVAS. */
export const CANVAS = 10000;

export interface StrataRect {
  id: number;
  /** 1 = child of the center, 2 = grandchild, ... */
  depth: number;
  /** Canvas units, 0..=CANVAS. */
  x: number;
  y: number;
  w: number;
  h: number;
  logical: number;
  onDisk: number;
  flags: number;
  /** Category id (CAT_*). */
  cat: number;
  /** Safety tier id (TIER_*). */
  tier: number;
  name: string;
}

export interface Strata {
  center: number;
  centerLogical: number;
  centerOnDisk: number;
  maxDepth: number;
  rects: StrataRect[];
}

class Reader {
  private view: DataView;
  private pos = 0;
  private utf8 = new TextDecoder();

  constructor(buf: ArrayBuffer) {
    this.view = new DataView(buf);
  }

  private need(n: number): void {
    if (this.pos + n > this.view.byteLength) {
      throw new Error("strata buffer truncated");
    }
  }

  private take(n: number): Uint8Array {
    this.need(n);
    const bytes = new Uint8Array(this.view.buffer, this.pos, n);
    this.pos += n;
    return bytes;
  }

  u8(): number {
    this.need(1);
    const v = this.view.getUint8(this.pos);
    this.pos += 1;
    return v;
  }

  u16(): number {
    this.need(2);
    const v = this.view.getUint16(this.pos, true);
    this.pos += 2;
    return v;
  }

  u32(): number {
    this.need(4);
    const v = this.view.getUint32(this.pos, true);
    this.pos += 4;
    return v;
  }

  u64(): number {
    this.need(8);
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

export function parseStrata(buf: ArrayBuffer): Strata {
  const r = new Reader(buf);
  const version = r.u16();
  if (version !== STRATA_VERSION) {
    throw new Error(`unsupported strata version ${version}`);
  }
  const center = r.u32();
  const centerLogical = r.u64();
  const centerOnDisk = r.u64();
  const maxDepth = r.u16();
  const count = r.u32();
  const rects: StrataRect[] = [];
  for (let i = 0; i < count; i++) {
    rects.push({
      id: r.u32(),
      depth: r.u16(),
      x: r.u32(),
      y: r.u32(),
      w: r.u32(),
      h: r.u32(),
      logical: r.u64(),
      onDisk: r.u64(),
      flags: r.u16(),
      cat: r.u8(),
      tier: r.u8(),
      name: r.str(),
    });
  }
  if (!r.atEnd()) {
    throw new Error("trailing bytes in strata buffer");
  }
  return { center, centerLogical, centerOnDisk, maxDepth, rects };
}

/**
 * The tile under pixel point (px, py) on a `width x height` canvas, or null.
 * Tiles are nested, so the DEEPEST hit wins (a grandchild tile sits inside
 * its parent's). Boundary rounding can make two sibling tiles overlap by a
 * unit: the deeper one is always the more specific answer.
 */
export function hitTestStrata(
  strata: Strata,
  px: number,
  py: number,
  width: number,
  height: number,
): StrataRect | null {
  if (width <= 0 || height <= 0) return null;
  const sx = width / CANVAS;
  const sy = height / CANVAS;
  let best: StrataRect | null = null;
  for (const rect of strata.rects) {
    if (rect.w === 0 || rect.h === 0) continue;
    const x0 = rect.x * sx;
    const y0 = rect.y * sy;
    const x1 = (rect.x + rect.w) * sx;
    const y1 = (rect.y + rect.h) * sy;
    if (px >= x0 && px < x1 && py >= y0 && py < y1) {
      if (best === null || rect.depth >= best.depth) best = rect;
    }
  }
  return best;
}
