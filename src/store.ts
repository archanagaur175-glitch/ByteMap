// Application store (zustand). The frontend holds only views; the tree
// lives in Rust. Item identity = node id, never a path string.

import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import { parseView } from "./parseView";
import { parseDiff } from "./diff";
import { parseOrbit } from "./orbit";
import { parseStrata } from "./strata";
import { refreshColors } from "./colors";
import { applyTheme, loadTheme, saveTheme } from "./theme";
import { exportCsv, formatExportNotice } from "./export";
import type { View, ViewItem } from "./parseView";
import type { Diff } from "./diff";
import type { Orbit } from "./orbit";
import type { Strata } from "./strata";
import type { Theme } from "./theme";

export type Phase = "idle" | "scanning" | "done" | "error";

/** Visual models the main pane can show (Phase 7). */
export type ViewMode = "orbit" | "strata";

// Bootstrap (Phase 7): apply the stored theme before the first paint, make
// the color cache match it, then ask Rust which window backdrop the OS
// accepted. The UI only lets the window be transparent when a real material
// is active.
applyTheme(loadTheme());
refreshColors();
try {
  void invoke<string>("backdrop_state")
    .then((material) => {
      if (typeof document !== "undefined") {
        document.documentElement.dataset.backdrop = material;
      }
    })
    .catch(() => {
      // No backdrop information: styles.css keeps the opaque background.
    });
} catch {
  // Not inside the Tauri webview (tests, plain browser): nothing to ask.
}

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

/** One saved snapshot row (mirrors snapshot_list JSON). */
export interface SnapshotEntry {
  name: string;
  bytes: number;
  createdUnix: number;
  rootName: string;
  tier: string;
  scanMs: number;
  nodes: number;
  files: number;
  dirs: number;
  denied: number;
  totalLogical: number;
  totalOnDisk: number;
  /** Carries a USN checkpoint: the Refresh button applies the journal delta. */
  refreshable: boolean;
  /** Listed but unreadable (still deletable). */
  error?: string;
}

/** Delta counts returned by snapshot_refresh (attached to the load payload). */
export interface RefreshReport {
  facts: number;
  added: number;
  removed: number;
  updated: number;
  unresolved: number;
  nodesBefore: number;
  nodesAfter: number;
}

/** Result of loading a snapshot (mirrors snapshot_load JSON). */
export interface LoadedSnapshot {
  rootId: number | null;
  name: string;
  rootName: string;
  createdUnix: number;
  tier: string;
  scanMs: number;
  counts: {
    nodes: number;
    files: number;
    dirs: number;
    denied: number;
    totalLogical: number;
    totalOnDisk: number;
  };
}

/**
 * Session state after a snapshot becomes the live tree. Loading and
 * refreshing share it so both paths clear the same stale views.
 */
function snapshotLoadedState(
  res: LoadedSnapshot,
  notice: string,
): Partial<ByteMapState> {
  return {
    phase: res.rootId === null ? "idle" : "done",
    rootId: res.rootId,
    view: null,
    orbit: null,
    strata: null,
    selectedId: null,
    tailName: null,
    // Snapshot node ids are snapshot-local; Rust cleared the basket.
    basket: [],
    basketNotice: null,
    dryRun: null,
    lastReport: null,
    lastUndo: null,
    loadedSnapshot: {
      name: res.name,
      rootName: res.rootName,
      createdUnix: res.createdUnix,
    },
    snapshotNotice: notice,
    snapshotsOpen: false,
    exportNotice: null,
    // The folder input shows the snapshot's root, so a fresh scan of the
    // same place is one click away.
    folder: res.rootName,
    error: null,
  };
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
  /** Strata (layered treemap) layout for the same node (null = not loaded). */
  strata: Strata | null;
  /** Which visual model the main pane shows. */
  viewMode: ViewMode;
  /** Active color theme; dark is the default and the choice persists locally. */
  theme: Theme;
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
  // --- Snapshots (Phase 8) ---
  /** Saved snapshots, newest first (header only; no body decompressed). */
  snapshots: SnapshotEntry[];
  /** Whether the Snapshots panel is open. */
  snapshotsOpen: boolean;
  /** Snapshot currently loaded into the session (null = live scan data). */
  loadedSnapshot: { name: string; rootName: string; createdUnix: number } | null;
  /** Last save/load/delete notice. */
  snapshotNotice: string | null;
  /** Snapshot diff being shown (null = the normal list). */
  diff: Diff | null;
  /** Names of the compared snapshots (old, new), for the diff header. */
  diffPair: { old: string; new: string } | null;
  /** True while two snapshots are being loaded and compared. */
  diffLoading: boolean;
  // --- CSV export (P8.T4) ---
  /** Last export notice (row count + file path), null = none yet. */
  exportNotice: string | null;
  // commands
  setFolder: (folder: string) => void;
  startScan: () => Promise<void>;
  cancelScan: () => Promise<void>;
  openNode: (nodeId: number) => Promise<void>;
  select: (id: number) => void;
  setViewMode: (mode: ViewMode) => void;
  setTheme: (theme: Theme) => void;
  toggleCat: (cat: number) => void;
  toggleTier: (tier: number) => void;
  refreshBasket: () => Promise<void>;
  addToBasket: (ids: number[], includeDocuments: boolean) => Promise<void>;
  removeFromBasket: (nodeId: number) => Promise<void>;
  clearBasket: () => Promise<void>;
  runDryRun: () => Promise<void>;
  executeAction: (mode: "recycle" | "permanent" | "archive", confirmation?: string) => Promise<void>;
  undoLast: () => Promise<void>;
  openSnapshots: () => void;
  closeSnapshots: () => void;
  refreshSnapshots: () => Promise<void>;
  saveSnapshot: () => Promise<void>;
  loadSnapshot: (name: string) => Promise<void>;
  refreshSnapshot: (name: string) => Promise<void>;
  deleteSnapshot: (name: string) => Promise<void>;
  diffSnapshots: (oldName: string, newName: string) => Promise<void>;
  closeDiff: () => void;
  exportViewCsv: () => Promise<void>;
  exportBasketCsv: () => Promise<void>;
}

export const useStore = create<ByteMapState>((set, get) => ({
  folder: "",
  phase: "idle",
  progress: null,
  error: null,
  rootId: null,
  view: null,
  orbit: null,
  strata: null,
  viewMode: "orbit",
  theme: loadTheme(),
  selectedId: null,
  tailName: null,
  hiddenCats: [],
  hiddenTiers: [],
  basket: [],
  basketNotice: null,
  dryRun: null,
  lastReport: null,
  lastUndo: null,
  snapshots: [],
  snapshotsOpen: false,
  loadedSnapshot: null,
  snapshotNotice: null,
  diff: null,
  diffPair: null,
  diffLoading: false,
  exportNotice: null,

  setFolder: (folder) => set({ folder }),

  startScan: async () => {
    const root = get().folder;
    if (!root.trim()) {
      set({ error: "enter a folder to scan" });
      return;
    }
    set({
      phase: "scanning", progress: null, error: null, rootId: null, view: null, orbit: null,
      strata: null, tailName: null,
      // New scan invalidates node ids: Rust empties the basket; mirror here.
      basket: [], basketNotice: null, dryRun: null, lastReport: null, lastUndo: null,
      // A live scan replaces snapshot mode (Rust clears its own flag too).
      loadedSnapshot: null, snapshotNotice: null, exportNotice: null,
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
      // Responses with Raw bodies arrive as ArrayBuffer. All three layouts
      // share the same tree read, so one round trip per model is fine and
      // switching views never needs another invoke.
      const [buf, obuf, sbuf] = (await Promise.all([
        invoke("node_view", { nodeId, limit: 200 }),
        invoke("node_orbit", { nodeId, rings: 4, maxArcs: 4000 }),
        invoke("node_strata", { nodeId, depth: 3, maxRects: 4000 }),
      ])) as [ArrayBuffer, ArrayBuffer, ArrayBuffer];
      set({
        view: parseView(buf),
        orbit: parseOrbit(obuf),
        strata: parseStrata(sbuf),
        selectedId: nodeId,
        tailName,
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  select: (id) => set({ selectedId: id }),

  setViewMode: (mode) => set({ viewMode: mode }),

  setTheme: (theme) => {
    applyTheme(theme);
    // CSS tokens are live now; refresh the cached copies the views read
    // (getComputedStyle forces the recalc, so the new values are available).
    refreshColors();
    saveTheme(theme);
    set({ theme });
    // Keep the OS backdrop in step: Mica is dark or light, never both.
    try {
      void invoke<string>("set_backdrop_theme", { dark: theme === "dark" })
        .then((material) => {
          if (typeof document !== "undefined") {
            document.documentElement.dataset.backdrop = material;
          }
        })
        .catch(() => {});
    } catch {
      // Not inside the Tauri webview: the CSS theme alone is enough.
    }
  },

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

  openSnapshots: () => {
    set({ snapshotsOpen: true });
    void get().refreshSnapshots();
  },

  closeSnapshots: () => set({ snapshotsOpen: false }),

  refreshSnapshots: async () => {
    try {
      const entries = await invoke<SnapshotEntry[]>("snapshot_list");
      set({ snapshots: entries });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  saveSnapshot: async () => {
    try {
      const entry = await invoke<SnapshotEntry>("snapshot_save");
      set({
        snapshotNotice: `Saved ${entry.name} · ${entry.nodes.toLocaleString()} entries`,
      });
      await get().refreshSnapshots();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  loadSnapshot: async (name) => {
    try {
      const res = await invoke<LoadedSnapshot>("snapshot_load", { name });
      set(snapshotLoadedState(res, `Loaded ${res.name}`));
      if (res.rootId !== null) {
        await get().openNode(res.rootId);
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },

  refreshSnapshot: async (name) => {
    try {
      const res = await invoke<LoadedSnapshot & { refresh?: RefreshReport }>(
        "snapshot_refresh",
        { name },
      );
      const r = res.refresh;
      const notice = r
        ? `Refreshed → ${res.name} · ${r.added} added, ${r.removed} removed, ${r.updated} updated` +
          (r.unresolved > 0 ? `, ${r.unresolved} unresolved` : "")
        : `Refreshed → ${res.name}`;
      set(snapshotLoadedState(res, notice));
      // The refresh saves a NEW snapshot, so the list gained a row.
      await get().refreshSnapshots();
      if (res.rootId !== null) {
        await get().openNode(res.rootId);
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },

  deleteSnapshot: async (name) => {
    try {
      await invoke("snapshot_delete", { name });
      set({ snapshotNotice: `Deleted ${name}` });
      await get().refreshSnapshots();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  diffSnapshots: async (oldName, newName) => {
    if (oldName === newName) {
      set({ error: "Pick two different snapshots to compare." });
      return;
    }
    set({ diffLoading: true, error: null });
    try {
      // Raw response bodies arrive as ArrayBuffer (same as node_view).
      const buf = await invoke<ArrayBuffer>("snapshot_diff", {
        old: oldName, new: newName, maxEntries: 500,
      });
      set({
        diff: parseDiff(buf),
        diffPair: { old: oldName, new: newName },
        diffLoading: false,
      });
    } catch (e) {
      set({ diffLoading: false, error: String(e) });
    }
  },

  closeDiff: () => set({ diff: null, diffPair: null }),

  exportViewCsv: async () => {
    const { view } = get();
    if (!view) {
      set({ error: "open a folder first, then export its items" });
      return;
    }
    try {
      // `requested` is the node the view belongs to — not the hovered row.
      const res = await exportCsv("view", view.requested);
      set({ exportNotice: formatExportNotice(res), error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  exportBasketCsv: async () => {
    try {
      const res = await exportCsv("basket");
      set({ exportNotice: formatExportNotice(res), error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));

export function isFolded(item: ViewItem): boolean {
  return item.isFolded;
}
