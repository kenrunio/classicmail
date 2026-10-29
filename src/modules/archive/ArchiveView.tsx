// An archive folder in the Mail module: folder title + Instant Search (this folder / this archive /
// all archives), the mail MessageList fed by archive.listMessages, and the Reading Pane. Read-only.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useShell, useStatus } from "../../shell/store";
import { Icon, Splitter, clamp, useDebounced, useHotkeys } from "../../ui";
import { archiveApi } from "../../api/archive";
import { DEFAULT_ARRANGE, folderKey, useMailSel, useMailUi, type FolderSel } from "../mail/store";
import { MessageList, type ListBackend, type ListHandle } from "../mail/MessageList";
import { MailDialogHost } from "../mail/dialogs";
import { isTwoLine, useColumns } from "../mail/columns";
import { listKeys, listRows } from "../mail/MailView";
import { useArchives } from "./nav";
import { ArchiveReader } from "./ArchiveReader";
import { archiveCompose, archiveMessageMenu, openArchiveItem, printArchiveItem, retryArchive } from "./actions";
import "./archive.css";

const BACKEND: ListBackend = {
  domain: "archive",
  list: (p) => archiveApi.listMessages(p as never),
  open: openArchiveItem,
  menu: archiveMessageMenu,
};

type Scope = "folder" | "archive" | "all";

export function ArchiveView({ sel: folder }: { sel: Extract<FolderSel, { kind: "archive" }> }) {
  const ui = useMailUi();
  const sel = useMailSel();
  const readingPane = useShell((s) => s.readingPane);
  const setLeft = useStatus((s) => s.setLeft);
  const archives = useArchives();
  const archive = archives.data?.find((a) => a.id === folder.archiveId);
  const af = archive?.folders.find((f) => f.id === folder.id);
  const fkey = folderKey(folder);
  const arrange = ui.arrangements[fkey] ?? DEFAULT_ARRANGE;

  // the archive was closed: back to the mailboxes
  useEffect(() => {
    if (archives.data && !af) useMailUi.setState({ folder: null });
  }, [archives.data, af]);

  // Instant Search
  const [text, setText] = useState("");
  const [scope, setScope] = useState<Scope>(sel.scope === "all" ? "all" : "folder");
  const searchRef = useRef<HTMLInputElement>(null);
  const search = useDebounced(text, 300).trim();
  useEffect(() => {
    setText("");
    setScope(useMailSel.getState().scope === "all" ? "all" : "folder");
  }, [fkey]);
  useEffect(() => {
    const all = () => {
      setScope("all");
      searchRef.current?.focus();
    };
    const focus = () => searchRef.current?.focus();
    window.addEventListener("cm-archive-search-all", all);
    window.addEventListener("cm-focus-search", focus);
    return () => {
      window.removeEventListener("cm-archive-search-all", all);
      window.removeEventListener("cm-focus-search", focus);
    };
  }, []);
  const searching = search.length > 0;

  const params = useMemo(
    () => ({
      source: { folderId: folder.id, archiveId: folder.archiveId },
      arrange: arrange.arrange,
      desc: arrange.desc,
      conversations: false,
      ...(searching ? { search, scope } : {}),
    }),
    [folder.id, folder.archiveId, arrange.arrange, arrange.desc, searching, search, scope],
  );

  const [counts, setCounts] = useState({ total: 0, unread: 0 });
  const onData = useCallback((h: ListHandle) => {
    listRows.current = h.rowsById;
    listKeys.current = h.rowsByKey;
    setCounts((c) => (c.total === h.messages && c.unread === h.unread ? c : { total: h.messages, unread: h.unread }));
  }, []);
  useEffect(() => {
    const t = counts.total.toLocaleString();
    setLeft(searching ? `Filter Applied    Items: ${t}` : `Items: ${t}    Unread: ${counts.unread.toLocaleString()}`);
  }, [counts, searching, setLeft]);
  useEffect(() => () => setLeft(""), [setLeft]);

  const single = sel.keys.length === 1 ? sel.focused : null;
  const one = () => (sel.focused && sel.focused > 0 ? sel.focused : null);
  useHotkeys({
    "ctrl+r": () => one() && archiveCompose("reply", one()!),
    "ctrl+shift+r": () => one() && archiveCompose("replyAll", one()!),
    "ctrl+f": () => one() && archiveCompose("forward", one()!),
    "ctrl+e": () => searchRef.current?.focus(),
    "ctrl+o": () => {
      const r = one() ? listRows.current.get(one()!) : undefined;
      if (r) openArchiveItem(r);
    },
    "ctrl+p": () => one() && printArchiveItem(one()!),
    escape: () => text && setText(""),
  });
  useEffect(() => {
    const onPrint = () => {
      const id = useMailSel.getState().focused;
      if (id && id > 0) printArchiveItem(id);
    };
    window.addEventListener("cm-print", onPrint);
    return () => window.removeEventListener("cm-print", onPrint);
  }, []);

  // layout (same rules as the mail view)
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
  const narrow = readingPane === "right" && viewW > 0 && viewW < MIN_LIST + MIN_READER + 5;
  const rp = narrow ? "bottom" : readingPane;
  const listWidth = viewW > 0 ? Math.max(MIN_LIST, Math.min(ui.listWidth, viewW - MIN_READER - 5)) : ui.listWidth;
  const layout = useColumns((st) => st.layout);
  const compact = isTwoLine(layout, rp, viewW);
  const eff = useRef({ listWidth, viewW });
  eff.current = { listWidth, viewW };
  const resizeList = (d: number) => {
    const { listWidth: cur, viewW: w } = eff.current;
    ui.set({ listWidth: clamp(cur + d, MIN_LIST, Math.max(MIN_LIST, Math.min(900, w - MIN_READER - 5))) });
  };

  const name = af?.name ?? "";
  const scopeLabel = scope === "all" ? "All Archives" : scope === "archive" ? (archive?.displayName ?? "this archive") : name;
  const status = archive?.status;
  const listCol = (
    <div className="cm-mail-listcol" style={rp === "right" ? { width: listWidth } : rp === "bottom" ? { height: ui.listHeight } : undefined}>
      <div className="cm-pane-title cm-mail-title">
        <span className="cm-fill cm-ellipsis" title={archive ? `${archive.displayName} › ${name}` : name}>
          {searching && scope !== "folder" ? scopeLabel : name}
        </span>
      </div>
      <div className="cm-mail-search">
        <input
          ref={searchRef}
          className="cm-mail-search-input"
          placeholder={`Search ${scopeLabel}`}
          value={text}
          dir="auto"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setText("");
            else if (e.key === "ArrowDown") (document.querySelector(".cm-mail-list") as HTMLElement | null)?.focus();
          }}
        />
        <button className="cm-mail-search-btn" title={text ? "Clear Search (Esc)" : "Search"} onClick={() => (text ? setText("") : searchRef.current?.focus())}>
          <Icon name={text ? "close" : "find"} size={text ? 10 : 14} />
        </button>
      </div>
      {searching && (
        <div className="cm-mail-searchbar">
          <span>Search results in {scopeLabel}.</span>
          {scope !== "archive" && (
            <a className="cm-link" onClick={() => setScope("archive")}>
              Search this archive
            </a>
          )}
          {scope !== "all" && (
            <a className="cm-link" onClick={() => setScope("all")}>
              Search All Archives
            </a>
          )}
          {scope !== "folder" && (
            <a className="cm-link" onClick={() => setScope("folder")}>
              Only {name}
            </a>
          )}
        </div>
      )}
      {archive && status !== "ready" && (
        <div className="cm-archive-bar">
          <Icon name={status === "locked" || status === "missing" || status === "error" ? "warning" : "info"} size={14} />
          <span>
            {status === "locked" || status === "missing" || status === "error"
              ? (archive.error ?? "The archive can't be read.")
              : `ClassicMail is indexing this archive (${archive.indexed.toLocaleString()} of ${archive.expected.toLocaleString()} items so far).`}
          </span>
          {(status === "locked" || status === "missing" || status === "error") && (
            <a className="cm-link" onClick={() => retryArchive(archive)}>
              Try Again
            </a>
          )}
        </div>
      )}
      <MessageList
        params={params as never}
        arrange={arrange}
        setArrange={(p) => ui.setArrange(fkey, p)}
        compact={compact}
        folderKey={fkey}
        folderRole={!searching && /^sent/i.test(name) ? "sent" : undefined}
        onData={onData}
        emptyText={searching ? "No items match your search." : status === "ready" ? undefined : "Items appear here as the archive is indexed."}
        backend={BACKEND}
      />
    </div>
  );
  const reader = (
    <div className="cm-mail-readingpane">
      {single !== null && single > 0 ? <ArchiveReader id={single} /> : <div className="cm-mail-rp-empty">{sel.keys.length > 1 ? "" : "Select an item to read."}</div>}
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
          <Splitter direction="horizontal" onResize={(d) => ui.set({ listHeight: clamp(useMailUi.getState().listHeight + d, 100, 900) })} />
          {reader}
        </>
      )}
      <MailDialogHost />
    </div>
  );
}
