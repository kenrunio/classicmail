// App-wide Undo / Redo (Ctrl+Z / Ctrl+Y). The history lives in the back end (undo.*), shared by
// every window; this file records actions (`undoable`), keeps the Edit menu labels current and
// handles the keys. Not persisted: the history is cleared when ClassicMail closes.
import { useSyncExternalStore } from "react";
import { rpc } from "./rpc";
import { onCmEvent } from "./events";
import { keyOf } from "../ui/keys";

export interface UndoState {
  undoLabel: string | null;
  redoLabel: string | null;
}

let state: UndoState = { undoLabel: null, redoLabel: null };
let loaded = false;
const subs = new Set<() => void>();
const messageSubs = new Set<(m: string) => void>();

function set(s: UndoState) {
  if (s.undoLabel === state.undoLabel && s.redoLabel === state.redoLabel) return;
  state = s;
  for (const f of subs) f();
}

function load() {
  if (loaded) return;
  loaded = true;
  rpc<{ undoLabel?: string | null; redoLabel?: string | null }>("undo.peek", {})
    .then((p) => set({ undoLabel: p.undoLabel ?? null, redoLabel: p.redoLabel ?? null }))
    .catch(() => {});
  onCmEvent((e) => {
    if (e.kind !== "undo.changed") return;
    set({ undoLabel: (e.undoLabel as string | null) ?? null, redoLabel: (e.redoLabel as string | null) ?? null });
    if (typeof e.message === "string" && e.message) for (const f of messageSubs) f(e.message);
  });
}

/** Current labels (for menus built on demand). */
export function undoState(): UndoState {
  load();
  return state;
}

export function useUndoState(): UndoState {
  load();
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => state,
  );
}

/** "Undid: …" / "Redid: …" / "Couldn't undo …" from any window. */
export function onUndoMessage(f: (m: string) => void): () => void {
  load();
  messageSubs.add(f);
  return () => messageSubs.delete(f);
}

/** Perform `method(params)` and remember how to undo it (the back end works out the inverse from
 * the state before the action; actions that can't be undone just run). */
export function undoable<T = unknown>(label: string, method: string, params: object = {}): Promise<T> {
  return rpc<T>("undo.run", { label, method, params });
}

/** Several actions as ONE undo step; results in order. */
export function undoableMany<T = unknown>(label: string, calls: { method: string; params: object }[]): Promise<T[]> {
  if (!calls.length) return Promise.resolve([]);
  return rpc<T[]>("undo.runMany", { label, calls });
}

/** With a label: recorded in the Undo history; without: a plain rpc. */
export function maybeUndoable<T = unknown>(label: string | undefined, method: string, params: object): Promise<T> {
  return label ? undoable<T>(label, method, params) : rpc<T>(method, params);
}

/** "1 item" / "3 items" */
export const items = (n: number) => (n === 1 ? "1 item" : `${n.toLocaleString()} items`);

let failureHandler: ((m: string) => void) | null = null;
/** How this window shows a failed undo when it has no status bar (item windows). */
export function setUndoFailureHandler(f: ((m: string) => void) | null) {
  failureHandler = f;
}

async function step(which: "undo" | "redo") {
  try {
    await rpc(`undo.${which}`, {});
  } catch (e) {
    failureHandler?.((e as Error).message ?? String(e));
  }
}

export const undo = () => step("undo");
export const redo = () => step("redo");

/** Ctrl+Z / Ctrl+Y (and Ctrl+Shift+Z) for this window, except while typing in a text field —
 * there the browser's own text undo works (compose editor, search boxes, forms). */
export function installUndoKeys() {
  load();
  window.addEventListener("keydown", (e) => {
    if (!e.ctrlKey || e.altKey || e.metaKey || e.defaultPrevented) return;
    const k = keyOf(e);
    const isUndo = k === "z" && !e.shiftKey;
    const isRedo = (k === "y" && !e.shiftKey) || (k === "z" && e.shiftKey);
    if (!isUndo && !isRedo) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
    if (document.querySelector(".cm-dialog-overlay")) return; // a modal dialog owns the keyboard
    e.preventDefault();
    void (isUndo ? undo() : redo());
  });
}
