// Mail Navigation Pane: Favorite Folders + Mail Folders (one "Mailbox - Name" tree per account).
import { useEffect, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { rpc } from "../../api/rpc";
import { queryClient } from "../../api/query";
import { undoable } from "../../api/undo";
import { openWindow } from "../../api/windows";
import { Icon, TreeView, type MenuItem, type TreeNode } from "../../ui";
import type { MailFolder, Mailbox } from "../../api/mail";
import { useMailUi, useMailSel, type FolderSel } from "./store";
import { canReceive, folderNode, mailboxNode, searchNodes, useMailboxes, type NodeData } from "./folders";
import { archiveItems, deleteFolder, emptyFolder, markAllRead, moveFolder, moveTo, newFolder, renameFolder } from "./actions";
import { archiveMenu, archiveNodeId, archiveNodes, isArchiveNode, selectArchiveNode, useArchives } from "../archive/nav";

/** Your own order for the folder tree (press and hold a folder, then drag it): parent node id
 *  ("root" for the mailboxes) → child node ids in order. Stored in settings, so it persists. */
const ORDER_KEY = "mail.folderOrder";
type FolderOrder = Record<string, string[]>;

function useFolderOrder(): FolderOrder {
  return useQuery({ queryKey: ["settings", ORDER_KEY], queryFn: () => rpc<FolderOrder | null>("settings.get", { key: ORDER_KEY }) }).data ?? {};
}

async function saveFolderOrder(parent: string | null, ids: string[]) {
  const key = parent ?? "root";
  const cur = (queryClient.getQueryData<FolderOrder | null>(["settings", ORDER_KEY]) ?? {}) as FolderOrder;
  const next = { ...cur, [key]: ids };
  queryClient.setQueryData(["settings", ORDER_KEY], next);
  await undoable("Reorder Folders", "settings.set", { key: ORDER_KEY, value: next }).catch(() => {});
}

/** Apply the saved order: listed ids first in that order, any others (new folders) after, as before. */
export function applyFolderOrder(nodes: TreeNode[], order: FolderOrder, parent = "root"): TreeNode[] {
  const want = order[parent];
  let out = nodes;
  if (want?.length) {
    const pos = new Map(want.map((id, i) => [id, i]));
    out = [...nodes].sort((a, b) => (pos.get(a.id) ?? 1e9) - (pos.get(b.id) ?? 1e9));
  }
  return out.map((n) => (n.children?.length ? { ...n, children: applyFolderOrder(n.children, order, n.id) } : n));
}

// ---- Favorite Folders ------------------------------------------------------------------------
/** Your favorites (node ids: "f<folderId>" or "s<searchFolder>:<accountId>"), in order. Unset = the
 *  defaults (main Inbox, For Follow Up, main Sent Items). Right-click a folder to add/remove. */
const FAV_KEY = "mail.favorites";
/** what the pane currently shows (the context menu reads it to offer Add / Remove) */
let currentFavs: string[] = [];

function useFavoriteIds(): string[] | null {
  return useQuery({ queryKey: ["settings", FAV_KEY], queryFn: () => rpc<string[] | null>("settings.get", { key: FAV_KEY }) }).data ?? null;
}

async function saveFavorites(ids: string[], label = "Reorder Favorite Folders") {
  currentFavs = ids;
  queryClient.setQueryData(["settings", FAV_KEY], ids);
  await undoable(label, "settings.set", { key: FAV_KEY, value: ids }).catch(() => {});
}

function favId(d: NodeData): string | null {
  if (d.kind === "folder") return `f${d.folder.id}`;
  if (d.kind === "search") return `s${d.sf}:${d.mailbox.accountId}`;
  return null;
}

function favoriteMenuItem(d: NodeData): MenuItem | null {
  const id = favId(d);
  if (!id) return null;
  const isFav = currentFavs.includes(id);
  return isFav
    ? {
        label: "Remove from Favorite Folders",
        icon: "folder",
        onClick: () =>
          saveFavorites(
            currentFavs.filter((x) => x !== id),
            "Remove from Favorite Folders",
          ),
      }
    : { label: "Add to Favorite Folders", icon: "folder", onClick: () => saveFavorites([...currentFavs, id], "Add to Favorite Folders") };
}

/** "QX Global IR" → "QXGIR", "Kenny Green" → "KG": all-caps words kept, others by first letter. */
export function accountInitials(name: string, email = ""): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (!words.length) return (email.split("@")[0] ?? "").toUpperCase();
  return words.map((w) => (w.length > 1 && w === w.toUpperCase() && /\p{L}/u.test(w) ? w : w[0].toUpperCase())).join("");
}

/** The Favorite Folders nodes; a folder whose name also exists in another mailbox gets the
 *  account's initials when it isn't from the main (first) mailbox: "Inbox [DS]". */
export function favoriteNodes(ids: string[], mbs: Mailbox[]): TreeNode[] {
  const primary = mbs[0];
  const nameCount = new Map<string, number>();
  for (const mb of mbs) {
    for (const n of new Set(mb.folders.filter((f) => !f.hidden).map((f) => f.name.trim().toLowerCase()))) nameCount.set(n, (nameCount.get(n) ?? 0) + 1);
  }
  const out: TreeNode[] = [];
  for (const id of ids) {
    let node: TreeNode | undefined;
    let mb: Mailbox | undefined;
    if (id.startsWith("f")) {
      const fid = Number(id.slice(1));
      mb = mbs.find((m) => m.folders.some((f) => f.id === fid));
      const fo = mb?.folders.find((f) => f.id === fid);
      if (fo && mb) node = folderNode(fo, mb);
    } else if (id.startsWith("s")) {
      const acct = Number(id.split(":")[1]);
      mb = mbs.find((m) => m.accountId === acct);
      node = mb ? searchNodes(mb).find((n) => n.id === id) : undefined;
    }
    if (!node || !mb) continue;
    const dup = id.startsWith("s") ? mbs.length > 1 : (nameCount.get(node.label.trim().toLowerCase()) ?? 0) > 1;
    if (dup && primary && mb.accountId !== primary.accountId) node = { ...node, label: `${node.label} [${accountInitials(mb.displayName, mb.email)}]` };
    out.push(node);
  }
  return out;
}

function selOf(d: NodeData): FolderSel | null {
  if (d.kind === "folder") return { kind: "folder", id: d.folder.id, accountId: d.folder.accountId };
  if (d.kind === "search") return { kind: "search", sf: d.sf, accountId: d.mailbox.accountId };
  return null;
}

export function selNodeId(f: FolderSel | null): string | null {
  if (!f) return null;
  if (f.kind === "folder") return `f${f.id}`;
  if (f.kind === "search") return `s${f.sf}:${f.accountId}`;
  if (f.kind === "archive") return archiveNodeId(f);
  return null;
}

export function folderMenu(d: NodeData): MenuItem[] {
  const open = () => {
    const s = selOf(d);
    if (s) useMailUi.getState().setFolder(s);
  };
  if (d.kind === "mailbox" || d.kind === "searchRoot") {
    return [
      {
        label: "Open in New Window",
        icon: "new-window",
        onClick: () => openWindow("main", { mailAccount: d.mailbox.accountId }, { title: "Mail - ClassicMail", width: 1100, height: 760 }),
      },
      { type: "separator" },
      { label: "New Folder…", icon: "new-folder", onClick: () => newFolder(undefined, d.mailbox.accountId) },
    ];
  }
  if (d.kind === "search") {
    const fav = favoriteMenuItem(d);
    return [{ label: "Open", bold: true, icon: "folder-open", onClick: open }, ...(fav ? [{ type: "separator" } as MenuItem, fav] : [])];
  }
  const f = d.folder;
  const user = f.role === "user";
  const items: MenuItem[] = [
    { label: "Open", bold: true, icon: "folder-open", onClick: open },
    { label: "Open in New Window", icon: "new-window", onClick: () => openWindow("main", { mailFolder: f.id }, { title: `${f.name} - ClassicMail`, width: 1100, height: 760 }) },
    { type: "separator" },
    { label: "Mark All as Read", icon: "mark-read", disabled: f.unread === 0, onClick: () => markAllRead(f) },
  ];
  const fav = favoriteMenuItem(d);
  if (fav) items.push({ type: "separator" }, fav);
  if (f.role === "trash" || f.role === "spam") items.push({ label: `Empty "${f.name}"`, icon: "delete", onClick: () => emptyFolder(f) });
  items.push(
    { type: "separator" },
    { label: "New Folder…", icon: "new-folder", disabled: f.role === "outbox", onClick: () => newFolder(f) },
    { label: `Rename "${f.name}"…`, icon: "rename", disabled: !user, onClick: () => renameFolder(f) },
    { label: `Delete "${f.name}"`, icon: "delete", disabled: !(user || f.role === "placeholder"), onClick: () => deleteFolder(f) },
  );
  return items;
}

/** Archive isn't a folder (it is Gmail's "in no folder") but takes drops: that archives them. */
const isArchive = (d: NodeData | undefined) => d?.kind === "search" && d.sf === "archive";

// ---- dragging a folder onto another one (a quick drag; press-and-hold reorders) ----------------
/** the folder being dragged in the Mail Folders tree, if any */
let draggedFolder: MailFolder | null = null;

/** your own folders can be dragged */
export function folderDragItem(n: TreeNode): string | null {
  const d = n.data as NodeData | undefined;
  return d?.kind === "folder" && d.folder.role === "user" ? JSON.stringify({ kind: "folder", id: d.folder.id }) : null;
}

export function onFolderDrag(n: TreeNode | null) {
  const d = n?.data as NodeData | undefined;
  draggedFolder = d?.kind === "folder" ? d.folder : null;
}

/** where folder `f` may go: its own mailbox's top, Inbox, or another of its folders — not where it
 *  already is, nor itself or one of its subfolders */
export function folderTarget(f: MailFolder, d: NodeData | undefined): boolean {
  if (d?.kind === "mailbox") return d.mailbox.accountId === f.accountId && f.parentId !== null;
  if (d?.kind !== "folder") return false;
  const t = d.folder;
  if (t.accountId !== f.accountId || t.id === f.parentId) return false;
  if (t.role === "inbox") return !f.path.toLowerCase().startsWith("inbox/") || f.path.split("/").length > 2;
  if (t.role !== "user" && t.role !== "placeholder") return false;
  const own = f.path.toLowerCase();
  const to = t.path.toLowerCase();
  return to !== own && !to.startsWith(`${own}/`);
}

export function onDrop(n: TreeNode, payload: string, copy: boolean) {
  const d = n.data as NodeData | undefined;
  try {
    const p = JSON.parse(payload) as { kind: string; ids?: number[]; id?: number };
    if (p.kind === "folder") {
      const f = draggedFolder;
      draggedFolder = null;
      if (f && f.id === p.id && folderTarget(f, d)) void moveFolder(f, d?.kind === "folder" ? d.folder : null);
      return;
    }
    if (d?.kind !== "folder" && !isArchive(d)) return;
    if (p.kind !== "mail" || !p.ids?.length) return;
    if (d?.kind === "folder") moveTo(p.ids, d.folder, copy);
    else archiveItems(p.ids);
  } catch {
    /* not ours */
  }
}

export function canDrop(n: TreeNode) {
  const d = n.data as NodeData | undefined;
  if (draggedFolder) return folderTarget(draggedFolder, d);
  return (d?.kind === "folder" && canReceive(d.folder)) || isArchive(d);
}

export function NavPane() {
  // fields, not the whole store (which a splitter drag writes to on every move)
  const folder = useMailUi((s) => s.folder);
  const expandedIds = useMailUi((s) => s.expanded);
  const setFolder = useMailUi((s) => s.setFolder);
  const mbs = useMailboxes().data;
  const primary: Mailbox | undefined = mbs?.[0];

  // default selection: the first mailbox's Inbox; also honour ?mailFolder= for "Open in New Window"
  useEffect(() => {
    if (!mbs?.length) return;
    const p = new URLSearchParams(window.location.search);
    const wanted = Number(p.get("mailFolder"));
    if (wanted) {
      for (const mb of mbs) {
        const f = mb.folders.find((x) => x.id === wanted);
        if (f && !(folder?.kind === "folder" && folder.id === f.id)) {
          p.delete("mailFolder");
          history.replaceState(null, "", `?${p.toString()}`);
          setFolder({ kind: "folder", id: f.id, accountId: f.accountId });
          return;
        }
      }
    }
    const cur = folder;
    const exists = cur && (cur.kind !== "folder" || mbs.some((m) => m.folders.some((f) => f.id === cur.id)));
    if (!exists) {
      const acct = Number(p.get("mailAccount"));
      const mb = mbs.find((m) => m.accountId === acct) ?? mbs[0];
      const inbox = mb.folders.find((f) => f.role === "inbox");
      if (inbox) setFolder({ kind: "folder", id: inbox.id, accountId: inbox.accountId });
    }
  }, [mbs, folder, setFolder]);

  const savedFavs = useFavoriteIds();
  const favorites: TreeNode[] = useMemo(() => {
    if (!primary || !mbs) return [];
    let ids = savedFavs;
    if (!ids) {
      // defaults: main Inbox, For Follow Up, main Sent Items
      const role = (r: string) => primary.folders.find((f: MailFolder) => f.role === r);
      ids = [role("inbox") ? `f${role("inbox")!.id}` : "", `sfollowUp:${primary.accountId}`, role("sent") ? `f${role("sent")!.id}` : ""].filter(Boolean);
    }
    currentFavs = ids;
    return favoriteNodes(ids, mbs);
  }, [primary, mbs, savedFavs]);

  const folderOrder = useFolderOrder();
  // Outlook archives (.pst) follow the mailboxes, each as its own root
  const archives = useArchives().data;
  const trees = useMemo(
    () => applyFolderOrder([...(mbs ?? []).map((mb) => mailboxNode(mb, { withSearch: true })), ...archiveNodes(archives ?? [])], folderOrder),
    [mbs, archives, folderOrder],
  );
  // everything starts expanded; collapsed node ids are remembered with a "-" prefix
  const expanded = useMemo(() => {
    const collapsed = new Set(expandedIds);
    const s = new Set<string>();
    const walk = (n: TreeNode) => {
      if (n.children?.length && !collapsed.has(`-${n.id}`)) s.add(n.id);
      n.children?.forEach(walk);
    };
    trees.forEach(walk);
    return s;
  }, [expandedIds, trees]);

  const toggle = (id: string, open: boolean) => {
    const cur = new Set(useMailUi.getState().expanded);
    if (open) cur.delete(`-${id}`);
    else cur.add(`-${id}`);
    useMailUi.getState().set({ expanded: [...cur] });
  };

  const select = (n: TreeNode) => {
    if (isArchiveNode(n.data)) return selectArchiveNode(n.data);
    const d = n.data as NodeData | undefined;
    if (!d) return;
    const s = selOf(d);
    if (s) {
      useMailSel.getState().setSearch("");
      setFolder(s);
    }
  };
  const selected = selNodeId(folder);
  const menuFor = (n: TreeNode) => (isArchiveNode(n.data) ? archiveMenu(n.data) : n.data ? folderMenu(n.data as NodeData) : []);

  return (
    <div className="cm-mail-nav">
      <div className="cm-nav-section">Favorite Folders</div>
      <TreeView
        nodes={favorites}
        selectedId={selected}
        onSelect={select}
        menuFor={menuFor}
        onDropItems={onDrop}
        canDrop={canDrop}
        rootClassName="cm-mail-favs"
        onReorder={(_parent, ids) => saveFavorites(ids)}
      />
      <div className="cm-nav-section">Mail Folders</div>
      <div className="cm-mail-allitems" title="All Mail Items">
        <Icon name="mailbox" />
        <span className="cm-fill">All Mail Items</span>
        <Icon name="chevron-down" size={10} />
      </div>
      <div className="cm-mail-tree">
        <TreeView
          nodes={trees}
          selectedId={selected}
          onSelect={select}
          expanded={expanded}
          onToggle={toggle}
          menuFor={menuFor}
          onDropItems={onDrop}
          canDrop={canDrop}
          onReorder={saveFolderOrder}
          dragItem={folderDragItem}
          onDragItem={onFolderDrag}
        />
      </div>
    </div>
  );
}
