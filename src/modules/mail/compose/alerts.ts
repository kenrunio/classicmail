// Outlook 2007 New Mail Desktop Alert: the main window calls startDesktopAlerts() once; on each
// `mail.newMail` it opens (or feeds) the translucent "desktop-alert" window at the bottom right,
// through a queue that loses no batch while the window opens or closes (alertQueue.ts).
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { primaryMonitor, currentMonitor } from "@tauri-apps/api/window";
import { emitTo, listen } from "@tauri-apps/api/event";
import { onCmEvent } from "../../../api/events";
import { rpc } from "../../../api/rpc";
import type { NewMailEvent } from "../../../api/mailCompose";
import { AlertQueue } from "./alertQueue";

export const ALERT_LABEL = "desktop-alert";
export const ALERT_EVENT = "cm-desktop-alert";
export const ALERT_SETTING = "mail.desktopAlert";
/** from the alert window to the main window: it listens now / it is about to close */
export const ALERT_READY = "cm-desktop-alert-ready";
export const ALERT_CLOSING = "cm-desktop-alert-closing";
export const ALERT_W = 330;
export const ALERT_H = 86;

/** Keep the URL small: at most 20 messages, short previews. */
export function trimAlert(e: NewMailEvent): NewMailEvent {
  return {
    accountId: e.accountId,
    messages: (e.messages ?? []).slice(0, 20).map((m) => ({
      id: m.id,
      from: String(m.from ?? "").slice(0, 120),
      subject: String(m.subject ?? "").slice(0, 200),
      preview: String(m.preview ?? "")
        .replace(/\s+/g, " ")
        .slice(0, 220),
    })),
  };
}

/** Opens the alert window showing `data`; resolves once it exists. */
async function openAlertWindow(data: NewMailEvent) {
  const mon = (await primaryMonitor().catch(() => null)) ?? (await currentMonitor().catch(() => null));
  let x = 100;
  let y = 100;
  if (mon) {
    const s = mon.scaleFactor || 1;
    const wa = mon.workArea ?? { position: mon.position, size: mon.size };
    x = Math.round((wa.position.x + wa.size.width) / s - ALERT_W - 12);
    y = Math.round((wa.position.y + wa.size.height) / s - ALERT_H - 12);
  }
  const qs = new URLSearchParams({ window: "desktop-alert", data: JSON.stringify(data) });
  const w = new WebviewWindow(ALERT_LABEL, {
    url: `index.html?${qs.toString()}`,
    title: "New Mail Desktop Alert",
    width: ALERT_W,
    height: ALERT_H,
    x,
    y,
    decorations: false,
    transparent: true,
    shadow: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    resizable: false,
    focus: false,
    visible: true,
  });
  await new Promise<void>((resolve, reject) => {
    // in case "created" came before the listener: carry on (had it failed, the next batch finds
    // no window and opens one)
    setTimeout(resolve, 3000);
    w.once("tauri://created", () => resolve()).catch(reject);
    w.once("tauri://error", (e) => reject(new Error(String(e.payload)))).catch(reject);
  });
}

const queue = new AlertQueue<NewMailEvent>({
  exists: () => WebviewWindow.getByLabel(ALERT_LABEL).then((w) => !!w),
  create: openAlertWindow,
  send: (data) => emitTo(ALERT_LABEL, ALERT_EVENT, data),
  log: (message) => void rpc("app.log", { level: "error", message }).catch(() => {}),
});

export async function showDesktopAlert(e: NewMailEvent) {
  const data = trimAlert(e);
  if (data.messages.length) await queue.show(data);
}

/** Start listening for new mail; returns the stop function. Respects settings `mail.desktopAlert`. */
export function startDesktopAlerts(): () => void {
  let enabled = true;
  const load = () =>
    rpc<boolean | null>("settings.get", { key: ALERT_SETTING })
      .then((v) => {
        enabled = v !== false;
      })
      .catch(() => {});
  load();
  const ready = listen(ALERT_READY, () => void queue.onReady());
  const closing = listen(ALERT_CLOSING, () => void queue.onClosing());
  const off = onCmEvent((e) => {
    if (e.kind === "settings.changed" && (e as { key?: string }).key === ALERT_SETTING) load();
    if (e.kind === "mail.newMail" && enabled) showDesktopAlert(e as unknown as NewMailEvent).catch((err) => console.warn("desktop alert", err));
  });
  return () => {
    off();
    for (const un of [ready, closing]) un.then((f) => f()).catch(() => {});
  };
}
