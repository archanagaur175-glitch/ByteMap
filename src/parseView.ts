// Decoder for the packed binary view format produced by bytemap-core::view
// and sent via tauri::ipc::Response (little-endian).
//
// Layout:
//   u16 version
//   u16 ancestor_count
//   u32 requested_node_id
//   per ancestor: u32 id, u16 name_len, name bytes (UTF-8)
//   u32 total_child_count
//   u32 item_count
//   per item: u32 id (0xFFFFFFFF = folded), u64 logical, u64 on_disk,
//             u16 flags, u16 name_len, name bytes
//
// Node id u32::MAX (0xFFFFFFFF) is the folded "N small items" sentinel.
export const FOLDED_ID = 0xffffffff;

export const FLAG_DIR = 1;
export const FLAG_REPARSE = 2;
export const FLAG_PLACEHOLDER = 4;
export const FLAG_DENIED = 8;
export const FLAG_DUPLICATE_HARDLINK = 16;
export const FLAG_FOLDED = 32;

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

export interface Ancestor {
  id: number;
  name: string;
}

export interface ViewItem {
  id: number;
  logical: number;
  onDisk: number;
  flags: number;
  /** Category id (CAT_*); u8 from wire format v2. */
  cat: number;
  /** Safety tier id (TIER_*); u8 from wire format v2. */
  tier: number;
  name: string;
  isFolded: boolean;
}

export interface View {
  requested: number;
  ancestors: Ancestor[];
  totalChildren: number;
  items: ViewItem[];
}

export class Reader {
  private view: DataView;
  private pos = 0;
  private utf8 = new TextDecoder();

  constructor(buf: ArrayBuffer) {
    this.view = new DataView(buf);
  }

  private take(n: number): Uint8Array {
    if (this.pos + n > this.view.byteLength) {
      throw new Error("view buffer truncated");
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
    // Sizes fit below 2^53 for realistic volumes; use the safe BigInt path
    // only to keep the low bits exact.
    const lo = BigInt(this.view.getUint32(this.pos, true));
    const hi = BigInt(this.view.getUint32(this.pos + 4, true));
    this.pos += 8;
    return Number((hi << 32n) | lo);
  }

  str(): string {
    const len = this.u16();
    return this.utf8.decode(this.take(len));
  }

  /** u32-length string (the diff format uses wide paths). */
  str32(): string {
    const len = this.u32();
    return this.utf8.decode(this.take(len));
  }

  atEnd(): boolean {
    return this.pos === this.view.byteLength;
  }
}

export function parseView(buf: ArrayBuffer): View {
  const r = new Reader(buf);
  const version = r.u16();
  if (version !== 2) {
    throw new Error(`unsupported view version ${version}`);
  }
  const ancestorCount = r.u16();
  const requested = r.u32();
  const ancestors: Ancestor[] = [];
  for (let i = 0; i < ancestorCount; i++) {
    const id = r.u32();
    ancestors.push({ id, name: r.str() });
  }
  const totalChildren = r.u32();
  const itemCount = r.u32();
  const items: ViewItem[] = [];
  for (let i = 0; i < itemCount; i++) {
    const id = r.u32();
    const logical = r.u64();
    const onDisk = r.u64();
    const flags = r.u16();
    const cat = r.u8();
    const tier = r.u8();
    const name = r.str();
    items.push({ id, logical, onDisk, flags, cat, tier, name, isFolded: id === FOLDED_ID });
  }
  if (!r.atEnd()) {
    throw new Error("trailing bytes in view buffer");
  }
  return { requested, ancestors, totalChildren, items };
}

export function formatBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  const digits = v >= 100 || u === 0 ? 0 : 1;
  return `${v.toFixed(digits)} ${units[u]}`;
}
