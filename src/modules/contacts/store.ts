// Contacts module UI state shared by the Navigation Pane, the view, the toolbar and the menus.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Contact } from "../../api/contacts";
import { isDateKey, type NameOrder, type SortSpec } from "./sort";

export type ViewId = "business" | "address" | "detailed" | "phone" | "company" | "location";

export const VIEWS: { id: ViewId; label: string }[] = [
  { id: "business", label: "Business Cards" },
  { id: "address", label: "Address Cards" },
  { id: "detailed", label: "Detailed Address Cards" },
  { id: "phone", label: "Phone List" },
  { id: "company", label: "By Company" },
  { id: "location", label: "By Location" },
];

export interface FolderSel {
  /** null = first account */
  accountId: number | null;
  groupId: number | null;
}

interface ContactsUi {
  folder: FolderSel;
  view: ViewId;
  query: string;
  /** View › Sort By (all views; table column clicks set it too) */
  sort: SortSpec;
  /** View › Display File As: "Last, First" (as filed) or "First Last" */
  nameOrder: NameOrder;
  selected: number[];
  anchor: number | null;
  /** contacts currently shown, in display order (for menus / keyboard) */
  items: Contact[];
  setFolder: (f: FolderSel) => void;
  setView: (v: ViewId) => void;
  setQuery: (q: string) => void;
  /** pick a sort order; choosing a new key starts ascending (dates: newest first) */
  setSort: (key: string, desc?: boolean) => void;
  setNameOrder: (o: NameOrder) => void;
  setItems: (items: Contact[]) => void;
  select: (ids: number[], anchor?: number | null) => void;
}

export const useContactsUi = create<ContactsUi>()(
  persist(
    (set) => ({
      folder: { accountId: null, groupId: null },
      view: "business",
      query: "",
      sort: { key: "fileAs", desc: false },
      nameOrder: "lastFirst",
      selected: [],
      anchor: null,
      items: [],
      setFolder: (folder) => set({ folder, selected: [], anchor: null, query: "" }),
      setView: (view) => set({ view }),
      setQuery: (query) => set({ query }),
      setSort: (key, desc) => set((s) => ({ sort: { key, desc: desc ?? (s.sort.key === key ? s.sort.desc : isDateKey(key)) } })),
      setNameOrder: (nameOrder) => set({ nameOrder }),
      setItems: (items) =>
        set((s) => {
          const ids = new Set(items.map((c) => c.id));
          const selected = s.selected.filter((id) => ids.has(id));
          return { items, selected: selected.length === s.selected.length ? s.selected : selected };
        }),
      select: (selected, anchor) => set((s) => ({ selected, anchor: anchor === undefined ? s.anchor : anchor })),
    }),
    { name: "cm:contacts", partialize: (s) => ({ folder: s.folder, view: s.view, sort: s.sort, nameOrder: s.nameOrder }) },
  ),
);

export function selectedContacts(): Contact[] {
  const s = useContactsUi.getState();
  const set = new Set(s.selected);
  return s.items.filter((c) => set.has(c.id));
}
