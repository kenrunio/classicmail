// The message list: virtualised, paged from the back end, Outlook 2007 arrangements with
// collapsible group headers, two-line rows (Reading Pane on the right) or a column grid, and
// Show as Conversations (one row per thread, expandable to the thread's messages in every folder).
import { createContext, memo, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { mailApi, type Arrange, type ListParams, type ListResult, type MessageRow } from "../../api/mail";
import { Icon, keyOf, messageBox, showMenu, contextMenu, type MenuItem } from "../../ui";
import { COLUMNS, colWidth, useColumns, visibleColumns, type ColDef, type ColId, type ListLayout } from "./columns";
import { useAccounts } from "./useAccounts";
import { useUiFontSize } from "../../ui/font";
import { conversationsOn, folderKey as folderKeyOf, useMailSel, useMailUi, type ArrangeState } from "./store";
import { columnDate, listDate, listSize } from "./format";
import { rowIcon } from "./icons";
import { followUpMenu, markRead, messageMenu, openItem, toggleFlag } from "./actions";
import { buildLines, childKey, expandable, PAGE, resolveKey, resolveSelection, rowFlagged, rowKey, type Line } from "./conversations";

export const ARRANGE_LABEL: Record<Arrange, string> = {
  date: "Date",
  conversation: "Conversation",
  from: "From",
  to: "To",
  subject: "Subject",
  size: "Size",
  flag: "Flag",
  attachments: "Attachments",
  importance: "Importance",
};

const ORDER_LABEL: Record<Arrange, [string, string]> = {
  date: ["Newest on top", "Oldest on top"],
  conversation: ["Newest on top", "Oldest on top"],
  from: ["Z on top", "A on top"],
  to: ["Z on top", "A on top"],
  subject: ["Z on top", "A on top"],
  size: ["Largest on top", "Smallest on top"],
  flag: ["Flagged on top", "Unflagged on top"],
  attachments: ["With attachments on top", "Without attachments on top"],
  importance: ["High on top", "Low on top"],
};

export const DEFAULT_DESC: Record<Arrange, boolean> = {
  date: true,
  conversation: true,
  from: false,
  to: false,
  subject: false,
  size: true,
  flag: true,
  attachments: true,
  importance: true,
};

export function arrangeMenu(a: ArrangeState, set: (p: Partial<ArrangeState>) => void): MenuItem[] {
  const item = (k: Arrange): MenuItem => ({ label: ARRANGE_LABEL[k], radio: true, checked: a.arrange === k, onClick: () => set({ arrange: k, desc: DEFAULT_DESC[k] }) });
  return [
    item("date"),
    item("conversation"),
    item("from"),
    item("to"),
    item("size"),
    item("subject"),
    item("attachments"),
    item("importance"),
    item("flag"),
    { type: "separator" },
    { label: "Show in Groups", checked: a.groups, onClick: () => set({ groups: !a.groups }) },
    { label: "Reverse Sort", onClick: () => set({ desc: !a.desc }) },
  ];
}

/** View › Message List: one line with columns, Outlook's two lines, or automatic. */
export function layoutMenu(): MenuItem[] {
  const { layout, setLayout } = useColumns.getState();
  const item = (l: ListLayout, label: string): MenuItem => ({ label, radio: true, checked: layout === l, onClick: () => setLayout(l) });
  return [item("auto", "Automatic (two lines with the Reading Pane on the right)"), item("singleLine", "Single line, in columns"), item("twoLine", "Two lines")];
}

/**
 * View › Show as Conversations. Like Outlook, turning it on or off asks whether it applies to
 * this folder or to all mailboxes.
 */
export async function toggleConversations() {
  const ui = useMailUi.getState();
  const key = folderKeyOf(ui.folder);
  const on = !conversationsOn(ui, key);
  const answer = await messageBox({
    title: "ClassicMail",
    text: on
      ? "Do you want to show messages arranged by conversations in all mailboxes or just this folder?"
      : "Do you want to stop showing messages arranged by conversations in all mailboxes or just this folder?",
    icon: "help",
    buttons: ["All mailboxes", "This folder", "Cancel"],
    defaultButton: "All mailboxes",
  });
  if (answer === "All mailboxes") useMailUi.getState().setConversations(key, on, "all");
  else if (answer === "This folder") useMailUi.getState().setConversations(key, on, "folder");
}

export function conversationsMenuItem(): MenuItem {
  const ui = useMailUi.getState();
  return { label: "Show as Conversations", checked: conversationsOn(ui, folderKeyOf(ui.folder)), onClick: () => void toggleConversations() };
}

/** The list's right-click menu on empty space / the arrangement bar. */
function viewMenu(a: ArrangeState, set: (p: Partial<ArrangeState>) => void): MenuItem[] {
  return [...arrangeMenu(a, set), { type: "separator" }, conversationsMenuItem(), { label: "Message List", submenu: layoutMenu() }];
}

/** Right-click on the column header: the column chooser + arrangement. */
function columnMenu(arrange: ArrangeState, setArrange: (p: Partial<ArrangeState>) => void): MenuItem[] {
  const { order, toggle, reset } = useColumns.getState();
  const choose = (Object.keys(COLUMNS) as ColId[]).map((id): MenuItem => ({
    label: COLUMNS[id].title,
    checked: order.includes(id),
    disabled: id === "subject",
    onClick: () => toggle(id),
  }));
  return [
    { type: "header", label: "Columns" },
    ...choose,
    { label: "Reset Columns", onClick: reset },
    { type: "separator" },
    { label: "Arrange By", icon: "sort", submenu: arrangeMenu(arrange, setArrange) },
    conversationsMenuItem(),
    { label: "Message List", submenu: layoutMenu() },
  ];
}

export interface ListHandle {
  /** rows of the list (conversations when they're shown) */
  total: number;
  /** messages in the view */
  messages: number;
  unread: number;
  /** loaded rows by message id (conversation headers under their newest message's id) */
  rowsById: Map<number, MessageRow>;
  /** loaded rows by selection key ("m:", "c:", "t:") */
  rowsByKey: Map<string, MessageRow>;
}

/**
 * Where rows come from when it isn't the mailboxes (Outlook archives): the list query, what
 * double-click / Enter and right-click do. Such lists are read-only (no drag, no flag clicks).
 */
export interface ListBackend {
  /** query-key domain (its `<domain>.changed` event refreshes the list) */
  domain: string;
  list: (p: ListParams & Record<string, unknown>) => Promise<ListResult>;
  open: (r: MessageRow) => void;
  menu: (rows: MessageRow[]) => MenuItem[];
}

/** read-only list (no flag toggling) */
const ReadOnlyList = createContext(false);

/** messages only Gmail's search found (Instant Search also asks Gmail): tagged "Gmail" in the list */
const GmailFound = createContext<ReadonlySet<number> | null>(null);

function GmailTag({ r }: { r: MessageRow }) {
  const found = useContext(GmailFound);
  if (!found || !(found.has(r.id) || r.conv?.ids.some((id) => found.has(id)))) return null;
  return (
    <span
      className="cm-mail-gmailtag"
      title="Found by Gmail's search, not by the search on this computer: the words are in a part of the message that isn't downloaded yet, or in an attachment"
    >
      Gmail
    </span>
  );
}

export interface MessageListProps {
  params: Omit<ListParams, "offset" | "limit" | "withGroups">;
  arrange: ArrangeState;
  setArrange: (p: Partial<ArrangeState>) => void;
  compact: boolean;
  folderKey: string;
  folderRole?: string;
  fromFolderId?: number;
  onData?: (h: ListHandle) => void;
  emptyText?: string;
  backend?: ListBackend;
  /** tag these messages "Gmail" (found only by Gmail's search) */
  gmailFound?: ReadonlySet<number> | null;
}

const NO_GROUPS: string[] = [];

/** Expanded conversations per folder (kept while the app runs, like Outlook). */
const expandedByFolder = new Map<string, Set<string>>();

type Mods = { ctrlKey?: boolean; shiftKey?: boolean };

// memo: the list doesn't redraw when only its surroundings do (a splitter drag, Instant Search typing)
export const MessageList = memo(function MessageList({
  params,
  arrange,
  setArrange,
  compact,
  folderKey,
  folderRole,
  fromFolderId,
  onData,
  emptyText,
  backend,
  gmailFound,
}: MessageListProps) {
  const listFn = backend?.list ?? mailApi.list;
  const domain = backend?.domain ?? "mail";
  const openRow = backend?.open ?? openItem;
  const toggleGroup = useMailUi((s) => s.toggleGroup);
  const sel = useMailSel();
  const colState = useColumns();
  const accounts = useAccounts();
  const acctName = useMemo(() => new Map(accounts.map((a) => [a.id, a.email])), [accounts]);
  const conv = !!params.conversations;
  const baseKey = useMemo(() => JSON.stringify(params), [params]);
  // the same view without Gmail's extra search results (alsoIds): when those arrive, the current
  // rows stay on screen until the merged list is in, and the keyboard position is kept
  const localKey = useMemo(() => JSON.stringify({ ...params, alsoIds: undefined }), [params]);
  const keepWhileMerging = <T,>(prev: T | undefined, prevQuery?: { queryKey: readonly unknown[] }): T | undefined => {
    const k = prevQuery?.queryKey[2];
    if (prev === undefined || typeof k !== "string") return undefined;
    try {
      const o = JSON.parse(k);
      delete o.alsoIds;
      return JSON.stringify(o) === localKey ? prev : undefined;
    } catch {
      return undefined;
    }
  };
  const viewKey = useMemo(() => JSON.stringify({ source: params.source, search: params.search, scope: params.scope }), [params.source, params.search, params.scope]);
  const meta = useQuery({
    queryKey: [domain, "list", baseKey, "meta", arrange.groups],
    queryFn: () => listFn({ ...params, offset: 0, limit: 0, withGroups: arrange.groups }),
    staleTime: 10_000,
    placeholderData: keepWhileMerging,
  });
  const total = meta.data?.total ?? 0;
  const collapsedGroups = useMailUi((s) => s.collapsed[folderKey]) ?? NO_GROUPS;
  const grouped = arrange.groups && !!meta.data?.groups;

  // expanded conversations (keys "<accountId>:<threadId>")
  const [expanded, setExpandedState] = useState<Set<string>>(() => expandedByFolder.get(folderKey) ?? new Set());
  useEffect(() => setExpandedState(expandedByFolder.get(folderKey) ?? new Set()), [folderKey]);
  const setExpanded = useCallback(
    (f: (s: Set<string>) => Set<string>) =>
      setExpandedState((cur) => {
        const next = f(cur);
        expandedByFolder.set(folderKey, next);
        return next;
      }),
    [folderKey],
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  // like Outlook, the message list has the keyboard when the window opens (nothing else focused)
  useEffect(() => {
    const t = setTimeout(() => {
      if (!document.activeElement || document.activeElement === document.body) scrollRef.current?.focus({ preventScroll: true });
    }, 0);
    return () => clearTimeout(t);
  }, []);

  // pages of rows seen so far (for selection / keyboard over loaded rows)
  const loaded = useRef(new Map<number, MessageRow[]>());
  const lastKey = useRef("");
  const lastLocalKey = useRef("");
  const focusLine = useRef(-1);
  if (lastKey.current !== baseKey) {
    loaded.current = new Map();
    lastKey.current = baseKey;
    if (lastLocalKey.current !== localKey) focusLine.current = -1;
    lastLocalKey.current = localKey;
  }
  const rowAt = (i: number): MessageRow | undefined => loaded.current.get(Math.floor(i / PAGE))?.[i % PAGE];

  // the visible pages depend on the lines, which depend on the expanded rows' children: the
  // lines are built from what's loaded, the virtualiser asks for pages, and children follow
  const [pageList, setPageList] = useState<number[]>([0]);
  const pageQs = useQueries({
    queries: pageList.map((p) => ({
      queryKey: [domain, "list", baseKey, "page", p],
      queryFn: () => listFn({ ...params, offset: p * PAGE, limit: PAGE }),
      staleTime: 10_000,
      placeholderData: keepWhileMerging,
    })),
  });
  pageList.forEach((p, i) => {
    const d = pageQs[i]?.data;
    if (d) loaded.current.set(p, d.rows);
  });
  const pagesSig = `${baseKey}|${pageQs.map((q) => q.dataUpdatedAt).join(",")}`;

  // children of expanded conversations
  const expandedRows = useMemo(() => {
    const out: { index: number; r: MessageRow }[] = [];
    if (!conv || expanded.size === 0) return out;
    for (const [p, rows] of loaded.current)
      rows.forEach((r, i) => {
        if (r.conv && expanded.has(r.conv.key) && expandable(r)) out.push({ index: p * PAGE + i, r });
      });
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pagesSig, expanded, conv]);
  const viewParams = useMemo(() => JSON.parse(viewKey) as Pick<ListParams, "source" | "search" | "scope">, [viewKey]);
  const kidQs = useQueries({
    queries: expandedRows.map(({ r }) => ({
      queryKey: ["mail", "thread", r.accountId, r.threadId, viewKey],
      queryFn: () => mailApi.listThread(r.accountId, r.threadId, viewParams),
      staleTime: 10_000,
    })),
  });
  const kidsSig = kidQs.map((q) => q.dataUpdatedAt).join(",");
  const kids = useMemo(() => {
    const m = new Map<number, MessageRow[] | null>();
    expandedRows.forEach(({ index }, k) => m.set(index, kidQs[k]?.data ?? null));
    return m;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expandedRows, kidsSig]);
  const childCounts = useMemo(() => {
    const m = new Map<number, number>();
    expandedRows.forEach(({ index, r }) => m.set(index, kids.get(index)?.length ?? r.conv!.total));
    return m;
  }, [expandedRows, kids]);

  const lines: Line[] = useMemo(
    () => buildLines(total, grouped ? meta.data!.groups : null, collapsedGroups, childCounts),
    [total, grouped, meta.data, collapsedGroups, childCounts],
  );
  const kidAt = (l: Line): MessageRow | undefined => (l.kind === "child" ? (kids.get(l.index) ?? undefined)?.[l.ci] : undefined);
  const rowOfLine = (li: number): MessageRow | undefined => {
    const l = lines[li];
    if (!l || l.kind === "group") return undefined;
    return l.kind === "row" ? rowAt(l.index) : kidAt(l);
  };
  const keyOfLine = (li: number): string | undefined => {
    const l = lines[li];
    const r = rowOfLine(li);
    if (!r || !l) return undefined;
    return l.kind === "child" ? childKey(r) : rowKey(r);
  };

  // row heights follow the UI font size (Options › Appearance): 13px → 36 / 20
  const fontPx = useUiFontSize();
  const rowH = compact ? Math.round(fontPx * 2.75) : Math.max(19, Math.round(fontPx * 1.55));
  const virt = useVirtualizer({
    count: lines.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (lines[i]?.kind === "group" ? Math.max(20, Math.round(fontPx * 1.65)) : lines[i]?.kind === "child" && compact ? rowH - 2 : rowH),
    overscan: 12,
  });
  // the virtualiser re-reads estimateSize only when the count changes: tell it when sizes do
  useEffect(() => virt.measure(), [compact, grouped, rowH, fontPx, virt]);
  const vis = virt.getVirtualItems();
  const wantPages = useMemo(() => {
    const s = new Set<number>();
    for (const v of vis) {
      const l = lines[v.index];
      if (l && l.kind !== "group") s.add(Math.floor(l.index / PAGE));
    }
    if (s.size === 0 && total > 0) s.add(0);
    return [...s].sort((a, b) => a - b);
  }, [vis, lines, total]);
  const wantSig = wantPages.join(",");
  useEffect(() => {
    setPageList((cur) => (cur.join(",") === wantSig ? cur : wantPages));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wantSig]);

  // lookups over what's loaded
  const { rowsById, rowsByKey, lineOfKey } = useMemo(() => {
    const byId = new Map<number, MessageRow>();
    const byKey = new Map<string, MessageRow>();
    for (const rows of loaded.current.values())
      for (const r of rows) {
        byId.set(r.id, r);
        byKey.set(rowKey(r), r);
      }
    for (const ks of kids.values())
      for (const k of ks ?? []) {
        byKey.set(childKey(k), k);
        if (!byId.has(k.id)) byId.set(k.id, k);
      }
    const lok = new Map<string, number>();
    lines.forEach((l, li) => {
      if (l.kind === "group") return;
      const r = l.kind === "row" ? rowAt(l.index) : kidAt(l);
      if (r) lok.set(l.kind === "row" ? rowKey(r) : childKey(r), li);
    });
    return { rowsById: byId, rowsByKey: byKey, lineOfKey: lok };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pagesSig, kids, lines]);
  const live = useRef({ rowsByKey, lineOfKey, lines });
  live.current = { rowsByKey, lineOfKey, lines };

  useEffect(() => {
    if (!meta.data) return; // still loading: keep the last counts in the status bar (no "Items: 0" flash)
    onData?.({ total, messages: meta.data.messages ?? total, unread: meta.data.unread ?? 0, rowsById, rowsByKey });
  }, [total, meta.data, rowsById, rowsByKey, onData]);

  /** set the selection; `extra` = rows not in the lookups yet (just loaded) */
  const commit = (keys: string[], focusKey: string | null, anchorKey?: string | null, extra?: Map<string, MessageRow>) => {
    const byKey = extra ? new Map([...live.current.rowsByKey, ...extra]) : live.current.rowsByKey;
    useMailSel.getState().select({ keys, focusKey, anchorKey, ...resolveSelection(keys, focusKey, byKey) });
  };

  // rows changed (new mail in a conversation, a refresh): re-resolve what the selected keys mean
  useEffect(() => {
    const st = useMailSel.getState();
    if (!st.keys.length) return;
    const r = resolveSelection(st.keys, st.focusKey, rowsByKey);
    if (r.focused !== st.focused || r.selected.length !== st.selected.length || r.selected.some((x, i) => x !== st.selected[i])) {
      if (r.selected.length || !st.keys.some((k) => k.startsWith("c:"))) st.select({ keys: st.keys, focusKey: st.focusKey, ...r });
    }
  }, [rowsByKey]);

  // the focused row vanished (deleted / moved): select the row now at its place, like Outlook
  useEffect(() => {
    const st = useMailSel.getState();
    const li = st.focusKey ? lineOfKey.get(st.focusKey) : undefined;
    if (li !== undefined) {
      focusLine.current = li;
      return;
    }
    const busy = pageQs.some((q) => q.isFetching) || kidQs.some((q) => q.isFetching);
    if (st.focusKey !== null && focusLine.current >= 0 && lines.length > 0 && !busy) {
      const start = Math.min(focusLine.current, lines.length - 1);
      for (let k = start, dir = 1; k >= 0; k += dir) {
        if (k >= lines.length) {
          k = start;
          dir = -1;
          continue;
        }
        const key = keyOfLine(k);
        if (key) {
          commit([key], key, key);
          focusLine.current = k;
          return;
        }
        if (lines[k]?.kind !== "group" && !rowOfLine(k)) return; // not loaded yet
      }
      commit([], null, null);
    } else if (st.focusKey !== null && total === 0 && meta.data && !busy) commit([], null, null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lineOfKey, total]);

  // new folder / search / arrangement → keep the selected item if it's in the new view, else the first
  const initFor = useRef("");
  useEffect(() => {
    if (initFor.current === baseKey) return;
    const firstLine = lines.findIndex((l) => l.kind !== "group");
    const first = firstLine >= 0 ? keyOfLine(firstLine) : undefined;
    if (first) {
      initFor.current = baseKey;
      const st = useMailSel.getState();
      let li = st.focusKey ? lineOfKey.get(st.focusKey) : undefined;
      if (li === undefined && st.focused !== null) {
        // conversations switched on / off: the row (or conversation) holding the focused message
        for (const [k, r] of rowsByKey)
          if ((r.id === st.focused && !r.conv) || r.conv?.ids.includes(st.focused)) {
            li = lineOfKey.get(k);
            if (li !== undefined) break;
          }
      }
      if (li !== undefined) {
        const k = keyOfLine(li)!;
        if (st.focusKey !== k) commit([k], k, k);
        focusLine.current = li;
        virt.scrollToIndex(li, { align: "center" });
      } else {
        commit([first], first, first);
        focusLine.current = firstLine;
        virt.scrollToIndex(0);
      }
    } else if (meta.data && total === 0) {
      initFor.current = baseKey;
      commit([], null, null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseKey, lineOfKey, meta.data]);

  const selSet = useMemo(() => new Set(sel.keys), [sel.keys]);

  // rows far from what's on screen (End, Page Down, Shift+click over thousands of items) may not be
  // loaded yet: fetch their pages first (same query keys as the visible pages, so they're cached)
  const qc = useQueryClient();
  const ensureLines = async (lo: number, hi: number) => {
    const key = baseKey;
    const ls = live.current.lines;
    let a = lo;
    while (a <= hi && ls[a]?.kind === "group") a++;
    let b = hi;
    while (b >= a && ls[b]?.kind === "group") b--;
    if (a > b) return true;
    const ra = (ls[a] as Exclude<Line, { kind: "group" }>).index;
    const rb = (ls[b] as Exclude<Line, { kind: "group" }>).index;
    for (let p = Math.floor(ra / PAGE); p <= Math.floor(rb / PAGE); p++) {
      if (loaded.current.has(p)) continue;
      const d = await qc.fetchQuery({
        queryKey: [domain, "list", key, "page", p],
        queryFn: () => listFn({ ...params, offset: p * PAGE, limit: PAGE }),
        staleTime: 10_000,
      });
      if (lastKey.current !== key) return false;
      loaded.current.set(p, d.rows);
    }
    return true;
  };

  const selectLine = async (li: number, e?: Mods) => {
    const st0 = useMailSel.getState();
    const anchorLi = e?.shiftKey && st0.anchorKey ? live.current.lineOfKey.get(st0.anchorKey) : undefined;
    const [lo, hi] = anchorLi !== undefined ? [Math.min(anchorLi, li), Math.max(anchorLi, li)] : [li, li];
    if (!(await ensureLines(lo, hi))) return;
    const key = keyOfLine(li);
    if (!key) return;
    const st = useMailSel.getState();
    const extra = new Map<string, MessageRow>();
    if (anchorLi !== undefined) {
      const keys: string[] = [];
      for (let k = lo; k <= hi; k++) {
        const kk = keyOfLine(k);
        const r = rowOfLine(k);
        if (kk && r) {
          keys.push(kk);
          extra.set(kk, r);
        }
      }
      commit(e?.ctrlKey ? [...new Set([...st.keys, ...keys])] : keys, key, st.anchorKey, extra);
    } else {
      const r = rowOfLine(li);
      if (r) extra.set(key, r);
      if (e?.ctrlKey) {
        const has = st.keys.includes(key);
        commit(has ? st.keys.filter((x) => x !== key) : [...st.keys, key], key, key, extra);
      } else commit([key], key, key, extra);
    }
    focusLine.current = li;
  };

  /** expand / collapse a conversation (by its row's line); collapsing moves a selected child's focus to the header */
  const setConvOpen = (li: number, open: boolean) => {
    const r = rowOfLine(li);
    if (!r?.conv || !expandable(r)) return false;
    const k = r.conv.key;
    if (expanded.has(k) === open) return false;
    setExpanded((s) => {
      const n = new Set(s);
      if (open) n.add(k);
      else n.delete(k);
      return n;
    });
    if (!open) {
      const st = useMailSel.getState();
      const kidKeys = new Set((kids.get((lines[li] as { index: number }).index) ?? []).map(childKey));
      if (st.focusKey && kidKeys.has(st.focusKey)) {
        const hk = rowKey(r);
        commit([hk], hk, hk);
        focusLine.current = li;
      }
    }
    return true;
  };
  const toggleRef = useRef((li: number) => void li);
  toggleRef.current = (li: number) => {
    const r = rowOfLine(li);
    if (r?.conv) setConvOpen(li, !expanded.has(r.conv.key));
  };
  const onToggle = useCallback((li: number) => toggleRef.current(li), []);

  const currentLine = () => {
    const st = useMailSel.getState();
    return st.focusKey ? (lineOfKey.get(st.focusKey) ?? -1) : -1;
  };

  const moveFocus = (delta: number, e: React.KeyboardEvent) => {
    const step = delta > 0 ? 1 : -1;
    let remaining = Math.abs(delta);
    let target = currentLine();
    while (remaining > 0) {
      let n = target + step;
      while (n >= 0 && n < lines.length && lines[n]?.kind === "group") n += step;
      if (n < 0 || n >= lines.length) break;
      target = n;
      remaining--;
    }
    if (target < 0 || lines[target]?.kind === "group") return;
    selectLine(target, { shiftKey: e.shiftKey, ctrlKey: false });
    virt.scrollToIndex(target, { align: "auto" });
  };

  /** Left / Right / + / -: conversations first, then the group of the focused row */
  const expandKey = (open: boolean, arrow: boolean) => {
    const li = currentLine();
    const l = lines[li];
    if (!l) return;
    if (conv && l.kind === "row") {
      const r = rowAt(l.index);
      if (expandable(r)) {
        const isOpen = expanded.has(r!.conv!.key);
        if (open && isOpen && arrow) {
          // Right on an open conversation: to its newest message
          if (lines[li + 1]?.kind === "child") {
            selectLine(li + 1);
            virt.scrollToIndex(li + 1, { align: "auto" });
          }
          return;
        }
        if (open !== isOpen) {
          setConvOpen(li, open);
          return;
        }
      }
    }
    if (conv && l.kind === "child" && !open) {
      // Left on a message of a conversation: back to (and collapse) the conversation
      for (let k = li - 1; k >= 0; k--)
        if (lines[k]?.kind === "row") {
          setConvOpen(k, false);
          selectLine(k);
          virt.scrollToIndex(k, { align: "auto" });
          return;
        }
    }
    if (!grouped || !arrow) return;
    for (let k = li; k >= 0; k--) {
      const it = lines[k];
      if (it.kind === "group") {
        if (!open !== it.collapsed) toggleGroup(folderKey, it.g.key);
        break;
      }
    }
  };

  const onKeyDown = async (e: React.KeyboardEvent) => {
    const pageRows = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 400) / rowH) - 1);
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        return moveFocus(1, e);
      case "ArrowUp":
        e.preventDefault();
        return moveFocus(-1, e);
      case "PageDown":
        e.preventDefault();
        return moveFocus(pageRows, e);
      case "PageUp":
        e.preventDefault();
        return moveFocus(-pageRows, e);
      case "Home":
        e.preventDefault();
        return moveFocus(-1e9, e);
      case "End":
        e.preventDefault();
        return moveFocus(1e9, e);
      case "Enter": {
        e.preventDefault();
        const st = useMailSel.getState();
        const r = st.focused !== null ? rowsById.get(st.focused) : undefined;
        if (r) openRow(r);
        return;
      }
      // Delete / Insert are window-wide hotkeys (MailView)
      case "ArrowLeft":
      case "ArrowRight":
        e.preventDefault();
        return expandKey(e.key === "ArrowRight", true);
      case "+":
      case "-":
        if (!conv || e.ctrlKey || e.altKey) return;
        e.preventDefault();
        return expandKey(e.key === "+", false);
    }
    if (e.ctrlKey && keyOf(e) === "a") {
      e.preventDefault();
      // the back end pages at most 2,000 rows per call: collect every row in the view
      const keys: string[] = [];
      const extra = new Map<string, MessageRow>();
      const key = baseKey;
      for (let off = 0; off < total; off += 2000) {
        const part = await listFn({ ...params, offset: off, limit: 2000 });
        if (lastKey.current !== key) return;
        for (const r of part.rows) {
          const k = rowKey(r);
          keys.push(k);
          extra.set(k, r);
        }
        if (part.rows.length < 2000) break;
      }
      const st = useMailSel.getState();
      commit(keys, st.focusKey, st.anchorKey, extra);
    }
  };

  const selectedRows = () =>
    useMailSel
      .getState()
      .keys.map((k) => rowsByKey.get(k))
      .filter((r): r is MessageRow => !!r);
  const sentLike = folderRole === "sent" || folderRole === "drafts" || folderRole === "outbox";
  const cross = !!meta.data?.crossFolder;

  const onRowMouseDown = (e: React.MouseEvent, li: number) => {
    const k = keyOfLine(li);
    if (e.button === 2) {
      if (k && !selSet.has(k)) selectLine(li);
      return;
    }
    // keep a multi-selection when starting a drag on it
    if (k && selSet.has(k) && !e.ctrlKey && !e.shiftKey && sel.keys.length > 1) return;
    selectLine(li, e);
  };
  const onRowClick = (e: React.MouseEvent, li: number) => {
    const k = keyOfLine(li);
    if (k && sel.keys.length > 1 && selSet.has(k) && !e.ctrlKey && !e.shiftKey) selectLine(li);
  };
  const onDragStart = (e: React.DragEvent, li: number) => {
    const r = rowOfLine(li);
    const k = keyOfLine(li);
    if (!r || !k || r.outboxId || backend) return e.preventDefault();
    const ids = selSet.has(k) ? useMailSel.getState().selected.filter((x) => x > 0) : resolveKey(k, rowsByKey).ids;
    e.dataTransfer.setData("application/x-cm-items", JSON.stringify({ kind: "mail", ids, fromFolderId }));
    e.dataTransfer.effectAllowed = "copyMove";
  };

  const cols = useMemo(() => visibleColumns(colState.order, cross), [colState.order, cross]);
  const headerBar = compact ? (
    <div className="cm-mail-arrbar" onContextMenu={contextMenu(() => viewMenu(arrange, setArrange))}>
      <span
        className="cm-mail-arrbar-l"
        onMouseDown={(e) => showMenu(e.clientX, (e.currentTarget as HTMLElement).getBoundingClientRect().bottom, arrangeMenu(arrange, setArrange))}
      >
        Arranged By: {ARRANGE_LABEL[arrange.arrange]}
      </span>
      <span className="cm-mail-arrbar-r" onMouseDown={() => setArrange({ desc: !arrange.desc })}>
        {ORDER_LABEL[arrange.arrange][arrange.desc ? 0 : 1]}
        <Icon name={arrange.desc ? "arrow-down" : "arrow-up"} size={10} />
      </span>
    </div>
  ) : (
    <GridHeader arrange={arrange} setArrange={setArrange} sentLike={sentLike} cols={cols} widths={colState.widths} conv={conv} />
  );

  return (
    <ReadOnlyList.Provider value={!!backend}>
      <GmailFound.Provider value={gmailFound ?? null}>
        <div className="cm-mail-list-wrap">
          {headerBar}
          <div
            ref={scrollRef}
            className={`cm-mail-list${compact ? " compact" : " grid"}${conv ? " conv" : ""}`}
            tabIndex={0}
            onKeyDown={onKeyDown}
            onContextMenu={(e) => {
              e.preventDefault();
              if ((e.target as HTMLElement).closest(".cm-mail-row")) {
                const rows = selectedRows();
                if (rows.length) showMenu(e.clientX, e.clientY, (backend?.menu ?? messageMenu)(rows));
              } else showMenu(e.clientX, e.clientY, viewMenu(arrange, setArrange));
            }}
          >
            {meta.data && total === 0 && <div className="cm-mail-list-empty">{emptyText ?? "There are no items to show in this view."}</div>}
            <div style={{ height: virt.getTotalSize(), position: "relative" }}>
              {vis.map((v) => {
                const l = lines[v.index];
                if (!l) return null;
                const style: React.CSSProperties = { position: "absolute", top: 0, left: 0, right: 0, height: v.size, transform: `translateY(${v.start}px)` };
                if (l.kind === "group")
                  return (
                    <div key={`g${l.g.key}`} className="cm-mail-group" style={style} onMouseDown={() => toggleGroup(folderKey, l.g.key)}>
                      <Icon name={l.collapsed ? "tree-collapsed" : "tree-expanded"} size={10} />
                      <span>{l.g.label}</span>
                      {!compact && (
                        <span className="cm-mail-group-count">
                          ({l.g.count.toLocaleString()} {l.g.count === 1 ? "item" : "items"})
                        </span>
                      )}
                    </div>
                  );
                const r = rowOfLine(v.index);
                const k = r ? (l.kind === "child" ? childKey(r) : rowKey(r)) : undefined;
                const exp: Exp = l.kind === "child" ? "child" : !conv ? "none" : expandable(r) ? (expanded.has(r!.conv!.key) ? "open" : "closed") : "leaf";
                const selected = !!k && selSet.has(k);
                const focused = !!k && sel.focusKey === k;
                return (
                  <div
                    key={k ?? `i${v.index}`}
                    style={style}
                    draggable={!!r && !backend}
                    onDragStart={(e) => onDragStart(e, v.index)}
                    onMouseDown={(e) => onRowMouseDown(e, v.index)}
                    onClick={(e) => onRowClick(e, v.index)}
                    onDoubleClick={() => r && openRow(r)}
                  >
                    {r ? (
                      compact ? (
                        <CompactRow r={r} selected={selected} focused={focused} sentLike={sentLike} exp={exp} line={v.index} onToggle={onToggle} />
                      ) : (
                        <GridRow
                          r={r}
                          selected={selected}
                          focused={focused}
                          sentLike={sentLike}
                          cols={cols}
                          widths={colState.widths}
                          account={acctName.get(r.accountId) ?? ""}
                          exp={exp}
                          line={v.index}
                          onToggle={onToggle}
                        />
                      )
                    ) : (
                      <div className="cm-mail-row loading" />
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      </GmailFound.Provider>
    </ReadOnlyList.Provider>
  );
});

/** Row kind for drawing: a plain row (conversations off), a conversation that expands / is open,
 * a single-message conversation, or a message inside an open conversation. */
type Exp = "none" | "closed" | "open" | "leaf" | "child";

/** Outlook's conversation arrow (▷ closed, ◢ open). Clicking it doesn't change the selection. */
function Twisty({ exp, line, onToggle }: { exp: Exp; line: number; onToggle: (li: number) => void }) {
  if (exp !== "closed" && exp !== "open") return <span className="cm-mail-twisty" />;
  return (
    <span
      className={`cm-mail-twisty ${exp}`}
      title={exp === "open" ? "Collapse conversation (Left)" : "Expand conversation (Right)"}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        e.stopPropagation();
        onToggle(line);
      }}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <svg width="9" height="9" viewBox="0 0 9 9" aria-hidden>
        {exp === "open" ? (
          <path d="M8 1.5 8 8 1.5 8z" fill="currentColor" />
        ) : (
          <path d="M2.5 1 7 4.5 2.5 8z" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round" />
        )}
      </svg>
    </span>
  );
}

function FlagCell({ r }: { r: MessageRow }) {
  const c = r.conv;
  const readOnly = useContext(ReadOnlyList);
  const flagged = rowFlagged(r);
  const done = !flagged && (c ? c.flag === 1 : r.flagStatus === "complete");
  if (readOnly)
    return (
      <span className={`cm-mail-flag${flagged ? " on" : ""}${done ? " done" : ""}`} title={flagged ? "Follow up" : done ? "Completed" : ""}>
        {(flagged || done) && <Icon name={flagged ? "flag" : "flag-complete"} size={14} />}
      </span>
    );
  return (
    <span
      className={`cm-mail-flag${flagged ? " on" : ""}${done ? " done" : ""}`}
      title={flagged ? (!c && r.flagDue ? `Follow up. Due ${r.flagDue}` : "Follow up") : done ? "Completed" : "Click to flag"}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        toggleFlag([r]);
      }}
      onContextMenu={(e) => {
        e.stopPropagation();
        contextMenu(followUpMenu(() => (c ? c.ids : [r.id])))(e);
      }}
    >
      <Icon name={flagged ? "flag" : done ? "flag-complete" : "flag-outline"} size={14} />
    </span>
  );
}

/** The row as drawn: a conversation header shows its aggregates. */
function look(r: MessageRow) {
  const c = r.conv;
  return {
    unread: c ? c.unread > 0 : !r.isRead,
    hasAttachments: c ? c.hasAttachments : r.hasAttachments,
    importance: c ? c.importance : r.importance,
    size: c ? c.size : r.size,
    icon: rowIcon(c ? { ...r, isRead: c.unread === 0 } : r),
  };
}

function rowClass(base: string, r: MessageRow, exp: Exp, selected: boolean, focused: boolean) {
  const lk = look(r);
  return `cm-mail-row ${base}${lk.unread ? " unread" : ""}${r.conv && r.conv.unread > 0 ? " conv-unread" : ""}${exp === "child" ? " child" : ""}${exp === "child" && !r.inView ? " elsewhere" : ""}${selected ? " selected" : ""}${focused ? " focused" : ""}`;
}

function who(r: MessageRow, sentLike: boolean, exp: Exp) {
  if (r.conv && r.conv.total > 1) return r.conv.participants || r.fromName || r.fromEmail;
  if (exp === "child") return r.fromName || r.fromEmail;
  return sentLike ? r.toDisplay || "(no recipient)" : r.fromName || r.fromEmail;
}

/** "(3)" after the participants of a collapsed conversation */
function countOf(r: MessageRow, exp: Exp) {
  return exp === "closed" && r.conv ? <span className="cm-mail-convcount">({r.conv.total})</span> : null;
}

/** a message of an open conversation: its folder (grey) when it isn't in this one, and its first line */
function childText(r: MessageRow) {
  return (
    <>
      {!r.inView && r.folder && <span className="cm-mail-childfolder">{r.folder}</span>}
      <span className="cm-mail-childtext">{r.preview || r.subject || "(no subject)"}</span>
    </>
  );
}

interface RowProps {
  r: MessageRow;
  selected: boolean;
  focused: boolean;
  sentLike: boolean;
  exp: Exp;
  line: number;
  onToggle: (li: number) => void;
}

const CompactRow = memo(function CompactRow({ r, selected, focused, sentLike, exp, line, onToggle }: RowProps) {
  const lk = look(r);
  const conv = exp !== "none";
  return (
    <div className={rowClass("compact", r, exp, selected, focused)} title={r.preview || undefined}>
      {conv && <Twisty exp={exp} line={line} onToggle={onToggle} />}
      <div className="c-icon">
        <Icon name={lk.icon} size={16} />
      </div>
      <div className="c-main">
        <div className="l1">
          <span className="who cm-ellipsis" dir="auto">
            {who(r, sentLike, exp)}
          </span>
          {countOf(r, exp)}
          <span className="date">{listDate(r.date)}</span>
          {lk.hasAttachments && <Icon name="attachment" size={12} />}
        </div>
        <div className="l2">
          {lk.importance === 2 && <Icon name="importance-high" size={11} />}
          {lk.importance === 0 && <Icon name="importance-low" size={11} />}
          <GmailTag r={r} />
          {exp === "child" ? (
            childText(r)
          ) : (
            <span className="subj cm-ellipsis" dir="auto">
              {r.subject || "(no subject)"}
            </span>
          )}
          {r.folder && exp !== "child" && <span className="folder">{r.folder}</span>}
        </div>
      </div>
      {!r.outboxId && <FlagCell r={r} />}
    </div>
  );
});

/** CSS class + inline width for a column cell (header and rows share it). */
function colProps(c: ColDef, widths: Partial<Record<ColId, number>>) {
  const w = colWidth(c, widths);
  const kind = c.fixed ? "fixed" : c.id === "size" ? "right" : "text";
  return { className: `g-col g-${c.id} ${kind}${w === null ? " flex" : ""}`, style: w === null ? undefined : { width: w } };
}

function cellContent(c: ColDef, r: MessageRow, sentLike: boolean, account: string, exp: Exp): React.ReactNode {
  const lk = look(r);
  switch (c.id) {
    case "importance":
      return lk.importance === 2 ? <Icon name="importance-high" size={11} /> : lk.importance === 0 ? <Icon name="importance-low" size={11} /> : null;
    case "icon":
      return <Icon name={lk.icon} size={16} />;
    case "attachment":
      return lk.hasAttachments ? <Icon name="attachment" size={12} /> : null;
    case "from": {
      const n = countOf(r, exp);
      const text = who(r, sentLike, exp);
      return n ? (
        <span className="cm-mail-fromcell">
          <span className="cm-ellipsis">{text}</span>
          {n}
        </span>
      ) : (
        text
      );
    }
    case "to":
      return r.toDisplay;
    case "subject":
      return (
        <>
          <GmailTag r={r} />
          {exp === "child" ? childText(r) : r.subject || "(no subject)"}
        </>
      );
    case "received":
      return columnDate(r.date);
    case "size":
      return listSize(lk.size);
    case "account":
      return account;
    case "folder":
      return r.folder;
    case "flag":
      return r.outboxId ? null : <FlagCell r={r} />;
  }
}

const GridRow = memo(function GridRow({
  r,
  selected,
  focused,
  sentLike,
  cols,
  widths,
  account,
  exp,
  line,
  onToggle,
}: RowProps & {
  cols: ColDef[];
  widths: Partial<Record<ColId, number>>;
  account: string;
}) {
  return (
    <div className={rowClass("grid", r, exp, selected, focused)} title={r.preview || undefined}>
      {exp !== "none" && <Twisty exp={exp} line={line} onToggle={onToggle} />}
      {cols.map((c) => {
        const p = colProps(c, widths);
        const text = !c.fixed && c.id !== "size";
        return (
          <span key={c.id} {...p} dir={text ? "auto" : undefined}>
            {cellContent(c, r, sentLike, account, exp)}
          </span>
        );
      })}
    </div>
  );
});

function GridHeader({
  arrange,
  setArrange,
  sentLike,
  cols,
  widths,
  conv,
}: {
  arrange: ArrangeState;
  setArrange: (p: Partial<ArrangeState>) => void;
  sentLike: boolean;
  cols: ColDef[];
  widths: Partial<Record<ColId, number>>;
  conv: boolean;
}) {
  const { move, setWidth } = useColumns.getState();
  return (
    <div className="cm-mail-gridhead" onContextMenu={contextMenu(() => columnMenu(arrange, setArrange))}>
      {conv && <span className="cm-mail-twisty" />}
      {cols.map((c) => {
        const p = colProps(c, widths);
        const label = c.id === "from" && sentLike && !conv ? "To" : c.label;
        const sorted = !!c.sort && arrange.arrange === c.sort;
        return (
          <span
            key={c.id}
            {...p}
            className={`${p.className}${sorted ? " sorted" : ""}`}
            title={c.title}
            draggable
            onDragStart={(e) => {
              e.dataTransfer.setData("application/x-cm-col", c.id);
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
              if (e.dataTransfer.types.includes("application/x-cm-col")) e.preventDefault();
            }}
            onDrop={(e) => {
              const id = e.dataTransfer.getData("application/x-cm-col") as ColId;
              if (id) move(id, c.id);
            }}
            onClick={(e) => {
              if ((e.target as HTMLElement).classList.contains("cm-mail-colresize") || !c.sort) return;
              if (sorted) setArrange({ desc: !arrange.desc });
              else setArrange({ arrange: c.sort, desc: DEFAULT_DESC[c.sort] });
            }}
          >
            {c.icon ? <Icon name={c.icon} size={c.id === "icon" ? 12 : 10} /> : label}
            {sorted && <Icon name={arrange.desc ? "arrow-down" : "arrow-up"} size={8} />}
            {!c.fixed && c.width !== null && (
              <span
                className="cm-mail-colresize"
                draggable={false}
                onMouseDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  const startX = e.clientX;
                  const startW = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect().width;
                  const mv = (ev: MouseEvent) => setWidth(c.id, startW + ev.clientX - startX);
                  const up = () => {
                    window.removeEventListener("mousemove", mv);
                    window.removeEventListener("mouseup", up);
                  };
                  window.addEventListener("mousemove", mv);
                  window.addEventListener("mouseup", up);
                }}
              />
            )}
          </span>
        );
      })}
    </div>
  );
}

export { markRead };
