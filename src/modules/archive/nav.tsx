// Outlook archives in the Mail Navigation Pane: one root per open .pst ("Archive 2016") with its
// folder tree and unread counts, plus the root's right-click menu.
import type { TreeNode, MenuItem } from "../../ui";
import { useRpc } from "../../api/query";
import type { Archive, ArchiveFolder } from "../../api/archive";
import { useMailSel, useMailUi, type FolderSel } from "../mail/store";
import { archiveProperties, closeArchive, searchAllArchives, toggleIndexText, retryArchive } from "./actions";
import "./archive.css";

export function useArchives() {
  return useRpc<Archive[]>(["archive", "list"], "archive.list", {}, { staleTime: 5000 });
}

export type ArchiveNodeData = { kind: "archiveRoot"; archive: Archive } | { kind: "archiveFolder"; archive: Archive; folder: ArchiveFolder };

export function isArchiveNode(d: unknown): d is ArchiveNodeData {
  const k = (d as { kind?: string } | undefined)?.kind;
  return k === "archiveRoot" || k === "archiveFolder";
}

/** Outlook's data-file icon, our own drawing: a filing box with a blue folder tab. */
export function ArchiveIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ flex: "none", display: "block" }}>
      <path d="M2.5 6.5h11v7h-11z" fill="#f3e3b5" stroke="#9a7a32" />
      <path d="M1.5 4.5h13v2h-13z" fill="#e2c77d" stroke="#9a7a32" />
      <path d="M3 4.5V2.5h4l1 1h5v1" fill="#c5d8f3" stroke="#5579b8" />
      <path d="M6 9h4" stroke="#9a7a32" strokeWidth="1.4" />
    </svg>
  );
}

const FOLDER_ICON: Record<string, string> = {
  inbox: "inbox",
  "sent items": "sent",
  sent: "sent",
  "deleted items": "deleted",
  drafts: "drafts",
  outbox: "outbox",
  "junk e-mail": "junk",
  "junk email": "junk",
};

function statusText(a: Archive): string {
  switch (a.status) {
    case "pending":
    case "indexing":
      return a.expected > 0 ? `Indexing… ${a.indexed.toLocaleString()} of ${a.expected.toLocaleString()}` : "Indexing…";
    case "locked":
      return "Open in Outlook";
    case "missing":
      return "File not found";
    case "error":
      return "Can't be read";
    default:
      return "";
  }
}

export function archiveNodes(archives: Archive[]): TreeNode[] {
  return archives.map((a) => {
    const mail = a.folders.filter((f) => f.kind === "mail");
    const kids = (parent: number | null): TreeNode[] =>
      mail
        .filter((f) => f.parentId === parent)
        .sort((x, y) => x.name.localeCompare(y.name, undefined, { sensitivity: "base" }))
        .map((f) => {
          const c = kids(f.id);
          return {
            id: `af${f.id}`,
            label: f.name,
            icon: FOLDER_ICON[f.name.trim().toLowerCase()] ?? "folder",
            count: f.unread,
            countStyle: "unread",
            title: `${f.total.toLocaleString()} item${f.total === 1 ? "" : "s"}`,
            children: c.length ? c : undefined,
            data: { kind: "archiveFolder", archive: a, folder: f } satisfies ArchiveNodeData,
          } as TreeNode;
        });
    // top-level folders whose parent isn't shown (e.g. under a calendar folder) still appear
    const shown = new Set(mail.map((f) => f.id));
    const orphans = mail.filter((f) => f.parentId !== null && !shown.has(f.parentId)).map((f) => f.parentId);
    const roots = [...kids(null), ...[...new Set(orphans)].flatMap((p) => kids(p))];
    const st = statusText(a);
    return {
      id: `ar${a.id}`,
      label: st ? `${a.displayName} (${st})` : a.displayName,
      prefix: <ArchiveIcon />,
      title: `${a.path}${a.error ? `\n${a.error}` : ""}`,
      children: roots.length ? roots : undefined,
      data: { kind: "archiveRoot", archive: a } satisfies ArchiveNodeData,
    } as TreeNode;
  });
}

export function archiveSel(d: ArchiveNodeData): FolderSel | null {
  if (d.kind === "archiveFolder") return { kind: "archive", id: d.folder.id, archiveId: d.archive.id };
  return null;
}

export function selectArchiveNode(d: ArchiveNodeData) {
  const s = archiveSel(d);
  if (s) {
    useMailSel.getState().setSearch("");
    useMailUi.getState().setFolder(s);
  }
}

export function archiveMenu(d: ArchiveNodeData): MenuItem[] {
  const a = d.archive;
  const items: MenuItem[] = [];
  if (d.kind === "archiveFolder") items.push({ label: "Open", bold: true, icon: "folder-open", onClick: () => selectArchiveNode(d) }, { type: "separator" });
  if (a.status === "locked" || a.status === "missing" || a.status === "error") items.push({ label: "Try Again", icon: "refresh", onClick: () => retryArchive(a) });
  items.push(
    {
      label: "Search All Archives…",
      icon: "find",
      onClick: () => searchAllArchives(d.kind === "archiveFolder" ? d.folder : (a.folders.find((f) => f.kind === "mail") ?? null), a),
    },
    { label: "Index message text for search", checked: a.indexText, disabled: a.status !== "ready" && !a.indexText, onClick: () => toggleIndexText(a) },
    { type: "separator" },
    { label: `Close "${a.displayName}"`, icon: "close", onClick: () => closeArchive(a) },
    { type: "separator" },
    { label: "Properties…", icon: "options", onClick: () => archiveProperties(a.id) },
  );
  return items;
}

export function archiveNodeId(f: FolderSel | null): string | null {
  return f?.kind === "archive" ? `af${f.id}` : null;
}
