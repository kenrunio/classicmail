// Main-window UI state shared across modules (persisted per machine in localStorage). A window
// opened with Open in New Window (`?window=explorer&module=calendar`) keeps its own copy, so it
// doesn't change which module and layout the main window comes back with.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { ModuleId } from "./types";
import type { SyncStatus } from "../api/types";

export type ReadingPane = "right" | "bottom" | "off";

/** This window is an extra main window (Navigation Pane › Open in New Window). */
export const isExplorer = new URLSearchParams(window.location.search).get("window") === "explorer";
const MODULE_IDS: ModuleId[] = ["mail", "calendar", "contacts", "tasks"];

interface ShellState {
  module: ModuleId;
  navWidth: number;
  navCollapsed: boolean;
  todoWidth: number;
  todoVisible: boolean;
  /** Outlook's "Minimized" To-Do Bar: a thin strip with an expand button */
  todoMinimized: boolean;
  readingPane: ReadingPane;
  setModule: (m: ModuleId) => void;
  set: (p: Partial<Omit<ShellState, "set" | "setModule">>) => void;
}

export const useShell = create<ShellState>()(
  persist(
    (set) => ({
      module: "mail",
      navWidth: 220,
      navCollapsed: false,
      todoWidth: 210,
      todoVisible: true,
      todoMinimized: false,
      readingPane: "right",
      setModule: (module) => set({ module }),
      set: (p) => set(p),
    }),
    { name: isExplorer ? "cm:shell:explorer" : "cm:shell" },
  ),
);

// an extra window opens on the module it was asked for (the saved state has loaded by now)
if (isExplorer) {
  const m = new URLSearchParams(window.location.search).get("module") as ModuleId | null;
  if (m && MODULE_IDS.includes(m)) useShell.setState({ module: m });
}

/** Transient status-bar state (not persisted). */
interface StatusState {
  /** left side, e.g. "Items: 1,234   Unread: 12" — set by the active module */
  left: string;
  sync: Record<number, SyncStatus>;
  setLeft: (s: string) => void;
  /** a short-lived message shown instead of `left` ("Undid: Move 3 items to Investors") */
  message: string;
  flash: (s: string, ms?: number) => void;
  setSync: (s: SyncStatus) => void;
}

let flashTimer: ReturnType<typeof setTimeout> | undefined;
export const useStatus = create<StatusState>((set) => ({
  left: "",
  sync: {},
  setLeft: (left) => set({ left }),
  message: "",
  flash: (message, ms = 4000) => {
    set({ message });
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => set({ message: "" }), ms);
  },
  setSync: (s) => set((st) => ({ sync: { ...st.sync, [s.accountId]: s } })),
}));
