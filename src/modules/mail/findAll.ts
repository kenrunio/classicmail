// Find All (Related Messages, Messages from Sender): an Instant Search in All Mail Items. A message
// window has no mail list, so it hands the search to the main window, which shows Mail with it and
// comes to the front.
import { emitTo, listen } from "@tauri-apps/api/event";
import { rpc } from "../../api/rpc";
import { queryClient } from "../../api/query";
import { windowParams } from "../../api/windows";
import type { Mailbox } from "../../api/mail";
import { useShell } from "../../shell/store";
import { searchHandoff, useMailSel, useMailUi } from "./store";

/** Tauri event to the main window; the payload is the search text. */
export const FIND_EVENT = "cm-mail-find";

/** Search All Mail Items for `search`, in the main window when this isn't a main window. */
export function findAll(search: string) {
  if (["main", "explorer"].includes(windowParams().get("window") || "main")) return showSearch(search);
  emitTo("main", FIND_EVENT, search).catch((e) => rpc("app.log", { level: "error", message: `Find All: ${String(e)}` }).catch(() => {}));
  rpc("app.showMain", {}).catch(() => {});
}

/** The main window's side: Mail, a mailbox folder (not an archive), the search in All Mail Items. */
export function showSearch(search: string) {
  const shell = useShell.getState();
  let appears = shell.module !== "mail";
  if (useMailUi.getState().folder?.kind === "archive") {
    const inbox = queryClient
      .getQueryData<Mailbox[]>(["mail", "folders"])
      ?.flatMap((m) => m.folders)
      .find((f) => f.role === "inbox");
    if (inbox) {
      useMailUi.getState().setFolder({ kind: "folder", id: inbox.id, accountId: inbox.accountId });
      appears = true;
    }
  }
  // the mail view isn't on screen yet: it keeps this search when it appears
  if (appears) searchHandoff.keep = true;
  useMailSel.getState().setSearch(search, "all");
  window.dispatchEvent(new CustomEvent("cm-mail-search-changed"));
  if (shell.module !== "mail") shell.setModule("mail");
}

/** Main window service: answers Find All from message windows. */
export function startFindService(): () => void {
  const un = listen<string>(FIND_EVENT, (e) => showSearch(e.payload));
  return () => {
    un.then((f) => f()).catch(() => {});
  };
}
