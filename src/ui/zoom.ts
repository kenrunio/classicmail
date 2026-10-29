// Message zoom (new Outlook's Ctrl + mouse wheel): scales message bodies — the Reading Pane, an
// opened message, the archive viewer and the body of a message being written — not the rest of
// the window. One level for all windows, kept in the `mail.bodyZoom` setting. Elements that
// zoom are marked `data-zoom`; Ctrl+wheel over them (or inside a message body frame), Ctrl+= /
// Ctrl+- / Ctrl+0 and View › Increase / Decrease Font Size change the level.
import { useSettings, setSetting } from "../api/settings";
import { queryClient } from "../api/query";

export const ZOOM_STEPS = [50, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/** The next step up (`dir` 1) or down (-1) from `cur`; 0 resets to 100%. */
export function stepZoom(cur: number, dir: 1 | -1 | 0): number {
  if (dir === 0) return 100;
  if (dir > 0) return ZOOM_STEPS.find((s) => s > cur) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1];
  return [...ZOOM_STEPS].reverse().find((s) => s < cur) ?? ZOOM_STEPS[0];
}

export function useBodyZoom(): number {
  return useSettings().values["mail.bodyZoom"];
}

export function currentZoom(): number {
  const v = queryClient.getQueryData<Record<string, unknown>>(["settings", "all"])?.["mail.bodyZoom"];
  return typeof v === "number" && v > 0 ? v : 100;
}

let badgeTimer: ReturnType<typeof setTimeout> | undefined;
/** "125%" in a small badge for a moment, like new Outlook. */
function showBadge(pct: number) {
  let el = document.querySelector<HTMLDivElement>(".cm-zoom-badge");
  if (!el) {
    el = document.createElement("div");
    el.className = "cm-zoom-badge";
    document.body.appendChild(el);
  }
  el.textContent = `${pct}%`;
  el.classList.add("show");
  clearTimeout(badgeTimer);
  badgeTimer = setTimeout(() => el?.classList.remove("show"), 1200);
}

/** Zoom in (1), out (-1) or back to 100% (0). */
export function changeZoom(dir: 1 | -1 | 0) {
  const next = stepZoom(currentZoom(), dir);
  showBadge(next);
  if (next !== currentZoom()) setSetting("mail.bodyZoom", next).catch(() => {});
}

// trackpads send many small Ctrl+wheel deltas
let acc = 0;
let accReset: ReturnType<typeof setTimeout> | undefined;
function onWheel(e: WheelEvent) {
  if (!e.ctrlKey) return;
  // never let the WebView zoom the whole window
  e.preventDefault();
  const zoomable = (e.target as Element | null)?.closest?.("[data-zoom]") || (e.target as Node | null)?.ownerDocument !== document;
  if (!zoomable) return;
  const dy = e.deltaMode === 1 ? e.deltaY * 33 : e.deltaY;
  if (Math.abs(dy) >= 50) {
    // a mouse-wheel notch (100 or 125 depending on display scaling): one step
    acc = 0;
    changeZoom(dy < 0 ? 1 : -1);
    return;
  }
  // trackpad: small deltas add up to a step
  acc += dy;
  clearTimeout(accReset);
  accReset = setTimeout(() => (acc = 0), 300);
  if (Math.abs(acc) >= 100) {
    changeZoom(acc < 0 ? 1 : -1);
    acc = 0;
  }
}

function onKey(e: KeyboardEvent) {
  if (!e.ctrlKey || e.altKey || !document.querySelector("[data-zoom]")) return;
  const k = e.key;
  const dir = k === "=" || k === "+" ? 1 : k === "-" || k === "_" ? -1 : k === "0" && !e.shiftKey ? 0 : null;
  if (dir === null) return;
  e.preventDefault();
  changeZoom(dir as 1 | -1 | 0);
}

/** Once per window (src/main.tsx). Message body frames call `wireFrame` for their own document. */
export function installZoom() {
  window.addEventListener("wheel", onWheel, { passive: false, capture: true });
  window.addEventListener("keydown", onKey, { capture: true });
}

/** Ctrl+wheel inside a message body iframe (its Ctrl keys already reach the window: MessageBody
 *  forwards them, so Ctrl+= / - / 0 work there without a listener of their own). */
export function wireFrame(d: Document) {
  d.addEventListener("wheel", onWheel, { passive: false });
}
