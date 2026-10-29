// Message-list layout (one line with columns vs Outlook's two-line rows) and the user's column
// set: which columns, their order and widths. Persisted per machine.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Arrange } from "../../api/mail";

export type ListLayout = "auto" | "singleLine" | "twoLine";

export type ColId = "importance" | "icon" | "attachment" | "from" | "to" | "subject" | "received" | "size" | "account" | "folder" | "flag";

export interface ColDef {
  id: ColId;
  /** header text (icon columns use `icon`) */
  label: string;
  /** shown in the column chooser */
  title: string;
  icon?: string;
  /** default width; `null` = takes the remaining space (Subject) */
  width: number | null;
  /** narrow icon columns can't be resized */
  fixed?: boolean;
  /** sort key when the header is clicked */
  sort?: Arrange;
}

export const COLUMNS: Record<ColId, ColDef> = {
  importance: { id: "importance", label: "", title: "Importance", icon: "importance-high", width: 18, fixed: true, sort: "importance" },
  icon: { id: "icon", label: "", title: "Icon", icon: "mail", width: 22, fixed: true },
  attachment: { id: "attachment", label: "", title: "Attachment", icon: "attachment", width: 18, fixed: true, sort: "attachments" },
  from: { id: "from", label: "From", title: "From", width: 190, sort: "from" },
  to: { id: "to", label: "To", title: "To", width: 190, sort: "to" },
  subject: { id: "subject", label: "Subject", title: "Subject", width: null, sort: "subject" },
  received: { id: "received", label: "Received", title: "Received", width: 150, sort: "date" },
  size: { id: "size", label: "Size", title: "Size", width: 64, sort: "size" },
  account: { id: "account", label: "Account", title: "Account", width: 150 },
  folder: { id: "folder", label: "In Folder", title: "In Folder (search results)", width: 110 },
  flag: { id: "flag", label: "", title: "Flag Status", icon: "flag-outline", width: 24, fixed: true, sort: "flag" },
};

export const DEFAULT_ORDER: ColId[] = ["importance", "icon", "attachment", "from", "subject", "received", "size", "folder", "flag"];

interface ColumnState {
  layout: ListLayout;
  order: ColId[];
  widths: Partial<Record<ColId, number>>;
  setLayout: (l: ListLayout) => void;
  toggle: (id: ColId) => void;
  move: (id: ColId, before: ColId | null) => void;
  setWidth: (id: ColId, w: number) => void;
  reset: () => void;
}

export const useColumns = create<ColumnState>()(
  persist(
    (set) => ({
      layout: "auto",
      order: DEFAULT_ORDER,
      widths: {},
      setLayout: (layout) => set({ layout }),
      toggle: (id) =>
        set((s) => {
          if (s.order.includes(id)) {
            // Subject always stays
            return id === "subject" ? s : { order: s.order.filter((c) => c !== id) };
          }
          // add before the trailing flag column if present, else at the end
          const at = s.order.indexOf("flag");
          const order = [...s.order];
          order.splice(at >= 0 ? at : order.length, 0, id);
          return { order };
        }),
      move: (id, before) =>
        set((s) => {
          if (id === before) return s;
          const order = s.order.filter((c) => c !== id);
          const at = before ? order.indexOf(before) : -1;
          order.splice(at >= 0 ? at : order.length, 0, id);
          return { order };
        }),
      setWidth: (id, w) => set((s) => ({ widths: { ...s.widths, [id]: Math.round(Math.max(40, Math.min(800, w))) } })),
      reset: () => set({ order: DEFAULT_ORDER, widths: {} }),
    }),
    { name: "cm:mailColumns" },
  ),
);

/** Columns to draw: the user's order; "In Folder" only appears in cross-folder views (search). */
export function visibleColumns(order: ColId[], cross: boolean): ColDef[] {
  return order.filter((c) => COLUMNS[c] && (c !== "folder" || cross)).map((c) => COLUMNS[c]);
}

export function colWidth(c: ColDef, widths: Partial<Record<ColId, number>>): number | null {
  if (c.width === null) return null;
  return c.fixed ? c.width : (widths[c.id] ?? c.width);
}

/** Should the list use Outlook's two-line rows? */
export function isTwoLine(layout: ListLayout, readingPane: string, viewWidth: number): boolean {
  if (layout === "twoLine") return true;
  if (layout === "singleLine") return false;
  // automatic, like Outlook: two lines with the Reading Pane on the right or in a narrow window
  return readingPane === "right" || (viewWidth > 0 && viewWidth < 640);
}
