// Outlook 2007 "Color Categories" — reusable by any module:
//   useCategories()                    the master list (+ palette), cached under ["calendar", "categories"]
//   categoryColor(name, list)          hex colour of a category name (undefined = none / unknown)
//   categorizeMenu(current, list, apply) the "Categorize ▸" submenu items (Ctrl+F2..F12 shown)
//   openColorCategories(selected)      the "Color Categories" dialog → Promise<string[] | null>
//   <ColorCategoriesHost/>             mount where the dialog can open (several per window is fine)
//   shortcutCategory(key, list)        "F2".."F12" → category name
// Definitions live in the back end (settings key `categories.list`, rpc calendar.*Categories).
import { useEffect, useState } from "react";
import { create } from "zustand";
import { useQueryClient } from "@tanstack/react-query";
import { useRpc } from "../../api/query";
import { cal, type Category, type CategoryList } from "../../api/calendar";
import { Dialog, errorBox, ICONS, confirmBox, type MenuItem } from "../../ui";

/** Outlook's 25 category colours (preset → [name, hex]); kept in step with categories.rs. */
export const PALETTE: [number, string, string][] = [
  [1, "Red", "#e7a1a2"],
  [2, "Orange", "#f9ba89"],
  [3, "Peach", "#f7dd8f"],
  [4, "Yellow", "#fcfa90"],
  [5, "Green", "#78d168"],
  [6, "Teal", "#9fdcc9"],
  [7, "Olive", "#c6d2b0"],
  [8, "Blue", "#9db7e8"],
  [9, "Purple", "#b5a1e2"],
  [10, "Maroon", "#daaec2"],
  [11, "Steel", "#dad9dc"],
  [12, "Dark Steel", "#6b7994"],
  [13, "Gray", "#bfbfbf"],
  [14, "Dark Gray", "#6f6f6f"],
  [15, "Black", "#4f4f4f"],
  [16, "Dark Red", "#c11a25"],
  [17, "Dark Orange", "#e2620d"],
  [18, "Dark Peach", "#c79930"],
  [19, "Dark Yellow", "#b9b300"],
  [20, "Dark Green", "#368f2b"],
  [21, "Dark Teal", "#329b7a"],
  [22, "Dark Olive", "#778b45"],
  [23, "Dark Blue", "#2858a5"],
  [24, "Dark Purple", "#5c3fa3"],
  [25, "Dark Maroon", "#93446b"],
];

/** Google Calendar event colours (colorId → hex), for items coloured in Google. */
export const GOOGLE_COLORS: Record<string, string> = {
  "1": "#a4bdfc",
  "2": "#7ae7bf",
  "3": "#dbadff",
  "4": "#ff887c",
  "5": "#fbd75b",
  "6": "#ffb878",
  "7": "#46d6db",
  "8": "#e1e1e1",
  "9": "#5484ed",
  "10": "#51b749",
  "11": "#dc2127",
};

export const SHORTCUTS = ["F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];

const DEFAULTS: Category[] = [
  { name: "Red Category", color: 1, shortcut: "" },
  { name: "Orange Category", color: 2, shortcut: "" },
  { name: "Yellow Category", color: 4, shortcut: "" },
  { name: "Green Category", color: 5, shortcut: "" },
  { name: "Blue Category", color: 8, shortcut: "" },
  { name: "Purple Category", color: 9, shortcut: "" },
];

export const presetHex = (p: number) => PALETTE.find((x) => x[0] === p)?.[2];

// colour swatches for menus: registered as icons named "cm-cat-<preset>" (0 = no colour)
for (const [p, , hex] of PALETTE) ICONS[`cm-cat-${p}`] = <rect x="2" y="3" width="12" height="10" rx="1.5" fill={hex} stroke="rgba(0,0,0,.35)" />;
ICONS["cm-cat-0"] = <rect x="2" y="3" width="12" height="10" rx="1.5" fill="#fff" stroke="rgba(0,0,0,.35)" />;

let latest: Category[] = DEFAULTS;

export function useCategories(): Category[] {
  const q = useRpc<CategoryList>(["calendar", "categories"], "calendar.listCategories", {}, { staleTime: 5 * 60_000 });
  const list = q.data?.categories ?? DEFAULTS;
  latest = list;
  return list;
}

/** The last list seen by `useCategories` (for menus built outside React). */
export const currentCategories = () => latest;

export function findCategory(name: string, list: Category[]) {
  const n = name.toLowerCase();
  return list.find((c) => c.name.toLowerCase() === n);
}

export function categoryColor(name: string, list: Category[]): string | undefined {
  const c = findCategory(name, list);
  return c && c.color ? presetHex(c.color) : undefined;
}

type Colorable = { summary?: string; categories?: string[]; colorId?: string; calendarColor: string };

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, (m) => `\\${m}`);
const reCache = new Map<string, RegExp>();
/** whole-word, case-insensitive, Unicode-aware (works for Hebrew too) */
function wordRe(word: string): RegExp {
  let re = reCache.get(word);
  if (!re) {
    re = new RegExp(`(^|[^\\p{L}\\p{N}])${escapeRe(word)}(?=$|[^\\p{L}\\p{N}])`, "iu");
    reCache.set(word, re);
  }
  return re;
}

/** Categories whose name (or one of their match words) appears in the subject. */
export function matchedCategories(subject: string | undefined, list: Category[]): string[] {
  if (!subject) return [];
  const out: string[] = [];
  for (const c of list) {
    if (c.autoMatch === false || !c.color) continue;
    const words = [c.name, ...(c.matchWords ?? [])].map((w) => w.trim()).filter((w) => w.length >= 2);
    if (words.some((w) => wordRe(w).test(subject))) out.push(c.name);
  }
  return out;
}

/** The item's categories: the ones assigned to it, then those its subject matches. */
export function effectiveCategories(ev: Colorable, list: Category[]): string[] {
  const own = ev.categories ?? [];
  const extra = matchedCategories(ev.summary, list).filter((n) => !own.some((o) => o.toLowerCase() === n.toLowerCase()));
  return [...own, ...extra];
}

/** All colours of the item's (coloured) categories; else its Google colour; else the calendar's. */
export function itemColors(ev: Colorable, list: Category[]): string[] {
  const cols = effectiveCategories(ev, list)
    .map((n) => categoryColor(n, list))
    .filter((h): h is string => !!h);
  if (cols.length) return [...new Set(cols)];
  if (ev.colorId && GOOGLE_COLORS[ev.colorId]) return [GOOGLE_COLORS[ev.colorId]];
  return [ev.calendarColor];
}

/** Outlook colours an item by its first (coloured) category; else its Google colour; else the calendar's. */
export function itemColor(ev: Colorable, list: Category[]): string {
  return itemColors(ev, list)[0];
}

export function shortcutCategory(key: string, list: Category[]) {
  return list.find((c) => c.shortcut.toLowerCase() === key.toLowerCase())?.name;
}

export function toggled(current: string[], name: string): string[] {
  return current.some((c) => c.toLowerCase() === name.toLowerCase()) ? current.filter((c) => c.toLowerCase() !== name.toLowerCase()) : [...current, name];
}

/** "Categorize ▸" items. `apply` receives the item's new category list. */
export function categorizeMenu(current: string[], list: Category[], apply: (names: string[]) => void): MenuItem[] {
  const has = (n: string) => current.some((c) => c.toLowerCase() === n.toLowerCase());
  return [
    { label: "Clear All Categories", disabled: current.length === 0, onClick: () => apply([]) },
    { type: "separator" },
    ...list.map((c): MenuItem => ({
      label: c.name,
      icon: `cm-cat-${c.color}`,
      checked: has(c.name),
      shortcut: c.shortcut ? `Ctrl+${c.shortcut}` : undefined,
      onClick: () => apply(toggled(current, c.name)),
    })),
    { type: "separator" },
    {
      label: "All Categories…",
      onClick: async () => {
        const r = await openColorCategories(current);
        if (r) apply(r);
      },
    },
  ];
}

// ---- the Color Categories dialog -----------------------------------------------------------

type Req = { selected: string[]; forItem: boolean; resolve: (v: string[] | null) => void };
const useDlg = create<{ req: Req | null; set: (r: Req | null) => void; hosts: number[] }>((set) => ({ req: null, set: (req) => set({ req }), hosts: [] }));
let lastHost = 0;

/** Open "Color Categories". With `selected` (an item's categories) the checkboxes assign them. */
export function openColorCategories(selected: string[] | null): Promise<string[] | null> {
  return new Promise((resolve) => useDlg.getState().set({ selected: selected ?? [], forItem: selected !== null, resolve }));
}

export function ColorCategoriesHost() {
  // the Calendar view and the To-Do Bar can both be mounted: only the first host shows the dialog
  const [id] = useState(() => ++lastHost);
  useEffect(() => {
    useDlg.setState((s) => ({ hosts: [...s.hosts, id] }));
    return () => useDlg.setState((s) => ({ hosts: s.hosts.filter((h) => h !== id) }));
  }, [id]);
  const req = useDlg((s) => (s.hosts[0] === id ? s.req : null));
  if (!req) return null;
  const done = (v: string[] | null) => {
    useDlg.getState().set(null);
    req.resolve(v);
  };
  return <ColorCategoriesDialog req={req} done={done} />;
}

interface Row extends Category {
  /** name when the dialog opened (for renames), "" for new rows */
  orig: string;
  checked: boolean;
  /** on the item but not in the master list */
  extra?: boolean;
}

function ColorCategoriesDialog({ req, done }: { req: Req; done: (v: string[] | null) => void }) {
  const list = useCategories();
  const qc = useQueryClient();
  const has = (n: string) => req.selected.some((c) => c.toLowerCase() === n.toLowerCase());
  const [rows, setRows] = useState<Row[]>(() => [
    ...list.map((c) => ({ ...c, orig: c.name, checked: has(c.name) })),
    // categories on the item that aren't in the master list (Outlook shows them as "not in Master Category List")
    ...req.selected.filter((n) => !findCategory(n, list)).map((n) => ({ name: n, color: 0, shortcut: "", autoMatch: true, matchWords: [], orig: "", checked: true, extra: true })),
  ]);
  const [sel, setSel] = useState(0);
  const [editing, setEditing] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const cur = rows[sel];
  const upd = (i: number, p: Partial<Row>) => setRows((r) => r.map((x, j) => (j === i ? { ...x, ...p } : p.shortcut && x.shortcut === p.shortcut ? { ...x, shortcut: "" } : x)));

  const add = () => {
    let k = 1;
    while (rows.some((r) => r.name.toLowerCase() === `new category${k > 1 ? ` ${k}` : ""}`)) k++;
    const name = `New Category${k > 1 ? ` ${k}` : ""}`;
    const used = new Set(rows.map((r) => r.color));
    const color = PALETTE.find((p) => !used.has(p[0]))?.[0] ?? 1;
    setRows((r) => [...r, { name, color, shortcut: "", autoMatch: true, matchWords: [], orig: "", checked: req.forItem }]);
    setSel(rows.length);
    setEditing(rows.length);
  };
  const remove = async () => {
    if (!cur) return;
    if (
      !(await confirmBox(
        `Are you sure you want to delete the "${cur.name}" category? Deleting this category will not remove it from items that are already assigned to it.`,
        "ClassicMail",
        "warning",
        "calendar.deleteCategory",
      ))
    )
      return;
    setRows((r) => r.filter((_, j) => j !== sel));
    setSel(Math.max(0, sel - 1));
  };
  const ok = async () => {
    const names = new Set<string>();
    for (const r of rows) {
      const n = r.name.replace(/[,;]/g, " ").trim();
      if (!n) return errorBox("Enter a name for the category.");
      if (names.has(n.toLowerCase())) return errorBox(`The name "${n}" is already in use. Type a different name.`);
      names.add(n.toLowerCase());
    }
    setBusy(true);
    try {
      const renames = rows.filter((r) => r.orig && r.orig !== r.name.trim()).map((r) => [r.orig, r.name.trim()] as [string, string]);
      await cal.saveCategories(
        rows
          .filter((r) => !r.extra || r.color || r.shortcut)
          .map((r) => ({
            name: r.name.trim(),
            color: r.color,
            shortcut: r.shortcut,
            autoMatch: r.autoMatch !== false,
            matchWords: (r.matchWords ?? []).map((w) => w.trim()).filter(Boolean),
          })),
        renames,
      );
      await qc.invalidateQueries({ queryKey: ["calendar", "categories"] });
      done(req.forItem ? rows.filter((r) => r.checked).map((r) => r.name.trim()) : []);
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title="Color Categories"
      width={470}
      onClose={() => done(null)}
      footer={
        <>
          <button className="cm-button default" onClick={ok} disabled={busy}>
            OK
          </button>
          <button className="cm-button" onClick={() => done(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-cal-catdlg">
        <div className="cm-cal-catdlg-hint">
          {req.forItem
            ? "To assign Color Categories to the currently selected items, use the checkboxes next to each category. To edit a category, select the category name and use the commands to the right."
            : "Select a category and use the commands to the right to edit it."}
        </div>
        <div className="cm-cal-catdlg-main">
          <div className="cm-cal-catlist" role="listbox" tabIndex={0}>
            {rows.map((r, i) => (
              <div key={i} className={`cm-cal-catrow${i === sel ? " sel" : ""}`} onMouseDown={() => setSel(i)} onDoubleClick={() => setEditing(i)}>
                {req.forItem && <input type="checkbox" checked={r.checked} onChange={(e) => upd(i, { checked: e.target.checked })} />}
                <span className="cm-cal-catswatch" style={{ background: presetHex(r.color) ?? "#fff" }} />
                {editing === i ? (
                  <input
                    className="cm-input cm-fill"
                    autoFocus
                    value={r.name}
                    onChange={(e) => upd(i, { name: e.target.value })}
                    onBlur={() => setEditing(null)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === "Escape") {
                        e.preventDefault();
                        e.stopPropagation();
                        setEditing(null);
                      }
                    }}
                  />
                ) : (
                  <span className="cm-fill cm-ellipsis">
                    {r.name}
                    {r.extra && r.color === 0 ? <span className="cm-muted"> (not in Master Category List)</span> : null}
                  </span>
                )}
                {r.shortcut && <span className="cm-muted">Ctrl+{r.shortcut}</span>}
              </div>
            ))}
          </div>
          <div className="cm-cal-catbtns">
            <button className="cm-button" onClick={add}>
              New...
            </button>
            <button className="cm-button" disabled={!cur} onClick={() => setEditing(sel)}>
              Rename
            </button>
            <button className="cm-button" disabled={!cur} onClick={remove}>
              Delete
            </button>
            <label className="cm-cal-catfield">
              Color:
              <select
                className="cm-input"
                value={cur?.color ?? 0}
                disabled={!cur}
                onChange={(e) => upd(sel, { color: Number(e.target.value) })}
                style={cur ? { background: presetHex(cur.color) ?? undefined } : undefined}
              >
                <option value={0}>None</option>
                {PALETTE.map(([p, n, hex]) => (
                  <option key={p} value={p} style={{ background: hex }}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <label className="cm-cal-catfield" title="Calendar items whose subject contains this category's name are coloured with it automatically">
              <span style={{ display: "flex", alignItems: "center", gap: 4 }}>
                <input type="checkbox" disabled={!cur} checked={cur ? cur.autoMatch !== false : false} onChange={(e) => upd(sel, { autoMatch: e.target.checked })} />
                Color items whose subject contains the name
              </span>
            </label>
            <label className="cm-cal-catfield" title="Other words that also match, separated by commas — e.g. ACMX, Acme Ltd">
              Also match:
              <input
                className="cm-input"
                disabled={!cur || cur.autoMatch === false}
                placeholder="e.g. ACMX, Acme Ltd"
                value={(cur?.matchWords ?? []).join(", ")}
                onChange={(e) => upd(sel, { matchWords: e.target.value.split(",").map((w) => w.replace(/^\s+/, "")) })}
              />
            </label>
            <label className="cm-cal-catfield">
              Shortcut Key:
              <select className="cm-input" value={cur?.shortcut ?? ""} disabled={!cur} onChange={(e) => upd(sel, { shortcut: e.target.value })}>
                <option value="">(None)</option>
                {SHORTCUTS.map((k) => (
                  <option key={k} value={k}>
                    CTRL+{k}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>
      </div>
    </Dialog>
  );
}
