// Application store (zustand). The frontend holds only views; the tree
// lives in Rust. Item identity = node id, never a path string.

import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import type { View, ViewItem } from "./parseView";

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
  selectedId: number | null;
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
  selectedId: null,

  setFolder: (folder) => set({ folder }),

  startScan: async () => {
    const root = get().folder;
    if (!root.trim()) {
      set({ error: "enter a folder to scan" });
      return;
    }
    set({ phase: "scanning", progress: null, error: null, rootId: null, view: null });
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
          set({ phase: "done", progress: null, rootId });
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
      // Response with Raw body arrives as ArrayBuffer.
      const buf = (await invoke("node_view", { nodeId, limit: 200 })) as ArrayBuffer;
      set({ view: parseViewItem(buf), selectedId: nodeId });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  select: (id) => set({ selectedId: id }),
}));

// Imported here (not at top) to keep the store's tauri imports together.
import { parseView } from "./parseView";
function parseViewItem(buf: ArrayBuffer): View {
  return parseView(buf);
}

export function isFolded(item: ViewItem): boolean {
  return item.isFolded;
}
