// Mail folder tree (Outlook order) built from mail.listFolders.
import type { TreeNode } from "../../ui";
import type { MailFolder, Mailbox, SearchFolder } from "../../api/mail";
import { useRpc } from "../../api/query";

export function useMailboxes() {
  return useRpc<Mailbox[]>(["mail", "folders"], "mail.listFolders", {}, { staleTime: 5000 });
}

const ROLE_ORDER: Record<string, number> = { trash: 1, drafts: 2, inbox: 3, spam: 4, outbox: 5, sent: 6 };

export const ROLE_ICON: Record<string, string> = {
  inbox: "inbox",
  drafts: "drafts",
  trash: "deleted",
  spam: "junk",
  outbox: "outbox",
  sent: "sent",
  all: "archive",
  user: "folder",
  placeholder: "folder",
};

export type NodeData =
  | { kind: "folder"; folder: MailFolder; mailbox: Mailbox }
  | { kind: "search"; sf: SearchFolder; mailbox: Mailbox }
  | { kind: "mailbox"; mailbox: Mailbox }
  | { kind: "searchRoot"; mailbox: Mailbox };

export function folderNode(f: MailFolder, mb: Mailbox, children?: TreeNode[]): TreeNode {
  const totalStyle = f.role === "drafts" || f.role === "outbox" || f.role === "spam";
  return {
    id: `f${f.id}`,
    label: f.name,
    icon: ROLE_ICON[f.role] ?? "folder",
    count: totalStyle ? f.total : f.unread,
    countStyle: totalStyle ? "total" : "unread",
    children,
    data: { kind: "folder", folder: f, mailbox: mb } satisfies NodeData,
  };
}

export function searchNodes(mb: Mailbox): TreeNode[] {
  const n = (sf: SearchFolder, label: string, count: number, style: "unread" | "total"): TreeNode => ({
    id: `s${sf}:${mb.accountId}`,
    label,
    icon: "search-folder",
    count,
    countStyle: style,
    data: { kind: "search", sf, mailbox: mb } satisfies NodeData,
  });
  // no "Unread Mail": with mark-as-read on view, the item you were reading vanished from under you
  return [n("followUp", "For Follow Up", mb.followUp, "total"), n("large", "Large Mail", 0, "unread")];
}

/** "Mailbox - Kenny Green" with system folders, Search Folders, then user folders nested. */
export function mailboxNode(mb: Mailbox, opts: { withSearch?: boolean; filter?: (f: MailFolder) => boolean } = {}): TreeNode {
  const visible = mb.folders.filter((f) => !f.hidden && (!opts.filter || opts.filter(f)));
  const kids = (parent: number | null): TreeNode[] =>
    visible
      .filter((f) => (f.role === "user" || f.role === "placeholder") && f.parentId === parent)
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
      .map((f) => {
        const c = kids(f.id);
        return folderNode(f, mb, c.length ? c : undefined);
      });
  // Labels named "Inbox/..." (e.g. created by Outlook over IMAP) belong under the real Inbox, not
  // under a second, empty "Inbox" folder: their top-level "Inbox" node is merged into the Inbox.
  const inboxAlias = visible.find((f) => (f.role === "user" || f.role === "placeholder") && f.parentId === null && f.name.trim().toLowerCase() === "inbox");
  const system = visible
    .filter((f) => ROLE_ORDER[f.role] !== undefined)
    .sort((a, b) => ROLE_ORDER[a.role] - ROLE_ORDER[b.role])
    .map((f) => {
      if (f.role === "inbox" && inboxAlias) {
        const c = kids(inboxAlias.id);
        return folderNode(f, mb, c.length ? c : undefined);
      }
      return folderNode(f, mb);
    });
  const all = visible.filter((f) => f.role === "all").map((f) => folderNode(f, mb));
  // Archive: Gmail-archived mail (not in the Inbox or any folder — what Backspace / Archive does)
  const archive: TreeNode = {
    id: `sarchive:${mb.accountId}`,
    label: "Archive",
    icon: "archive",
    title: "Archived messages: not in the Inbox or any other folder (Gmail All Mail only)",
    data: { kind: "search", sf: "archive", mailbox: mb } satisfies NodeData,
  };
  const children: TreeNode[] = [...system, archive];
  if (opts.withSearch) {
    children.push({
      id: `sroot:${mb.accountId}`,
      label: "Search Folders",
      icon: "search-folder",
      children: searchNodes(mb),
      data: { kind: "searchRoot", mailbox: mb } satisfies NodeData,
    });
  }
  children.push(...all, ...kids(null).filter((n) => !inboxAlias || n.id !== `f${inboxAlias.id}`));
  return {
    id: `mb${mb.accountId}`,
    label: `Mailbox - ${mb.displayName || mb.email}`,
    icon: "mailbox",
    title: mb.email,
    children,
    data: { kind: "mailbox", mailbox: mb } satisfies NodeData,
  };
}

export function findFolder(mbs: Mailbox[] | undefined, id: number | undefined): { folder: MailFolder; mailbox: Mailbox } | null {
  if (!mbs || id === undefined) return null;
  for (const mb of mbs) {
    const f = mb.folders.find((x) => x.id === id);
    if (f) return { folder: f, mailbox: mb };
  }
  return null;
}

export function folderOfRole(mbs: Mailbox[] | undefined, accountId: number, role: string): MailFolder | undefined {
  return mbs?.find((m) => m.accountId === accountId)?.folders.find((f) => f.role === role);
}

/** Can items be moved/dropped into this folder? */
export function canReceive(f: MailFolder): boolean {
  return !["drafts", "outbox", "placeholder"].includes(f.role);
}
