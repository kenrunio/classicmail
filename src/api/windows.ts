// Opening separate windows (compose, message, appointment, contact, task, dialogs).
// Every window loads index.html?window=<kind>&<params>; src/main.tsx routes on `window`.
import { WebviewWindow, getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { rpc } from "./rpc";

export type WindowKind =
  | "main"
  | "explorer"
  | "compose"
  | "message"
  | "appointment"
  | "contact"
  | "task"
  | "reminders"
  | "desktop-alert"
  | "options"
  | "rules"
  | "accounts"
  | "address-book"
  | "recurrence";

export interface OpenWindowOptions {
  title?: string;
  width?: number;
  height?: number;
  /** Reuse/focus an existing window with this label instead of opening another. */
  label?: string;
  resizable?: boolean;
  alwaysOnTop?: boolean;
  decorations?: boolean;
  x?: number;
  y?: number;
  skipTaskbar?: boolean;
  focus?: boolean;
  modal?: boolean;
}

const DEFAULT_SIZES: Partial<Record<WindowKind, [number, number]>> = {
  // a second main window (Open in New Window), e.g. Calendar on another screen
  explorer: [1200, 800],
  // wide enough for the ribbon incl. Reply to One, Translate and AI Review
  compose: [1040, 700],
  message: [900, 700],
  appointment: [860, 640],
  contact: [860, 600],
  task: [760, 560],
  reminders: [430, 330],
  options: [560, 520],
  rules: [640, 520],
  accounts: [640, 460],
  "address-book": [640, 480],
  recurrence: [520, 420],
};

/** Windows still being created, by label: Tauri registers a window only once it exists, so two
 *  quick opens of the same label (e.g. the start-up reminder check and the reminder loop's first
 *  tick) would both create one, and the second would be left orphaned — on screen, but every call
 *  from it fails with "failed to acquire webview reference". */
const creating = new Map<string, Promise<WebviewWindow>>();

export async function openWindow(kind: WindowKind, params: Record<string, string | number | boolean | undefined> = {}, opts: OpenWindowOptions = {}) {
  const label = (opts.label ?? `${kind}-${Date.now()}-${Math.floor(Math.random() * 1e6)}`).replace(/[^a-zA-Z0-9\-_/:]/g, "_");
  const pending = creating.get(label);
  if (pending) {
    const w = await pending;
    await w.setFocus().catch(() => {});
    return w;
  }
  const p = openOrFocus(label, kind, params, opts);
  creating.set(label, p);
  try {
    return await p;
  } finally {
    creating.delete(label);
  }
}

async function openOrFocus(label: string, kind: WindowKind, params: Record<string, string | number | boolean | undefined>, opts: OpenWindowOptions) {
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.unminimize().catch(() => {});
    await existing.show().catch(() => {});
    await existing.setFocus();
    return existing;
  }
  const qs = new URLSearchParams({ window: kind });
  for (const [k, v] of Object.entries(params)) if (v !== undefined) qs.set(k, String(v));
  // a compose window: take the spare that is already loaded in the background (src-tauri/src/spare.rs)
  if (SPARE_KINDS.includes(kind) && !opts.label) {
    const spare = await rpc<string | null>("app.claimSpare", { kind, query: qs.toString(), title: opts.title ?? "ClassicMail" }).catch(() => null);
    const w = spare ? await WebviewWindow.getByLabel(spare) : null;
    if (w) return w;
  }
  const [w, h] = DEFAULT_SIZES[kind] ?? [800, 600];
  const win = new WebviewWindow(label, {
    url: `index.html?${qs.toString()}`,
    title: opts.title ?? "ClassicMail",
    width: opts.width ?? w,
    height: opts.height ?? h,
    resizable: opts.resizable ?? true,
    alwaysOnTop: opts.alwaysOnTop ?? false,
    decorations: opts.decorations ?? true,
    skipTaskbar: opts.skipTaskbar ?? false,
    // the page's own drag and drop works only without the native drop handler on Windows (as in
    // the main window's config and the spare windows); dropped files come in as page drops
    dragDropEnabled: false,
    focus: opts.focus ?? true,
    visible: !NORMAL_SIZE_KINDS.includes(kind),
    center: opts.x === undefined,
    x: opts.x,
    y: opts.y,
  });
  // settle once Tauri has the window (or refused it), so later opens find it by label
  await new Promise<void>((resolve) => {
    void win.once("tauri://created", () => resolve());
    void win.once("tauri://error", () => resolve());
    window.setTimeout(resolve, 10_000);
  });
  return win;
}

/** Kinds that open at their last normal size even when the last one was closed maximized: created
 *  hidden, they show themselves (showAtNormalSize) once loaded. */
const NORMAL_SIZE_KINDS: WindowKind[] = ["appointment"];

/** Show this window at its remembered normal size, never maximized (see NORMAL_SIZE_KINDS). */
export async function showAtNormalSize() {
  const w = getCurrentWebviewWindow();
  try {
    await rpc("app.restoreNormalSize", { label: w.label });
  } finally {
    await w.show();
    await w.setFocus();
  }
}

/** Kinds that keep a hidden, loaded spare window ready (opens at once instead of after ~1 s). */
export const SPARE_KINDS: WindowKind[] = ["compose", "message"];

/**
 * A spare window was claimed and has drawn its content: show it. Called by the window component
 * once its real content (or an error) is on the page; a no-op in a window that is already shown.
 */
export function markPainted() {
  window.dispatchEvent(new Event("cm-painted"));
}

/** Params of the current window (from its URL). */
export function windowParams(): URLSearchParams {
  return new URLSearchParams(window.location.search);
}

export function closeCurrentWindow() {
  return getCurrentWebviewWindow().close();
}

export function setWindowTitle(title: string) {
  document.title = title;
  return getCurrentWebviewWindow()
    .setTitle(title)
    .catch(() => {});
}
