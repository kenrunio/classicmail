// Mail module view: folder title + Instant Search, message list, Reading Pane (right/bottom/off).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useShell, useStatus } from "../../shell/store";
import { useRpc } from "../../api/query";
import { Icon, Splitter, clamp, useDebounced, useHotkeys } from "../../ui";
import type { MessageRow } from "../../api/mail";
import { mailApi } from "../../api/mail";
import { DEFAULT_ARRANGE, conversationsOn, folderKey, searchHandoff, sourceOf, useMailSel, useMailUi, type ArrangeState } from "./store";
import { findFolder, useMailboxes } from "./folders";
import { MessageList, type ListHandle } from "./MessageList";
import { MessageReader } from "./MessageReader";
import { MailDialogHost } from "./dialogs";
import { compose, deleteFolder, deleteItems, archiveItems, markRead, moveToFolder, newFolder, openItem, printMessage, toggleFlag } from "./actions";
import { isTwoLine, useColumns } from "./columns";
import { ArchiveView } from "../archive/ArchiveView";

/** Rows currently known to the list (for toolbar / menu commands): by message id and by row key. */
export const listRows = { current: new Map<number, MessageRow>() };
export const listKeys = { current: new Map<string, MessageRow>() };

/** The selected rows (a conversation header row stands for its messages in the view). */
export function selectedRows(): MessageRow[] {
  return useMailSel
    .getState()
    .keys.map((k) => listKeys.current.get(k))
    .filter((r): r is MessageRow => !!r);
}

interface MailSettings {
  "mail.markReadMode"?: "delay" | "selectionChange" | "never" | null;
  "mail.markReadDelaySeconds"?: number | null;
}

/** The mail view, or an Outlook archive folder (src/modules/archive) when one is selected. */
export function MailView() {
  const folder = useMailUi((s) => s.folder);
  if (folder?.kind === "archive") return <ArchiveView key="archive" sel={folder} />;
  return <MailFolderView key="mail" />;
}

function MailFolderView() {
  // fields, not the whole store: a change elsewhere in it (collapsed groups, the folder tree)
  // doesn't redraw the view
  const folder = useMailUi((s) => s.folder);
  const fkey = folderKey(folder);
  const arrange = useMailUi((s) => s.arrangements[fkey]) ?? DEFAULT_ARRANGE;
  const convOn = useMailUi((s) => conversationsOn(s, fkey));
  const listWidthPref = useMailUi((s) => s.listWidth);
  const listHeight = useMailUi((s) => s.listHeight);
  const sel = useMailSel();
  const shell = { readingPane: useShell((s) => s.readingPane) };
  const setLeft = useStatus((s) => s.setLeft);
  const mbs = useMailboxes().data;
  const found = folder?.kind === "folder" ? findFolder(mbs, folder.id) : null;
  const title =
    folder?.kind === "folder"
      ? (found?.folder.name ?? "")
      : folder?.kind === "search"
        ? { unread: "Unread Mail", followUp: "For Follow Up", large: "Large Mail", archive: "Archive" }[folder.sf]
        : folder?.kind === "ids"
          ? folder.label
          : "Mail";

  // Instant Search
  const [text, setText] = useState(sel.search);
  const searchRef = useRef<HTMLInputElement>(null);
  const debounced = useDebounced(text, 300);
  useEffect(() => {
    if (debounced !== useMailSel.getState().search) useMailSel.getState().setSearch(debounced);
  }, [debounced]);
  useEffect(() => {
    const onChange = () => setText(useMailSel.getState().search);
    window.addEventListener("cm-mail-search-changed", onChange);
    return () => window.removeEventListener("cm-mail-search-changed", onChange);
  }, []);
  // another folder starts unfiltered; on the first draw, a search handed over just before stays
  const searchedKey = useRef<string | null>(null);
  useEffect(() => {
    if (searchedKey.current === fkey) return; // StrictMode's second run
    const first = searchedKey.current === null;
    searchedKey.current = fkey;
    if (first && searchHandoff.keep) {
      searchHandoff.keep = false;
      return;
    }
    setText("");
    useMailSel.getState().setSearch("");
  }, [fkey]);
  useEffect(() => {
    const focus = () => searchRef.current?.focus();
    window.addEventListener("cm-focus-search", focus);
    return () => window.removeEventListener("cm-focus-search", focus);
  }, []);

  const searching = sel.search.trim().length > 0;

  // Gmail's own search runs alongside the local one (3+ characters): what it finds that the local
  // search can't (bodies not downloaded, text in attachments) is merged into the same list
  const gmailAccount = sel.scope === "all" ? undefined : found?.folder.accountId;
  const gmailKey = searching && sel.search.trim().length >= 3 ? `${sel.scope}|${gmailAccount ?? "all"}|${sel.search.trim()}` : null;
  const [gmail, setGmail] = useState<{ key: string; status: "searching" | "done" | "error"; ids: number[]; extraIds: number[] } | null>(null);
  // "Show only these": just what Gmail found and the local search didn't
  const [gmailOnly, setGmailOnly] = useState(false);
  useEffect(() => {
    if (!gmailKey) {
      setGmail(null);
      return;
    }
    let stale = false;
    setGmail({ key: gmailKey, status: "searching", ids: [], extraIds: [] });
    setGmailOnly(false);
    mailApi
      .searchServer(sel.search.trim(), gmailAccount)
      .then((r) => !stale && setGmail({ key: gmailKey, status: "done", ids: r.ids, extraIds: r.extraIds ?? [] }))
      .catch(() => !stale && setGmail({ key: gmailKey, status: "error", ids: [], extraIds: [] }));
    return () => {
      stale = true;
    };
  }, [gmailKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const gmailDone = gmail?.key === gmailKey && gmail.status === "done" ? gmail : null;
  const gmailIds = gmailDone && gmailDone.extraIds.length > 0 ? gmailDone.ids : null;
  const gmailExtra = useMemo(() => (gmailDone?.extraIds.length ? new Set(gmailDone.extraIds) : null), [gmailDone]);
  const onlyGmail = gmailOnly && !!gmailExtra;
  // just Gmail's extra results, in the view's scope (All Mail Items, or the folder searched)
  const gmailSource = useMemo(() => {
    if (!gmailDone?.extraIds.length || !folder) return null;
    return sel.scope === "all" ? { source: { ids: gmailDone.extraIds } } : { source: sourceOf(folder), onlyIds: gmailDone.extraIds };
  }, [gmailDone, folder, sel.scope]);
  // how many of them the list shows (the others are in other folders)
  const gmailHere =
    useRpc<{ total: number }>(["mail", "gmailExtraCount", gmailSource], "mail.listMessages", { ...gmailSource, limit: 1 }, { enabled: !!gmailSource }).data?.total ?? 0;
  // View › Show as Conversations (per folder, or for all mailboxes); not for the Outbox queue
  const conversations = convOn && found?.folder.role !== "outbox";
  const params = useMemo(() => {
    if (!folder) return null;
    if (onlyGmail && gmailSource) return { ...gmailSource, arrange: arrange.arrange, desc: arrange.desc, conversations };
    return {
      source: sourceOf(folder),
      arrange: arrange.arrange,
      desc: arrange.desc,
      conversations,
      ...(searching ? { search: sel.search, scope: sel.scope, ...(gmailIds ? { alsoIds: gmailIds } : {}) } : {}),
    };
  }, [folder, arrange.arrange, arrange.desc, searching, sel.search, sel.scope, gmailIds, conversations, onlyGmail, gmailSource]);

  const [counts, setCounts] = useState<{ total: number; unread: number }>({ total: 0, unread: 0 });
  const onData = useCallback((h: ListHandle) => {
    listRows.current = h.rowsById;
    listKeys.current = h.rowsByKey;
    // Outlook counts items (messages), also when they're shown as conversations
    setCounts((c) => (c.total === h.messages && c.unread === h.unread ? c : { total: h.messages, unread: h.unread }));
  }, []);
  useEffect(() => {
    const t = counts.total.toLocaleString();
    setLeft(searching ? `Filter Applied\u00a0\u00a0\u00a0\u00a0Items: ${t}` : `Items: ${t}\u00a0\u00a0\u00a0\u00a0Unread: ${counts.unread.toLocaleString()}`);
  }, [counts, searching, setLeft]);
  useEffect(() => () => setLeft(""), [setLeft]);

  // mark as read after the item has been shown for a moment (Tools › Options › Reading Pane)
  const settings = useRpc<MailSettings>(["settings", "mail", "reading"], "settings.getMany", { keys: ["mail.markReadMode", "mail.markReadDelaySeconds"] }).data;
  const mode = settings?.["mail.markReadMode"] ?? "delay";
  const delay = Math.max(0, settings?.["mail.markReadDelaySeconds"] ?? 1);
  // one row selected: the message it shows (a conversation shows its newest message)
  const single = sel.keys.length === 1 ? sel.focused : null;
  const prevSingle = useRef<number | null>(null);
  useEffect(() => {
    const prev = prevSingle.current;
    prevSingle.current = single;
    if (shell.readingPane === "off") return;
    if (mode === "selectionChange" && prev && prev !== single) {
      const r = listRows.current.get(prev);
      if (r && !r.isRead) markRead([prev], true, false);
    }
    if (mode !== "delay" || !single || single < 0) return;
    const r = listRows.current.get(single);
    if (!r || r.isRead) return;
    const t = setTimeout(() => {
      // still the one row showing it (a conversation header selects all its messages)
      const st = useMailSel.getState();
      if (st.keys.length === 1 && st.focused === single) markRead([single], true, false);
    }, delay * 1000);
    return () => clearTimeout(t);
  }, [single, mode, delay, shell.readingPane]);

  // keyboard (window-wide while the mail module is shown)
  const one = () => (sel.focused && sel.focused > 0 ? sel.focused : null);
  // with the folder list focused, Delete is about the folder, not the messages
  const inTree = () => !!document.activeElement?.closest(".cm-mail-tree");
  const goTo = (role: string) => {
    const mb = found?.mailbox ?? mbs?.[0];
    const f = mb?.folders.find((x) => x.role === role);
    if (f) useMailUi.getState().setFolder({ kind: "folder", id: f.id, accountId: f.accountId });
  };
  useHotkeys({
    "ctrl+r": () => one() && compose("replyAll", one()!),
    "ctrl+shift+r": () => one() && compose("replyAll", one()!),
    "ctrl+f": () => one() && compose("forward", one()!),
    "ctrl+q": () => markRead(sel.selected, true),
    "ctrl+u": () => markRead(sel.selected, false),
    "ctrl+shift+v": () => moveToFolder(sel.selected, false, found?.folder.accountId),
    "ctrl+e": () => searchRef.current?.focus(),
    f3: () => searchRef.current?.focus(),
    "ctrl+shift+i": () => goTo("inbox"),
    "ctrl+shift+o": () => goTo("outbox"),
    "ctrl+shift+e": () => (found ? newFolder(found.folder) : newFolder(undefined, mbs?.[0]?.accountId)),
    "ctrl+o": () => {
      const r = one() ? listRows.current.get(one()!) : undefined;
      if (r) openItem(r);
    },
    delete: () => {
      if (!inTree()) deleteItems(sel.selected.filter((x) => x > 0));
      else if (found && (found.folder.role === "user" || found.folder.role === "placeholder")) void deleteFolder(found.folder);
    },
    backspace: () => !inTree() && archiveItems(sel.selected.filter((x) => x > 0)),
    "ctrl+d": () => !inTree() && deleteItems(sel.selected.filter((x) => x > 0)),
    insert: () => !inTree() && toggleFlag(selectedRows()),
    "ctrl+p": () => one() && printMessage(one()!),
    escape: () => {
      if (text) {
        setText("");
        useMailSel.getState().setSearch("");
      }
    },
  });
  useEffect(() => {
    const onPrint = () => {
      const id = useMailSel.getState().focused;
      if (id && id > 0) printMessage(id);
    };
    window.addEventListener("cm-print", onPrint);
    return () => window.removeEventListener("cm-print", onPrint);
  }, []);

  // a narrow window: shrink the list first (down to 220 px), and when even that leaves the Reading Pane
  // under ~300 px, show it below the list for now (Outlook's "Auto"); the chosen layout isn't changed
  const viewRef = useRef<HTMLDivElement>(null);
  const [viewW, setViewW] = useState(0);
  useEffect(() => {
    const el = viewRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewW(el.clientWidth));
    ro.observe(el);
    setViewW(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  const MIN_LIST = 220;
  const MIN_READER = 300;
  const narrow = shell.readingPane === "right" && viewW > 0 && viewW < MIN_LIST + MIN_READER + 5;
  const rp = narrow ? "bottom" : shell.readingPane;
  const listWidth = viewW > 0 ? Math.max(MIN_LIST, Math.min(listWidthPref, viewW - MIN_READER - 5)) : listWidthPref;
  // like Outlook, a narrow list uses the two-line layout even with the Reading Pane below or off
  const layout = useColumns((st) => st.layout);
  const compact = isTwoLine(layout, rp, viewW);
  const eff = useRef({ listWidth, viewW });
  eff.current = { listWidth, viewW };
  const resizeList = (d: number) => {
    const { listWidth: cur, viewW: w } = eff.current;
    useMailUi.getState().set({ listWidth: clamp(cur + d, MIN_LIST, Math.max(MIN_LIST, Math.min(900, w - MIN_READER - 5))) });
  };
  const role = found?.folder.role;
  const setArrange = useCallback((p: Partial<ArrangeState>) => useMailUi.getState().setArrange(fkey, p), [fkey]);
  const searchPlaceholder = sel.scope === "all" && searching ? "Search All Mail Items" : `Search ${title}`;

  const listCol = (
    <div className="cm-mail-listcol" style={rp === "right" ? { width: listWidth } : rp === "bottom" ? { height: listHeight } : undefined}>
      <div className="cm-pane-title cm-mail-title">
        <span className="cm-fill cm-ellipsis">{searching && sel.scope === "all" ? "All Mail Items" : title}</span>
      </div>
      <div className="cm-mail-search">
        <input
          ref={searchRef}
          className="cm-mail-search-input"
          placeholder={searchPlaceholder}
          value={text}
          dir="auto"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setText("");
              useMailSel.getState().setSearch("");
            } else if (e.key === "Enter") {
              useMailSel.getState().setSearch(text);
            } else if (e.key === "ArrowDown") {
              (document.querySelector(".cm-mail-list") as HTMLElement | null)?.focus();
            }
          }}
        />
        <button
          className="cm-mail-search-btn"
          title={text ? "Clear Search (Esc)" : "Search"}
          onClick={() => {
            if (text) {
              setText("");
              useMailSel.getState().setSearch("");
            } else searchRef.current?.focus();
          }}
        >
          <Icon name={text ? "close" : "find"} size={text ? 10 : 14} />
        </button>
      </div>
      {searching && (
        <div className="cm-mail-searchbar">
          <span>{sel.scope === "all" ? "Search results in All Mail Items." : `Search results in ${title}.`}</span>
          {sel.scope !== "all" && (
            <a className="cm-link" onClick={() => useMailSel.getState().setSearch(sel.search, "all")}>
              Try searching again in All Mail Items
            </a>
          )}
          {gmail?.key === gmailKey && (
            <span className="cm-muted">
              {gmail.status === "searching"
                ? "Searching Gmail too…"
                : gmail.status === "error"
                  ? "Gmail couldn't be searched (offline?) — local results only."
                  : gmailHere > 0
                    ? onlyGmail
                      ? `Showing only the ${gmailHere} that Gmail found.`
                      : `Also searched Gmail: ${gmailHere} more found, tagged`
                    : gmail.extraIds.length > 0 && sel.scope !== "all"
                      ? `Also searched Gmail: ${gmail.extraIds.length} more found in other folders.`
                      : "Also searched Gmail: nothing more found."}
              {gmailHere > 0 && !onlyGmail && <span className="cm-mail-gmailtag">Gmail</span>}
            </span>
          )}
          {gmailHere > 0 && (
            <a className="cm-link" onClick={() => setGmailOnly(!onlyGmail)}>
              {onlyGmail ? "Show all results" : "Show only these"}
            </a>
          )}
        </div>
      )}
      {params && (
        <MessageList
          params={params}
          arrange={arrange}
          setArrange={setArrange}
          compact={compact}
          folderKey={fkey}
          folderRole={searching && sel.scope === "all" ? undefined : role}
          fromFolderId={folder?.kind === "folder" && !searching ? folder.id : undefined}
          onData={onData}
          emptyText={searching ? "No items match your search." : undefined}
          gmailFound={gmailExtra}
        />
      )}
    </div>
  );

  const reader = (
    <div className="cm-mail-readingpane">
      {single !== null && single > 0 ? <MessageReader id={single} /> : <div className="cm-mail-rp-empty">{sel.keys.length > 1 ? "" : "Select an item to read."}</div>}
    </div>
  );

  return (
    <div ref={viewRef} className={`cm-mail-view rp-${rp}`}>
      {listCol}
      {rp === "right" && (
        <>
          <Splitter direction="vertical" onResize={resizeList} />
          {reader}
        </>
      )}
      {rp === "bottom" && (
        <>
          <Splitter direction="horizontal" onResize={(d) => useMailUi.getState().set({ listHeight: clamp(useMailUi.getState().listHeight + d, 100, 900) })} />
          {reader}
        </>
      )}
      <MailDialogHost />
    </div>
  );
}
