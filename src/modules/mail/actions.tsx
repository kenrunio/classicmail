// Mail commands shared by the toolbar, menus, context menus, keyboard and the message window.
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { rpc, type RpcError } from "../../api/rpc";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openWindow } from "../../api/windows";
import { queryClient } from "../../api/query";
import { folderOf, inFolder, mailApi, type Attachment, type FlagKind, type ListResult, type MailFolder, type MessageRow, type MessageView } from "../../api/mail";
import { alertBox, confirmBox, errorBox, messageBox, type MenuItem } from "../../ui";
import { customFlagDialog, messageOptionsDialog, newFolderDialog, pickFolder, promptDialog } from "./dialogs";
import { useMailSel, useMailUi } from "./store";
import { addrList, escapeHtml as esc, fullDate } from "./format";
import { groupsWithout, idsOfRows, PAGE, rowFlagged } from "./conversations";
import { findAll } from "./findAll";
import { items, undoable } from "../../api/undo";
import { onCmEvent } from "../../api/events";
import { useStatus } from "../../shell/store";

const fail = (e: unknown) => errorBox(e);

/**
 * The messages leave the lists on screen at once (Archive, Delete, Move, Junk), as in Outlook: the
 * list's own reload, which follows, can take a second on a big mailbox. A conversation goes when all
 * of its messages do. Returns how to put them back if the action fails.
 */
function dropFromLists(ids: number[]): (e: unknown) => void {
  const gone = new Set(ids);
  const goes = (r: MessageRow) => (r.conv ? r.conv.ids.every((i) => gone.has(i)) : gone.has(r.id));
  // positions of the removed rows in each list (the view's key): its total and the counts of its
  // groups (Today, Yesterday…) drop with them, so the rows below and the headers move up together
  const removed = new Map<unknown, number[]>();
  for (const [key, d] of queryClient.getQueriesData<ListResult>({ queryKey: ["mail", "list"] })) {
    if (!d?.rows?.length || key[3] !== "page") continue;
    const at = d.rows.flatMap((r, i) => (goes(r) ? [Number(key[4]) * PAGE + i] : []));
    if (!at.length) continue;
    queryClient.setQueryData<ListResult>(key, { ...d, rows: d.rows.filter((r) => !goes(r)), total: Math.max(0, d.total - at.length) });
    removed.set(key[2], [...(removed.get(key[2]) ?? []), ...at]);
  }
  for (const [key, d] of queryClient.getQueriesData<ListResult>({ queryKey: ["mail", "list"] })) {
    const at = removed.get(key[2]);
    if (!d || !at || key[3] !== "meta") continue;
    queryClient.setQueryData<ListResult>(key, { ...d, total: Math.max(0, d.total - at.length), groups: d.groups && groupsWithout(d.groups, at) });
  }
  return (e) => {
    void queryClient.invalidateQueries({ queryKey: ["mail", "list"] });
    fail(e);
  };
}

export type ComposeMode = "new" | "reply" | "replyAll" | "forward" | "draft";

export function compose(mode: ComposeMode, id?: number, extra: Record<string, string> = {}) {
  const title = mode === "new" ? "Untitled - Message" : mode === "forward" ? "FW: - Message" : mode === "draft" ? "Message" : "RE: - Message";
  return openWindow("compose", { mode, id, ...extra }, { title });
}

/** Reply with AI: a Reply All whose window asks the AI for a short draft (ComposeWindow `aiDraft`). */
export function replyWithAi(id: number) {
  return compose("replyAll", id, { aiDraft: "1" });
}

/** Open an item like a double-click: drafts → compose, Outbox items → compose, else the message window. */
export function openItem(row: Pick<MessageRow, "id" | "isDraft" | "subject"> & { outboxId?: number | null }) {
  if (row.outboxId) return openWindow("compose", { mode: "draft", outboxId: row.outboxId }, { title: `${row.subject || "Untitled"} - Message` });
  if (row.isDraft) return compose("draft", row.id);
  return openWindow("message", { id: row.id }, { title: `${row.subject || "(no subject)"} - Message` });
}

/** Actions › Create Rule… for one message: its own dialog-sized window (not the Rules and Alerts one). */
export function createRule(messageId: number) {
  return openWindow("rules", { fromMessageId: messageId }, { title: "Create Rule", width: 540, height: 340 });
}

export function selectedIds(): number[] {
  return useMailSel.getState().selected.filter((id) => id > 0);
}

export function currentFolderId(): number | undefined {
  const f = useMailUi.getState().folder;
  return f?.kind === "folder" ? f.id : undefined;
}

/** `record` = false for automatic mark-as-read (reading pane / opening): not an undo step. */
export async function markRead(ids: number[], read: boolean, record = true) {
  if (!ids.length) return;
  if (!record) return void (await mailApi.markRead(ids, read).catch(fail));
  await undoable(`Mark ${items(ids.length)} as ${read ? "Read" : "Unread"}`, "mail.markRead", { ids, read }).catch(fail);
}

export async function setFlag(ids: number[], flag: FlagKind) {
  if (!ids.length) return;
  let due: string | undefined;
  if (flag === "custom") {
    const d = await customFlagDialog();
    if (!d) return;
    due = d;
  }
  const what = flag === "none" ? "Clear Flag" : flag === "complete" ? "Mark Complete" : "Flag";
  await undoable(`${what} (${items(ids.length)})`, "mail.setFlag", { ids, flag, due }).catch(fail);
}

/** Insert / flag column click: flag (today's quick flag) or clear. A conversation row flags all
 * of its messages in the view. */
export async function toggleFlag(rows: Pick<MessageRow, "id" | "isFlagged" | "flagStatus" | "conv">[]) {
  if (!rows.length) return;
  const flagged = rows.every(rowFlagged);
  await setFlag(idsOfRows(rows), flagged ? "complete" : "today");
}

export function followUpMenu(ids: () => number[]): MenuItem[] {
  const f = (label: string, flag: FlagKind, icon = "flag"): MenuItem => ({ label, icon, onClick: () => setFlag(ids(), flag) });
  return [
    f("Today", "today"),
    f("Tomorrow", "tomorrow"),
    f("This Week", "thisWeek"),
    f("Next Week", "nextWeek"),
    f("No Date", "noDate"),
    f("Custom…", "custom"),
    { type: "separator" },
    f("Mark Complete", "complete", "flag-complete"),
    f("Clear Flag", "none", "flag-outline"),
  ];
}

/** The warning when `permanent` of the `total` items being deleted are in Deleted Items. */
export function permanentDeleteText(permanent: number, total: number): string {
  if (permanent < total) return `${permanent} of the ${total} selected items will be permanently deleted. The others will be moved to the Deleted Items folder.`;
  return total === 1 ? "The selected item will be permanently deleted." : `The ${total} selected items will be permanently deleted.`;
}

/** Delete: to Deleted Items, or for good for what is already there (after asking, wherever they
 *  are listed: a folder, search results, a message window). Returns whether it ran. */
export async function deleteItems(ids: number[]): Promise<boolean> {
  if (!ids.length) return false;
  let permanent: number;
  try {
    // the back end's own rule (src-tauri/src/mail/actions.rs, permanent_count)
    permanent = (await mailApi.deleteInfo(ids)).permanent;
  } catch (e) {
    fail(e);
    return false;
  }
  if (permanent > 0) {
    // "Don't ask me again" here is the Options checkbox "Warn before permanently deleting items"
    const ok = await messageBox({
      title: "ClassicMail",
      text: permanentDeleteText(permanent, ids.length),
      icon: "warning",
      buttons: ["Yes", "No"],
      defaultButton: "Yes",
      dontAsk: { setting: "mail.confirmPermanentDelete", answer: "Yes" },
    });
    if (ok !== "Yes") return false;
  }
  const restore = dropFromLists(ids);
  return undoable(`Delete ${items(ids.length)}`, "mail.delete", { ids }).then(
    () => true,
    (e) => (restore(e), false),
  );
}

/** Archive (Backspace): out of the folder being viewed, kept in All Mail (Gmail archive). Returns
 *  whether it ran. */
export async function archiveItems(ids: number[]): Promise<boolean> {
  if (!ids.length) return false;
  const restore = dropFromLists(ids);
  return undoable(`Archive ${items(ids.length)}`, "mail.archive", { ids, fromFolderId: currentFolderId() }).then(
    () => true,
    (e) => (restore(e), false),
  );
}

/** Move / Copy to Folder…: any folder of any mailbox (into another mailbox the messages are copied
 *  across, like Outlook). `_accountId`: the mailbox of the messages (the picker shows them all). */
export async function moveToFolder(ids: number[], copy = false, _accountId?: number) {
  if (!ids.length) return;
  const f = await pickFolder({
    title: copy ? "Copy Items" : "Move Items",
    prompt: copy ? "Copy the selected items to the folder:" : "Move the selected items to the folder:",
    initial: currentFolderId(),
  });
  if (!f) return;
  await moveTo(ids, f, copy);
}

/** What mail.move / mail.copy did with messages from another mailbox (src-tauri mail/actions.rs). */
interface Transferred {
  imported: { accountId: number; gmailId: string }[];
  failed: { id: number; message: string }[];
}

/** Into another mailbox each message is downloaded and uploaded again: the status bar counts them
 *  (`mail.transfer` events), and any that couldn't be copied are listed afterwards. */
async function transferring(copy: boolean, f: MailFolder, run: () => Promise<Transferred | null>) {
  const what = copy ? "Copying" : "Moving";
  const stop = onCmEvent((e) => {
    if (e.kind !== "mail.transfer") return;
    const total = Number(e.total);
    if (Number(e.done) < total) useStatus.getState().flash(`${what} ${Number(e.done) + 1} of ${items(total)} to ${f.name}…`, 60000);
  });
  try {
    const r = await run();
    if (r?.imported.length) useStatus.getState().flash(`${copy ? "Copied" : "Moved"} ${items(r.imported.length)} to ${f.name}`);
    if (r?.failed.length) {
      const why = [...new Set(r.failed.map((x) => x.message))].join("\n");
      await alertBox(`${items(r.failed.length)} couldn't be ${copy ? "copied" : "moved"} to ${f.name}:\n${why}`, "ClassicMail", "warning");
    }
  } finally {
    stop();
  }
}

export async function moveTo(ids: number[], f: MailFolder, copy = false) {
  if (copy) {
    await transferring(true, f, () => undoable<Transferred>(`Copy ${items(ids.length)} to ${f.name}`, "mail.copy", { ids, folderId: f.id })).catch(fail);
    return;
  }
  const restore = dropFromLists(ids);
  await transferring(false, f, () => undoable<Transferred>(`Move ${items(ids.length)} to ${f.name}`, "mail.move", { ids, folderId: f.id, fromFolderId: currentFolderId() })).catch(
    restore,
  );
}

export function markJunk(ids: number[], junk: boolean) {
  if (!ids.length) return Promise.resolve();
  const restore = dropFromLists(ids);
  return undoable(`Mark ${items(ids.length)} as ${junk ? "Junk" : "Not Junk"}`, junk ? "mail.markJunk" : "mail.notJunk", { ids }).then(() => {}, restore);
}

export function junkMenu(ids: () => number[], sender?: () => string | undefined): MenuItem[] {
  return [
    {
      label: "Add Sender to Blocked Senders List",
      icon: "junk",
      onClick: () => markJunk(ids(), true),
    },
    {
      label: "Add Sender to Safe Senders List",
      onClick: () => {
        const s = sender?.();
        if (s) mailApi.addSafeSender(s).catch(fail);
      },
    },
    {
      label: "Add Sender's Domain (@example.com) to Safe Senders List",
      onClick: () => {
        const s = sender?.();
        if (s?.includes("@")) mailApi.addSafeSender(`@${s.split("@")[1]}`).catch(fail);
      },
    },
    { type: "separator" },
    { label: "Mark as Junk", icon: "junk", onClick: () => markJunk(ids(), true) },
    { label: "Mark as Not Junk", onClick: () => markJunk(ids(), false) },
  ];
}

export function findFromSender(email: string) {
  findAll(`from:"${email}"`);
}

export function findRelated(subject: string) {
  const s = subject.replace(/^((re|fw|fwd)\s*:\s*)+/i, "").trim();
  findAll(`subject:"${s.replace(/"/g, "")}"`);
}

/** Right-click menu for message rows (Outlook 2007 order). A conversation row stands for its
 * messages in the view (Open / Reply / Print use its newest one). */
export function messageMenu(rows: MessageRow[]): MenuItem[] {
  const ids = () => idsOfRows(rows);
  const one = rows.length === 1 ? rows[0] : null;
  const anyUnread = rows.some((r) => (r.conv ? r.conv.unread > 0 : !r.isRead));
  if (rows.some((r) => r.outboxId)) {
    return [{ label: "Open", bold: true, onClick: () => one && openItem(one) }];
  }
  return [
    { label: "Open", icon: "folder-open", bold: true, disabled: !one, onClick: () => one && openItem(one) },
    { label: "Print", icon: "print", disabled: !one, onClick: () => one && printMessage(one.id) },
    { type: "separator" },
    // Reply replies to everyone (Reply to One is a toggle in the compose window)
    { label: "Reply", icon: "reply", disabled: !one, onClick: () => one && compose("replyAll", one.id) },
    { label: "Reply to All", icon: "reply-all", disabled: !one, onClick: () => one && compose("replyAll", one.id) },
    { label: "Reply with AI", icon: "reply-ai", disabled: !one, onClick: () => one && replyWithAi(one.id) },
    { label: "Forward", icon: "forward", disabled: !one, onClick: () => one && compose("forward", one.id) },
    { type: "separator" },
    { label: "Follow Up", icon: "flag", submenu: followUpMenu(ids) },
    anyUnread
      ? { label: "Mark as Read", icon: "mark-read", shortcut: "Ctrl+Q", onClick: () => markRead(ids(), true) }
      : { label: "Mark as Unread", icon: "mark-unread", shortcut: "Ctrl+U", onClick: () => markRead(ids(), false) },
    {
      label: "Find All",
      icon: "find",
      disabled: !one,
      submenu: one
        ? [
            { label: "Related Messages…", onClick: () => findRelated(one.subject) },
            { label: "Messages from Sender…", onClick: () => findFromSender(one.fromEmail) },
          ]
        : [],
    },
    { type: "separator" },
    {
      label: "Create Rule…",
      icon: "rules",
      disabled: !one,
      onClick: () => one && createRule(one.id),
    },
    { label: "Junk E-mail", icon: "junk", submenu: junkMenu(ids, () => one?.fromEmail) },
    { type: "separator" },
    { label: "Archive", icon: "archive", shortcut: "Backspace", onClick: () => archiveItems(ids()) },
    { label: "Delete", icon: "delete", onClick: () => deleteItems(ids()) },
    { label: "Move to Folder…", icon: "move", shortcut: "Ctrl+Shift+V", onClick: () => moveToFolder(ids(), false, rows[0].accountId) },
    { label: "Copy to Folder…", icon: "copy", onClick: () => moveToFolder(ids(), true, rows[0].accountId) },
    { type: "separator" },
    { label: "Message Options…", icon: "options", disabled: !one, onClick: () => one && messageOptionsDialog(one.id, one.importance) },
  ];
}

// ---- folders -----------------------------------------------------------------------------------

export async function newFolder(parent?: MailFolder, accountId?: number) {
  const id = await newFolderDialog(parent, accountId);
  if (id) useMailUi.getState().setFolder({ kind: "folder", id, accountId: parent?.accountId ?? accountId ?? 0 });
}

export async function renameFolder(f: MailFolder) {
  const name = await promptDialog(`Rename "${f.name}"`, "New name:", f.name);
  if (name && name !== f.name) await undoable(`Rename Folder "${f.name}" to "${name}"`, "mail.renameFolder", { folderId: f.id, name }).catch(fail);
}

/** Drag a folder onto another folder, the Inbox, or the mailbox itself (`to` = null: the top). */
export async function moveFolder(f: MailFolder, to: MailFolder | null) {
  const where = to ? { parentId: to.id } : { accountId: f.accountId };
  await undoable(`Move Folder "${f.name}"`, "mail.moveFolder", { folderId: f.id, ...where }).catch(fail);
}

export async function deleteFolder(f: MailFolder) {
  const ok = await confirmBox(
    `Are you sure you want to delete the folder "${f.name}" and move all of its contents into the Deleted Items folder?`,
    "ClassicMail",
    "warning",
    "mail.deleteFolder",
  );
  if (ok) await mailApi.deleteFolder(f.id).catch(fail);
}

export async function emptyFolder(f: MailFolder) {
  const ok = await confirmBox(`Everything in the "${f.name}" folder will be permanently deleted.\nDo you want to continue?`, "ClassicMail", "warning", "mail.emptyFolder");
  if (ok) await mailApi.emptyFolder(f.id).catch(fail);
}

export async function markAllRead(f: MailFolder) {
  await undoable(`Mark All as Read in ${f.name}`, "mail.markAllRead", { folderId: f.id }).catch(fail);
}

// ---- attachments -------------------------------------------------------------------------------

/**
 * Open an attachment from the internet, Outlook-style: blocked types are refused (Save As still
 * works), others the back end flags with `confirm_open` ask first ("Opening Mail Attachment").
 * `open(confirmed)` does the rpc; `save` offers Save As instead.
 */
export async function openWithPrompt(open: (confirmed: boolean) => Promise<unknown>, save?: () => unknown) {
  // a big attachment can take a few seconds to download: show it's working, ignore repeat clicks
  if (opening) return;
  opening = true;
  document.body.classList.add("cm-busy");
  try {
    await open(false);
  } catch (e) {
    const err = e as RpcError;
    if (err?.code !== "confirm_open") {
      await alertBox(err?.message ?? String(e), "ClassicMail", "warning");
      return;
    }
    const answer = await messageBox({
      title: "Opening Mail Attachment",
      text: err.message,
      icon: "warning",
      buttons: save ? ["Open", "Save", "Cancel"] : ["Open", "Cancel"],
      defaultButton: "Cancel",
    });
    if (answer === "Open") await open(true).catch((e2) => alertBox(e2 instanceof Error ? e2.message : String(e2), "ClassicMail", "warning"));
    else if (answer === "Save" && save) await save();
  } finally {
    opening = false;
    document.body.classList.remove("cm-busy");
  }
}
let opening = false;

/** `choose`: Windows' "Open with" list instead of the default program. */
export async function openAttachment(a: Attachment, choose = false) {
  await openWithPrompt(
    (confirmed) => mailApi.openAttachment(a.id, confirmed, choose),
    () => saveAttachment(a),
  );
}

// ---- dragging an attachment out (src-tauri/src/mail/drag_out.rs) -----------------------------

export type DragOutOf = { attachmentId: number } | { archiveId: number; index: number };

/** On mouse down: get the attachment's file ready, so a drag can start at once. */
export function prepareDragOut(w: DragOutOf) {
  rpc("mail.attachments.prepareDrag", w).catch(() => {});
}

/** The page's own drag would carry no file: cancel it and let Windows drag the real file, which
 *  Explorer, the desktop, Word, Teams… take like a file dragged from a folder. */
export function dragOut(e: React.DragEvent, w: DragOutOf) {
  e.preventDefault();
  rpc("mail.attachments.drag", { ...w, window: getCurrentWebviewWindow().label }).catch(fail);
}

// Save As / Save All open in the folder used before for the sender's company (save_folders.rs)
export async function saveAttachment(a: Attachment) {
  const f = { attachmentId: a.id };
  const folder = await mailApi.suggestFolder(f).catch(() => null);
  const path = await saveDialog({ defaultPath: inFolder(folder, a.filename), title: "Save Attachment" }).catch(() => null);
  if (!path) return;
  await mailApi
    .saveAttachment(a.id, path)
    .then(() => mailApi.rememberFolder(f, folderOf(path)), fail)
    .catch(() => {});
}

export async function saveAllAttachments(messageId: number) {
  const f = { messageId };
  const folder = await mailApi.suggestFolder(f).catch(() => null);
  const dir = await openDialog({ directory: true, title: "Save All Attachments", defaultPath: folder ?? undefined }).catch(() => null);
  if (typeof dir !== "string" || !dir) return;
  await mailApi
    .saveAllAttachments(messageId, dir)
    .then(() => mailApi.rememberFolder(f, dir), fail)
    .catch(() => {});
}

// ---- print -------------------------------------------------------------------------------------

/** Print-friendly render (Outlook memo style) in a hidden iframe. */
export function printView(m: MessageView) {
  const row = (k: string, v: string) => (v ? `<tr><td class="k">${k}</td><td>${esc(v)}</td></tr>` : "");
  const from = m.fromName ? `${m.fromName} [${m.fromEmail}]` : m.fromEmail;
  const atts = m.attachments
    .filter((a) => !a.isInline)
    .map((a) => a.filename)
    .join("; ");
  // the body is the sanitised mail HTML; the frame isn't sandboxed (print needs it), so a CSP keeps
  // scripts out and remote content as blocked as in the reading pane
  const remote = m.hasRemote && !m.remoteBlocked ? " https: http:" : "";
  const csp = `default-src 'none'; script-src 'none'; img-src http://cmattach.localhost cmattach: data:${remote}; style-src 'unsafe-inline'${remote}; font-src data:${remote}; form-action 'none'; base-uri 'none'`;
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><title>${esc(m.subject)}</title><style>
    body{font-family:Calibri,'Segoe UI',Arial,sans-serif;font-size:11pt;margin:24px}
    .name{font-size:14pt;font-weight:bold;border-bottom:3px solid #000;padding-bottom:4px;margin-bottom:8px}
    table.h{border-collapse:collapse;margin-bottom:14px} table.h td{padding:1px 8px 1px 0;vertical-align:top} td.k{font-weight:bold;width:110px}
    img{max-width:100%}
  </style></head><body><div class="name">${esc(m.accountEmail)}</div><table class="h">
  ${row("From:", from)}${row("Sent:", fullDate(m.sentAt ?? m.receivedAt))}${row("To:", addrList(m.to))}${row("Cc:", addrList(m.cc))}${row("Subject:", m.subject)}${row("Attachments:", atts)}
  </table><div dir="auto">${m.html}</div></body></html>`;
  const f = document.createElement("iframe");
  f.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden";
  document.body.appendChild(f);
  const d = f.contentDocument!;
  d.open();
  d.write(doc);
  d.close();
  const go = () => {
    try {
      f.contentWindow?.focus();
      f.contentWindow?.print();
    } finally {
      setTimeout(() => f.remove(), 2000);
    }
  };
  // give images a moment
  setTimeout(go, 400);
}

export async function printMessage(id: number) {
  try {
    printView(await mailApi.get(id));
  } catch (e) {
    fail(e);
  }
}
