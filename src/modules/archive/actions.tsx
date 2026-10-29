// Archive commands: File › Open › Outlook Data File…, Close, Properties, text indexing, Reply /
// Forward (normal compose windows prefilled from the archived item), attachments, print.
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { rpc } from "../../api/rpc";
import { queryClient } from "../../api/query";
import { openWindow } from "../../api/windows";
import { Dialog, alertBox, confirmBox, errorBox, type MenuItem } from "../../ui";
import { folderOf, inFolder, mailApi, type MessageRow, type MessageView } from "../../api/mail";
import { archiveApi, type Archive, type ArchiveAttachment, type ArchiveFolder, type ArchiveMessageView } from "../../api/archive";
import { openWithPrompt, printView } from "../mail/actions";
import { showMailDialog } from "../mail/dialogs";
import { useMailSel, useMailUi } from "../mail/store";
import { fullDate, sizeText } from "../mail/format";

const fail = (e: unknown) => errorBox(e);
const refresh = () => queryClient.invalidateQueries({ queryKey: ["archive"] });

/** File › Open › Outlook Data File… (several at once; remembers the folder). */
export async function openArchiveDialog() {
  const last = await rpc<string | null>("settings.get", { key: "archive.lastFolder" }).catch(() => null);
  const picked = await openDialog({
    title: "Open Outlook Data File",
    multiple: true,
    directory: false,
    defaultPath: last ?? undefined,
    filters: [{ name: "Outlook Data Files (*.pst)", extensions: ["pst"] }],
  }).catch(() => null);
  const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
  if (!paths.length) return;
  try {
    const r = await archiveApi.open(paths);
    refresh();
    if (r.errors.length) {
      const text = r.errors.map((e) => `${e.path.split(/[\\/]/).pop()}: ${e.message}`).join("\n\n");
      await alertBox(text, "ClassicMail", "warning");
    }
  } catch (e) {
    fail(e);
  }
}

export async function closeArchive(a: Archive) {
  if (
    !(await confirmBox(
      `Close "${a.displayName}"? The file itself is not changed; you can open it again with File › Open › Outlook Data File.`,
      "Close Archive",
      "warning",
      "archive.close",
    ))
  )
    return;
  const cur = useMailUi.getState().folder;
  if (cur?.kind === "archive" && cur.archiveId === a.id) useMailUi.setState({ folder: null });
  await archiveApi.close(a.id).catch(fail);
  refresh();
}

export async function toggleIndexText(a: Archive) {
  await archiveApi.setIndexText(a.id, !a.indexText).catch(fail);
  refresh();
}

export async function retryArchive(a: Archive) {
  await archiveApi.reindex(a.id).catch(fail);
  refresh();
}

/** Select a folder of the archive and search all archives (focus the search box). */
export function searchAllArchives(folder: ArchiveFolder | null, a: Archive) {
  if (folder) useMailUi.getState().setFolder({ kind: "archive", id: folder.id, archiveId: a.id });
  useMailSel.getState().setSearch("", "all");
  setTimeout(() => window.dispatchEvent(new CustomEvent("cm-archive-search-all")), 50);
}

function fmtBytes(n: number) {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(2)} GB`;
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

const STATUS: Record<string, string> = {
  pending: "Waiting to be indexed",
  indexing: "Indexing…",
  ready: "Indexed",
  error: "Could not be read",
  missing: "File not found",
  locked: "Open in Outlook",
};

/** Right-click › Properties: path, size, items, index state. */
export async function archiveProperties(id: number) {
  let a: Archive;
  try {
    a = await archiveApi.properties(id);
  } catch (e) {
    return fail(e);
  }
  const folders = a.folders.filter((f) => f.kind === "mail").length;
  const skipped = a.folders.filter((f) => f.kind !== "mail");
  const rows: [string, string][] = [
    ["Name:", a.displayName],
    ["File:", a.path],
    ["Size:", `${fmtBytes(a.fileSize)} (${a.format === "ansi" ? "Outlook 97-2002 format" : "Outlook 2003 and later format"})`],
    ["Folders:", `${folders.toLocaleString()}${skipped.length ? ` mail folders (${skipped.length} calendar/contacts/other folders not shown)` : ""}`],
    ["Items:", `${a.indexed.toLocaleString()}${a.status !== "ready" && a.expected ? ` of ${a.expected.toLocaleString()}` : ""}`],
    ["Status:", `${STATUS[a.status] ?? a.status}${a.error ? ` — ${a.error}` : ""}`],
    ["Indexed:", a.indexedAt ? `${fullDate(a.indexedAt)}${a.indexMs ? ` (${(a.indexMs / 1000).toFixed(1)} s)` : ""}` : "—"],
    [
      "Message text:",
      a.indexText
        ? a.textDone
          ? `Indexed for search (${a.textIndexed.toLocaleString()} items)`
          : `Indexing… ${a.textIndexed.toLocaleString()} of ${a.indexed.toLocaleString()}`
        : "Not indexed (subject, from and to are searchable)",
    ],
  ];
  showMailDialog((close) => (
    <Dialog
      title={`${a.displayName} Properties`}
      onClose={close}
      width={480}
      footer={
        <>
          <button className="cm-button" onClick={close} autoFocus>
            OK
          </button>
        </>
      }
    >
      <table className="cm-archive-props">
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}>
              <td>{k}</td>
              <td dir="auto">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="cm-archive-props-note">ClassicMail only reads this file; it is never changed.</div>
    </Dialog>
  ));
}

// ---- items --------------------------------------------------------------------------------------

export function archiveCompose(mode: "reply" | "replyAll" | "forward", id: number) {
  const title = mode === "forward" ? "FW: - Message" : "RE: - Message";
  return openWindow("compose", { mode, archive: id }, { title });
}

export function openArchiveItem(r: Pick<MessageRow, "id" | "subject">) {
  return openWindow("message", { archive: r.id }, { title: `${r.subject || "(no subject)"} - Message` });
}

/** Print through the mail print view (the archive name stands where the mailbox name goes). */
export function printArchive(m: ArchiveMessageView) {
  const v = {
    ...m,
    accountEmail: m.archiveName,
    attachments: m.attachments.map((a) => ({ id: a.index, filename: a.filename, mimeType: a.mimeType, size: a.size, isInline: a.isInline, contentId: a.contentId })),
  } as unknown as MessageView;
  printView(v);
}

export async function printArchiveItem(id: number) {
  try {
    printArchive(await archiveApi.get(id));
  } catch (e) {
    fail(e);
  }
}

export function archiveMessageMenu(rows: MessageRow[]): MenuItem[] {
  const one = rows.length === 1 ? rows[0] : null;
  return [
    { label: "Open", icon: "folder-open", bold: true, disabled: !one, onClick: () => one && openArchiveItem(one) },
    { label: "Print", icon: "print", disabled: !one, onClick: () => one && printArchiveItem(one.id) },
    { type: "separator" },
    { label: "Reply", icon: "reply", disabled: !one, onClick: () => one && archiveCompose("reply", one.id) },
    { label: "Reply to All", icon: "reply-all", disabled: !one, onClick: () => one && archiveCompose("replyAll", one.id) },
    { label: "Forward", icon: "forward", disabled: !one, onClick: () => one && archiveCompose("forward", one.id) },
  ];
}

export async function openArchiveAttachment(messageId: number, a: ArchiveAttachment, choose = false) {
  await openWithPrompt(
    (confirmed) => archiveApi.openAttachment(messageId, a.index, confirmed, choose),
    () => saveArchiveAttachment(messageId, a),
  );
}

// like mail's Save As / Save All: the folder used before for the sender's company
export async function saveArchiveAttachment(messageId: number, a: ArchiveAttachment) {
  const f = { archiveMessageId: messageId };
  const folder = await mailApi.suggestFolder(f).catch(() => null);
  const path = await saveDialog({ defaultPath: inFolder(folder, a.filename), title: "Save Attachment" }).catch(() => null);
  if (!path) return;
  await archiveApi
    .saveAttachment(messageId, a.index, path)
    .then(() => mailApi.rememberFolder(f, folderOf(path)), fail)
    .catch(() => {});
}

export async function saveAllArchiveAttachments(messageId: number) {
  const f = { archiveMessageId: messageId };
  const folder = await mailApi.suggestFolder(f).catch(() => null);
  const dir = await openDialog({ directory: true, title: "Save All Attachments", defaultPath: folder ?? undefined }).catch(() => null);
  if (typeof dir !== "string" || !dir) return;
  await archiveApi
    .saveAllAttachments(messageId, dir)
    .then(() => mailApi.rememberFolder(f, dir), fail)
    .catch(() => {});
}

export function attachmentTitle(a: ArchiveAttachment) {
  return `${a.filename} (${sizeText(a.size)})\nDouble-click to open`;
}
