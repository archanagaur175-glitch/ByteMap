// Application store (zustand). The frontend holds only views; the tree
// lives in Rust. Item identity = node id, never a path string.

import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import { parseView } from "./parseView";
import { parseOrbit } from "./orbit";
import type { View, ViewItem } from "./parseView";
import type { Orbit } from "./orbit";

export type Phase = "idle" | "scanning" | "done" | "error";

/** One staged cleanup candidate (mirrors basket_list JSON). */
export interface BasketEntry {
  nodeId: number;
  path: string;
  name: string;
  logical: number;
  onDisk: number;
  tier: number;
  cat: number;
}

/** One dry-run line (mirrors dry_run_basket JSON). */
export interface DryRunEntry {
  nodeId: number;
  path: string;
  logicalScan: number;
  sizeNow: number | null;
  tier: number;
  cat: number;
  inUse: boolean;
  probeError: string | null;
  status: "ok" | "changed" | "missing";
}

/** Result of an executed action (mirrors execute_* JSON). */
export interface ExecuteReport {
  executed: { nodeId: number; path: string }[];
  dropped: { nodeId: number; path: string; reason: string }[];
  failed: { nodeId: number; path: string; error: string }[];
  journaled: boolean;
  count: number;
}

/** Result of undo_last (mirrors undo JSON). */
export interface UndoReport {
  undoable: boolean;
  action?: string;
  restored?: string[];
  notFound?: string[];
  conflicts?: string[];
  reason?: string;
  error?: string;
}

export interface Progress {
  files: number;
  dirs: number;
  denied: number;
  elapsedMs: number;
}

interface ByteMapState {
  folder: string;
  phase: Phase;
  progress: Progress | null;
  error: string | null;
  rootId: number | null;
  view: View | null;
  /** Sunburst layout for the same node as `view` (null = not loaded). */
  orbit: Orbit | null;
  selectedId: number | null;
  /** Name of the currently open node (for the breadcrumb's last crumb). */
  tailName: string | null;
  /** Category ids filtered out of views (dimmed, Phase 4). */
  hiddenCats: number[];
  /** Tier ids filtered out of views (dimmed, Phase 4). */
  hiddenTiers: number[];
  // --- Action Hub (Phase 5) ---
  basket: BasketEntry[];
  /** Human-readable notice from the last basket add (e.g. Documents excluded). */
  basketNotice: string | null;
  /** Dry-run report for the basket (null = not computed yet). */
  dryRun: DryRunEntry[] | null;
  /** Result of the last executed action. */
  lastReport: ExecuteReport | null;
  /** Result of the last undo. */
  lastUndo: UndoReport | null;
  // commands
  setFolder: (folder: string) => void;
  startScan: () => Promise<void>;
  cancelScan: () => Promise<void>;
  openNode: (nodeId: number) => Promise<void>;
  select: (id: number) => void;
  toggleCat: (cat: number) => void;
  toggleTier: (tier: number) => void;
  refreshBasket: () => Promise<void>;
  addToBasket: (ids: number[], includeDocuments: boolean) => Promise<void>;
  removeFromBasket: (nodeId: number) => Promise<void>;
  clearBasket: () => Promise<void>;
  runDryRun: () => Promise<void>;
  executeAction: (mode: "recycle" | "permanent" | "archive", confirmation?: string) => Promise<void>;
  undoLast: () => Promise<void>;
}

export const useStore = create<ByteMapState>((set, get) => ({
  folder: "",
  phase: "idle",
  progress: null,
  error: null,
  rootId: null,
  view: null,
  orbit: null,
  selectedId: null,
  tailName: null,
  hiddenCats: [],
  hiddenTiers: [],
  basket: [],
  basketNotice: null,
  dryRun: null,
  lastReport: null,
  lastUndo: null,

  setFolder: (folder) => set({ folder }),

  startScan: async () => {
    const root = get().folder;
    if (!root.trim()) {
      set({ error: "enter a folder to scan" });
      return;
    }
    set({
      phase: "scanning", progress: null, error: null, rootId: null, view: null, orbit: null,
      tailName: null,
      // New scan invalidates node ids: Rust empties the basket; mirror here.
      basket: [], basketNotice: null, dryRun: null, lastReport: null, lastUndo: null,
    });
    const channel = new Channel<string>();
    channel.onmessage = (msg) => {
      const event = JSON.parse(msg) as
        | { type: "progress" } & Progress
        | { type: "complete"; rootId: number | null } & Progress
        | { type: "cancelled" }
        | { type: "error"; message: string };
      switch (event.type) {
        case "progress":
          set({
            progress: {
              files: event.files,
              dirs: event.dirs,
              denied: event.denied,
              elapsedMs: event.elapsedMs,
            },
          });
          break;
        case "complete": {
          const rootId = event.rootId ?? null;
          set({ phase: "done", progress: null, rootId, tailName: "root" });
          if (rootId !== null) {
            void get().openNode(rootId);
          }
          break;
        }
        case "cancelled":
          set({ phase: "idle", progress: null });
          break;
        case "error":
          set({ phase: "error", error: event.message, progress: null });
          break;
      }
    };
    try {
      await invoke("start_scan", { root, onEvent: channel });
    } catch (e) {
      set({ phase: "error", error: String(e), progress: null });
    }
  },

  cancelScan: async () => {
    try {
      await invoke("cancel_scan");
    } catch (e) {
      set({ error: String(e) });
    }
  },

  openNode: async (nodeId) => {
    try {
      // The name of the node being opened: look it up in the current view
      // (items or ancestors); fall back to null => breadcrumb shows "root".
      const cur = get().view;
      const item = cur?.items.find((it) => it.id === nodeId && !it.isFolded);
      const anc = cur?.ancestors.find((a) => a.id === nodeId);
      const tailName = item?.name ?? anc?.name ?? get().tailName ?? null;
      // Responses with Raw bodies arrive as ArrayBuffer.
      const [buf, obuf] = (await Promise.all([
        invoke("node_view", { nodeId, limit: 200 }),
        invoke("node_orbit", { nodeId, rings: 4, maxArcs: 4000 }),
      ])) as [ArrayBuffer, ArrayBuffer];
      set({ view: parseView(buf), orbit: parseOrbit(obuf), selectedId: nodeId, tailName });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  select: (id) => set({ selectedId: id }),

  toggleCat: (cat) =>
    set((s) => ({
      hiddenCats: s.hiddenCats.includes(cat)
        ? s.hiddenCats.filter((c) => c !== cat)
        : [...s.hiddenCats, cat],
    })),

  toggleTier: (tier) =>
    set((s) => ({
      hiddenTiers: s.hiddenTiers.includes(tier)
        ? s.hiddenTiers.filter((t) => t !== tier)
        : [...s.hiddenTiers, tier],
    })),

  refreshBasket: async () => {
    try {
      const res = await invoke<{ items: BasketEntry[]; count: number }>("basket_list");
      set({ basket: res.items });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  addToBasket: async (ids, includeDocuments) => {
    if (ids.length === 0) return;
    try {
      const res = await invoke<{
        added: number[];
        refused: { nodeId: number; reason: string }[];
        count: number;
      }>("basket_add", { nodeIds: ids, includeDocuments });
      // Documents-excluded notice on bulk adds (brief line 24).
      const docExcluded = res.refused.filter(
        (r) => r.reason === "documents-default-excluded",
      ).length;
      const other = res.refused.filter((r) => r.reason !== "documents-default-excluded");
      set({
        basketNotice:
          docExcluded > 0
            ? `${docExcluded} Documents item${docExcluded === 1 ? "" : "s"} excluded from the bulk add (default; re-add individually or with "include Documents").`
            : other.length > 0
              ? `${other.length} item${other.length === 1 ? "" : "s"} refused (${other.map((r) => r.reason).join(", ")}).`
              : null,
      });
      await get().refreshBasket();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  removeFromBasket: async (nodeId) => {
    try {
      await invoke("basket_remove", { nodeId });
      set((s) => ({ basket: s.basket.filter((b) => b.nodeId !== nodeId) }));
    } catch (e) {
      set({ error: String(e) });
    }
  },

  clearBasket: async () => {
    try {
      await invoke("basket_clear");
      set({ basket: [], dryRun: null, basketNotice: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  runDryRun: async () => {
    try {
      const res = await invoke<{ entries: DryRunEntry[] }>("dry_run_basket");
      set({ dryRun: res.entries });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  executeAction: async (mode, confirmation) => {
    try {
      const res = await invoke<ExecuteReport>(`execute_${mode}`, confirmation === undefined ? {} : { confirmation });
      set({ lastReport: res, dryRun: null });
      await get().refreshBasket();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  undoLast: async () => {
    try {
      const res = await invoke<UndoReport>("undo_last");
      set({ lastUndo: res });
      await get().refreshBasket();
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));

export function isFolded(item: ViewItem): boolean {
  return item.isFolded;
}
