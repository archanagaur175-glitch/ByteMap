// Application store (zustand). The frontend holds only views; the tree
// lives in Rust. Item identity = node id, never a path string.

import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import { parseView } from "./parseView";
import { parseOrbit } from "./orbit";
import type { View, ViewItem } from "./parseView";
import type { Orbit } from "./orbit";

export type Phase = "idle" | "scanning" | "done" | "error";

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
  // commands
  setFolder: (folder: string) => void;
  startScan: () => Promise<void>;
  cancelScan: () => Promise<void>;
  openNode: (nodeId: number) => Promise<void>;
  select: (id: number) => void;
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

  setFolder: (folder) => set({ folder }),

  startScan: async () => {
    const root = get().folder;
    if (!root.trim()) {
      set({ error: "enter a folder to scan" });
      return;
    }
    set({ phase: "scanning", progress: null, error: null, rootId: null, view: null, orbit: null, tailName: null });
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
}));

export function isFolded(item: ViewItem): boolean {
  return item.isFolded;
}
