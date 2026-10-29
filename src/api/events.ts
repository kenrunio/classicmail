// Back-end events (`cm-event`, payload { kind, ...data }) → subscribers + query invalidation.
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { queryClient } from "./query";

export type CmEvent = { kind: string; [k: string]: unknown };
type Handler = (e: CmEvent) => void;

const handlers = new Set<Handler>();
let started = false;

/** Which query-key prefixes to invalidate for each event kind. */
const INVALIDATE: Record<string, string[][]> = {
  "mail.changed": [["mail"]],
  "calendar.changed": [["calendar"], ["todo"]],
  "contacts.changed": [["contacts"]],
  "tasks.changed": [["tasks"], ["todo"]],
  "rules.changed": [["rules"]],
  "accounts.changed": [["accounts"], ["mail"], ["calendar"], ["contacts"], ["tasks"], ["todo"]],
  "settings.changed": [["settings"]],
  "ai.changed": [["ai"]],
  "archive.changed": [["archive"]],
};

// A sync batch or a multi-message action sends many events within milliseconds; each invalidation
// would restart the same refetches (mail.listFolders scans every message). Invalidations arriving
// within BATCH_MS are merged — one refetch per key — and none waits longer than MAX_WAIT_MS.
const BATCH_MS = 200;
const MAX_WAIT_MS = 1000;
const pending = new Map<string, string[]>();
let flushTimer: ReturnType<typeof setTimeout> | undefined;
let firstPendingAt = 0;

function flush() {
  flushTimer = undefined;
  const keys = [...pending.values()];
  pending.clear();
  for (const key of keys) queryClient.invalidateQueries({ queryKey: key });
}

/** Invalidate `key` soon (batched with the other invalidations of this burst). */
export function invalidateSoon(key: string[]) {
  const now = Date.now();
  if (!pending.size) firstPendingAt = now;
  pending.set(JSON.stringify(key), key);
  clearTimeout(flushTimer);
  flushTimer = setTimeout(flush, Math.max(0, Math.min(BATCH_MS, firstPendingAt + MAX_WAIT_MS - now)));
}

export function startEvents() {
  if (started) return;
  started = true;
  listen<CmEvent>("cm-event", ({ payload }) => {
    for (const key of INVALIDATE[payload.kind] ?? []) invalidateSoon(key);
    // mail.changed also moves flagged-mail tasks in the To-Do Bar
    if (payload.kind === "mail.changed") invalidateSoon(["todo"]);
    for (const h of handlers) h(payload);
  });
}

export function onCmEvent(h: Handler): () => void {
  handlers.add(h);
  return () => handlers.delete(h);
}

/** Subscribe to one event kind (or all with "*") for the component's lifetime. */
export function useCmEvent(kind: string, handler: Handler) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => onCmEvent((e) => (kind === "*" || e.kind === kind) && ref.current(e)), [kind]);
}
