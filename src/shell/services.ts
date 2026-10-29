// Main-window-only background behaviour: tray badge + unread tooltip, tray menu actions, and the
// module services (desktop alerts, reminders).
import { onCmEvent } from "../api/events";
import { errorForLog, rpc } from "../api/rpc";
import { mailApi } from "../api/mail";
import { queryClient } from "../api/query";
import { newMail } from "./newItems";
import { openWindow } from "../api/windows";
import { MODULE_SERVICES } from "./moduleServices";

/** Total unread in every account's Inbox → tray tooltip ("ClassicMail — 12 unread"). */
async function refreshUnread(newMailArrived: boolean) {
  try {
    // the folder list the mail module shows (refetched on mail.changed): no second scan of every
    // message. New mail always reads it fresh (it may arrive without a mail.changed).
    const boxes = await queryClient.fetchQuery({ queryKey: ["mail", "folders"], queryFn: mailApi.folders, staleTime: newMailArrived ? 0 : 5000 });
    const unread = boxes
      .flatMap((b) => b.folders)
      .filter((f) => f.role === "inbox")
      .reduce((n, f) => n + f.unread, 0);
    await rpc("app.setTrayState", { newMail: newMailArrived ? true : undefined, unread });
  } catch {
    /* mail not ready yet */
  }
}

/** Messages Word / Excel / other programs sent with ClassicMail as the mail client: one window each. */
let taking = false;
async function openMapiMessages() {
  if (taking) return;
  taking = true;
  try {
    const dirs = await rpc<string[]>("mail.mapi.take");
    for (const dir of dirs) await openWindow("compose", { mapi: dir }, { title: "Untitled - Message" });
  } catch (err) {
    rpc("app.log", { level: "error", message: `opening a message from another program failed: ${errorForLog(err)}` }).catch(() => {});
  } finally {
    taking = false;
  }
}

export function startDesktopServices(): () => void {
  const stops: (() => void)[] = [];
  let timer: number | undefined;
  let pendingNew = false;
  const schedule = (isNew: boolean) => {
    pendingNew = pendingNew || isNew;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      refreshUnread(pendingNew && !document.hasFocus());
      pendingNew = false;
    }, 800);
  };
  stops.push(
    onCmEvent((e) => {
      if (e.kind === "ui.action" && e.action === "new-mail") newMail();
      if (e.kind === "mail.mapiPending") openMapiMessages();
      if (e.kind === "mail.newMail") schedule(true);
      else if (e.kind === "mail.changed") schedule(false);
    }),
  );
  schedule(false);
  openMapiMessages();
  for (const start of MODULE_SERVICES) {
    try {
      stops.push(start());
    } catch (err) {
      rpc("app.log", { level: "error", message: `service start failed: ${errorForLog(err)}` }).catch(() => {});
    }
  }
  return () => {
    window.clearTimeout(timer);
    stops.forEach((s) => s());
  };
}
