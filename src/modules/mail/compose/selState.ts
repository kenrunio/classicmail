// What the ribbon shows about the selection on the compose page: the Font group's values, the
// pressed state of alignment / list buttons and whether the Table group is on. Worked out once per
// selection change (at most once a frame) into a small store that only the buttons showing it
// read, so moving the caret redraws those buttons and not the whole window.
import { create } from "zustand";
import { combine, domFormats, type FmtState } from "./formatState";
import { selKey, tableSelection } from "./pageTables";

/** Browser command states the ribbon shows as pressed buttons. */
export const COMMANDS = ["justifyLeft", "justifyCenter", "justifyRight", "justifyFull", "insertUnorderedList", "insertOrderedList"] as const;
export type Command = (typeof COMMANDS)[number];

export interface SelState {
  /** the formatting at the caret / over the selection; null before the page had a selection */
  fmt: FmtState | null;
  /** which COMMANDS are on there */
  on: Partial<Record<Command, boolean>>;
  /** the selection in a table: "" none, else "cell" / "cells" (+ " merged" for a cell that can be split) */
  table: string;
}

const EMPTY: SelState = { fmt: null, on: {}, table: "" };
export const useSelState = create<SelState>(() => EMPTY);

/** Forget the selection (the page was reloaded). */
export const resetSelState = () => useSelState.setState(EMPTY, true);

/** The ribbon's view of `range` on `page`. */
export function readSelState(range: Range, page: HTMLElement): SelState {
  const on: SelState["on"] = {};
  if (typeof document.queryCommandState === "function") for (const c of COMMANDS) if (document.queryCommandState(c)) on[c] = true;
  return { fmt: combine(domFormats(range, page)), on, table: selKey(tableSelection(range, page)) };
}

/** Follow the selection on the page `getPage()` gives: each change (and each edit, which can change
 *  the formatting without moving the caret) hands the range to `onRange` and, on the next frame,
 *  updates the store when what it shows changed. Returns the function that stops it. */
export function trackSelection(getPage: () => HTMLElement | null, onRange: (r: Range) => void): () => void {
  let frame = 0;
  const update = () => {
    frame = 0;
    const sel = window.getSelection();
    const page = getPage();
    if (!page || !sel?.rangeCount || !page.contains(sel.anchorNode)) return;
    const next = readSelState(sel.getRangeAt(0), page);
    if (JSON.stringify(next) !== JSON.stringify(useSelState.getState())) useSelState.setState(next, true);
  };
  const onChange = () => {
    const sel = window.getSelection();
    const page = getPage();
    if (!page || !sel?.rangeCount || !page.contains(sel.anchorNode)) return;
    onRange(sel.getRangeAt(0).cloneRange());
    if (!frame) frame = requestAnimationFrame(update);
  };
  document.addEventListener("selectionchange", onChange);
  document.addEventListener("input", onChange);
  return () => {
    if (frame) cancelAnimationFrame(frame);
    document.removeEventListener("selectionchange", onChange);
    document.removeEventListener("input", onChange);
  };
}
