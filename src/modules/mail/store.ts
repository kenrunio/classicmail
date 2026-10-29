// Mail module UI state: current folder, selection, arrangement per folder, Instant Search.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Arrange, ListSource, SearchFolder } from "../../api/mail";

export type FolderSel =
  | { kind: "folder"; id: number; accountId: number }
  | { kind: "search"; sf: SearchFolder; accountId: number }
  | { kind: "ids"; ids: number[]; label: string }
  /** a folder of an Outlook archive (.pst), see src/modules/archive */
  | { kind: "archive"; id: number; archiveId: number };

export function folderKey(f: FolderSel | null): string {
  if (!f) return "";
  if (f.kind === "folder") return `f${f.id}`;
  if (f.kind === "search") return `s${f.sf}${f.accountId}`;
  if (f.kind === "archive") return `a${f.id}`;
  return `ids`;
}

export function sourceOf(f: FolderSel): ListSource {
  if (f.kind === "folder") return { folderId: f.id };
  if (f.kind === "search") return { searchFolder: f.sf, accountId: f.accountId };
  if (f.kind === "archive") return { ids: [] };
  return { ids: f.ids };
}

export interface ArrangeState {
  arrange: Arrange;
  desc: boolean;
  groups: boolean;
}

export const DEFAULT_ARRANGE: ArrangeState = { arrange: "date", desc: true, groups: true };

interface MailUi {
  folder: FolderSel | null;
  /** View › Show as Conversations for all mailboxes ("All mailboxes") */
  convDefault: boolean;
  /** per-folder overrides ("This folder") */
  convFolders: Record<string, boolean>;
  /** per folder key */
  arrangements: Record<string, ArrangeState>;
  /** collapsed group keys per folder key */
  collapsed: Record<string, string[]>;
  listWidth: number;
  listHeight: number;
  expanded: string[];
  setFolder: (f: FolderSel) => void;
  setArrange: (key: string, a: Partial<ArrangeState>) => void;
  toggleGroup: (key: string, group: string) => void;
  set: (p: Partial<Pick<MailUi, "listWidth" | "listHeight" | "expanded">>) => void;
  /** turn conversations on/off for one folder or for all of them (clears the overrides) */
  setConversations: (key: string, on: boolean, scope: "folder" | "all") => void;
}

/** Is Show as Conversations on for this folder key? */
export function conversationsOn(s: Pick<MailUi, "convDefault" | "convFolders">, key: string): boolean {
  return s.convFolders[key] ?? s.convDefault;
}

export const useMailUi = create<MailUi>()(
  persist(
    (set) => ({
      folder: null,
      convDefault: false,
      convFolders: {},
      arrangements: {},
      collapsed: {},
      listWidth: 380,
      listHeight: 260,
      expanded: [],
      setFolder: (folder) => set({ folder }),
      setArrange: (key, a) => set((s) => ({ arrangements: { ...s.arrangements, [key]: { ...(s.arrangements[key] ?? DEFAULT_ARRANGE), ...a } } })),
      toggleGroup: (key, group) =>
        set((s) => {
          const cur = s.collapsed[key] ?? [];
          return { collapsed: { ...s.collapsed, [key]: cur.includes(group) ? cur.filter((g) => g !== group) : [...cur, group] } };
        }),
      set: (p) => set(p),
      setConversations: (key, on, scope) => set((s) => (scope === "all" ? { convDefault: on, convFolders: {} } : { convFolders: { ...s.convFolders, [key]: on } })),
    }),
    {
      name: "cm:mail",
      partialize: (s) => ({
        folder: s.folder?.kind === "ids" ? null : s.folder,
        arrangements: s.arrangements,
        convDefault: s.convDefault,
        convFolders: s.convFolders,
        listWidth: s.listWidth,
        listHeight: s.listHeight,
        expanded: s.expanded,
      }),
    },
  ),
);

/** A search set just before the mail view appears (Find All from a message window, while another
 *  module or an archive was shown): the view keeps it instead of starting unfiltered. */
export const searchHandoff = { keep: false };

/**
 * Transient: selection + search (not persisted).
 *
 * The list selects rows by key: "m:<id>" a message row, "c:<accountId>:<threadId>" a conversation
 * header, "t:<id>" a message inside an expanded conversation. `selected` / `focused` are the
 * message ids those resolve to — what commands act on (a conversation header = all its messages
 * in the view; its newest one is `focused`, shown in the Reading Pane).
 */
interface MailSel {
  selected: number[];
  focused: number | null;
  keys: string[];
  focusKey: string | null;
  anchorKey: string | null;
  search: string;
  scope: "folder" | "all";
  /** select messages by id (rows "m:<id>") */
  setSelection: (selected: number[], focused?: number | null) => void;
  /** the list's selection: row keys + what they resolve to */
  select: (p: { keys: string[]; focusKey: string | null; anchorKey?: string | null; selected: number[]; focused: number | null }) => void;
  setSearch: (search: string, scope?: "folder" | "all") => void;
}

export const useMailSel = create<MailSel>((set) => ({
  selected: [],
  focused: null,
  keys: [],
  focusKey: null,
  anchorKey: null,
  search: "",
  scope: "folder",
  setSelection: (selected, focused) => {
    const f = focused === undefined ? (selected[selected.length - 1] ?? null) : focused;
    set({ selected, focused: f, keys: selected.map((id) => `m:${id}`), focusKey: f === null ? null : `m:${f}` });
  },
  select: ({ keys, focusKey, anchorKey, selected, focused }) => set((s) => ({ keys, focusKey, anchorKey: anchorKey === undefined ? s.anchorKey : anchorKey, selected, focused })),
  setSearch: (search, scope) => set((s) => ({ search, scope: scope ?? (search ? s.scope : "folder") })),
}));
