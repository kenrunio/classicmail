// Contacts module (Outlook 2007): My Contacts + Current View in the Navigation Pane, card and
// table views with Instant Search, the Contacts toolbar and menus.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { keepPreviousData } from "@tanstack/react-query";
import type { ModuleDef, ModuleMenus } from "../../shell/types";
import { useStatus } from "../../shell/store";
import { newItemMenu, newTask } from "../../shell/newItems";
import {
  Dialog,
  Icon,
  TbButton,
  TbDropdown,
  TbSep,
  TbSplit,
  Toolbar,
  TreeView,
  alertBox,
  errorBox,
  showMenu,
  useDebounced,
  useHotkeys,
  type MenuItem,
  type TreeNode,
} from "../../ui";
import { useRpc } from "../../api/query";
import { rpc } from "../../api/rpc";
import { undoableMany } from "../../api/undo";
import { contactsApi, type Contact, type ContactFolder } from "../../api/contacts";
import { CardView } from "./CardView";
import { ListView } from "./ListView";
import { VIEWS, selectedContacts, useContactsUi } from "./store";
import { isSortKey, sortContacts, sortLabel } from "./sort";
import { currentViewMenu, nameOrderMenu, sortMenu } from "./sortMenu";
import {
  callMenu,
  contextMenuFor,
  currentAccountId,
  currentGroupResource,
  deleteContacts,
  folderLabel,
  moveContacts,
  moveMenu,
  newContact,
  newMeetingWith,
  newMessageTo,
  openAddressBook,
  openContact,
} from "./actions";
import { printContacts } from "./print";
import { useContactFolders } from "./SelectNames";
import "./contacts.css";

/** The selected folder, resolved against what exists (accounts can be removed). */
function useCurrentFolder() {
  const folders = useContactFolders().data ?? [];
  const sel = useContactsUi((s) => s.folder);
  const idx = Math.max(0, sel.accountId == null ? 0 : folders.findIndex((f) => f.accountId === sel.accountId));
  const folder = folders[idx] as ContactFolder | undefined;
  const group = folder?.groups.find((g) => g.id === sel.groupId);
  const title = group ? group.name : folder ? folderLabel(folder, idx) : "Contacts";
  return { folders, folder, group, title, accountId: folder?.accountId ?? null };
}

// ---- Navigation Pane --------------------------------------------------------------------------

function NavPane() {
  const { folders, folder, group } = useCurrentFolder();
  const setFolder = useContactsUi((s) => s.setFolder);
  const view = useContactsUi((s) => s.view);
  const setView = useContactsUi((s) => s.setView);
  const nodes: TreeNode[] = folders.map((f, i) => ({
    id: `a${f.accountId}`,
    label: folderLabel(f, i),
    icon: "contacts",
    title: f.email,
    data: { accountId: f.accountId },
    children: f.groups.map((g) => ({ id: `g${g.id}`, label: g.name, icon: "contact-group", title: `${g.count} contacts`, data: { accountId: f.accountId, groupId: g.id } })),
  }));
  const selectedId = group ? `g${group.id}` : folder ? `a${folder.accountId}` : null;

  const onDrop = async (n: TreeNode, payload: string) => {
    const d = n.data as { accountId: number; groupId?: number };
    let ids: number[] = [];
    try {
      ids = JSON.parse(payload).contacts ?? [];
    } catch {
      return;
    }
    const items = useContactsUi.getState().items.filter((c) => ids.includes(c.id));
    if (!items.length) return;
    if (d.groupId == null) return moveContacts(items, d.accountId);
    const g = folders.flatMap((f) => f.groups).find((x) => x.id === d.groupId);
    if (!g) return;
    try {
      const add = items.filter((c) => c.accountId === d.accountId && !c.groups.includes(g.resourceName));
      await undoableMany(
        `Add ${add.length === 1 ? "1 contact" : `${add.length} contacts`} to ${g.name}`,
        add.map((c) => ({ method: "contacts.update", params: { id: c.id, contact: { ...c, groups: [...c.groups, g.resourceName] } } })),
      );
    } catch (e) {
      errorBox(e);
    }
  };

  return (
    <div className="cm-contacts-nav">
      <div className="cm-nav-section">My Contacts</div>
      <TreeView
        nodes={nodes}
        selectedId={selectedId}
        onSelect={(n) => {
          const d = n.data as { accountId: number; groupId?: number };
          setFolder({ accountId: d.accountId, groupId: d.groupId ?? null });
        }}
        onDropItems={(n, payload) => onDrop(n, payload)}
        menuFor={(n) => {
          const d = n.data as { accountId: number; groupId?: number };
          return [
            { label: "Open", bold: true, icon: "folder-open", onClick: () => setFolder({ accountId: d.accountId, groupId: d.groupId ?? null }) },
            { label: "New Contact", icon: "contact", onClick: () => newContact(d.accountId) },
            { type: "separator" },
            { label: "Send/Receive", icon: "send-receive", onClick: () => rpc("sync.now", { accountId: d.accountId }) },
          ];
        }}
      />
      {folders.length === 0 && <div className="cm-empty">No contact folders.</div>}
      <div className="cm-nav-section">Current View</div>
      <div className="cm-contacts-viewlist" role="radiogroup" aria-label="Current View">
        {VIEWS.map((v) => (
          <label key={v.id} className="cm-contacts-viewopt">
            <input type="radio" name="cm-contacts-view" checked={view === v.id} onChange={() => setView(v.id)} />
            {v.label}
          </label>
        ))}
      </div>
      <div className="cm-contacts-navlinks">
        <span className="cm-contacts-navlink disabled" title="Not available for Google accounts">
          Open a Shared Contacts...
        </span>
        <span className="cm-contacts-navlink disabled" title="Not available for Google accounts">
          Share My Contacts Folder...
        </span>
        <span className="cm-contacts-navlink disabled">Customize Current View...</span>
      </div>
    </div>
  );
}

// ---- View --------------------------------------------------------------------------------------

function View() {
  const { title, accountId, group } = useCurrentFolder();
  const view = useContactsUi((s) => s.view);
  const query = useContactsUi((s) => s.query);
  const setQuery = useContactsUi((s) => s.setQuery);
  const setItems = useContactsUi((s) => s.setItems);
  const q = useDebounced(query.trim(), 200);
  const params = { accountId, groupId: group?.id ?? null, query: q || undefined };
  const list = useRpc<Contact[]>(["contacts", "list", accountId, group?.id ?? null, q], "contacts.list", params, {
    enabled: accountId != null,
    placeholderData: keepPreviousData,
  });
  const sort = useContactsUi((s) => s.sort);
  const nameOrder = useContactsUi((s) => s.nameOrder);
  const isCards = view === "business" || view === "address" || view === "detailed";
  // card views show the list in the View › Sort By order (the table views sort and group themselves)
  const raw = list.data;
  const items = useMemo(() => (raw ? (isCards ? sortContacts(raw, sort, nameOrder) : raw) : []), [raw, isCards, sort, nameOrder]);
  useEffect(() => {
    if (isCards) setItems(items);
  }, [items, isCards, setItems]);
  const setLeft = useStatus((s) => s.setLeft);
  useEffect(() => {
    setLeft(`Items: ${items.length.toLocaleString()}`);
    return () => setLeft("");
  }, [items.length, setLeft]);
  const [range, setRange] = useState("");
  const onRange = useCallback((t: string) => setRange(t), []);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const focus = () => searchRef.current?.focus();
    const print = () => {
      const sel = selectedContacts();
      printContacts(sel.length > 1 ? sel : useContactsUi.getState().items, title);
    };
    window.addEventListener("cm-focus-search", focus);
    window.addEventListener("cm-print", print);
    return () => {
      window.removeEventListener("cm-focus-search", focus);
      window.removeEventListener("cm-print", print);
    };
  }, [title]);

  useHotkeys({
    "ctrl+e": () => searchRef.current?.focus(),
    "ctrl+shift+b": () => openAddressBook(),
    "ctrl+d": () => deleteContacts(selectedContacts()),
    // selected text copies as text; otherwise the selected contacts
    "ctrl+c": () => (window.getSelection()?.toString() || !selectedContacts().length ? document.execCommand("copy") : copyContacts(selectedContacts())),
    "ctrl+p": () => window.dispatchEvent(new CustomEvent("cm-print")),
    "ctrl+shift+c": () => newContact(currentAccountId(), groupParam()),
  });

  return (
    <div className="cm-contacts-view" onDragStart={onDragStart}>
      <div className="cm-pane-title cm-contacts-title">
        <span className="cm-ellipsis">{title}</span>
        {isCards && range && <span className="cm-contacts-range cm-ellipsis">{range}</span>}
        <div className="cm-fill" />
        <SortByButton />
        <div className={`cm-contacts-search${query ? " active" : ""}`}>
          <input
            ref={searchRef}
            placeholder="Search Contacts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setQuery("");
            }}
          />
          {query ? (
            <button className="cm-contacts-search-btn" title="Clear Search" onClick={() => setQuery("")}>
              <Icon name="close" size={10} />
            </button>
          ) : (
            <span className="cm-contacts-search-btn">
              <Icon name="find" size={14} />
            </span>
          )}
        </div>
      </div>
      {q && <div className="cm-contacts-searchbar">Search Results{list.isFetching ? "…" : items.length ? "" : ": No matches found"}</div>}
      <div className="cm-contacts-body">
        {accountId == null ? (
          <div className="cm-empty">No contacts folder is available.</div>
        ) : isCards ? (
          items.length === 0 && !list.isFetching ? (
            <div className="cm-empty">{q ? "No items match your search." : "There are no items to show in this view."}</div>
          ) : (
            <CardView
              key={view}
              items={items}
              kind={view as "business" | "address" | "detailed"}
              sortKey={isSortKey(sort.key) ? sort.key : "fileAs"}
              sortDesc={sort.desc}
              nameOrder={nameOrder}
              onRange={onRange}
            />
          )
        ) : (
          <ListView key={view} items={items} kind={view as "phone" | "company" | "location"} />
        )}
      </div>
    </div>
  );
}

/** "Sort by: File As ▲" in the pane header (as in the new Outlook): opens the Sort By menu. */
function SortByButton() {
  const sort = useContactsUi((s) => s.sort);
  const label = isSortKey(sort.key) ? sortLabel(sort.key) : "Column";
  return (
    <button
      className="cm-contacts-sortby"
      title="Sort by"
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        showMenu(r.left, r.bottom, sortMenu());
      }}
    >
      <span className="cm-contacts-sortby-label">Sort by:</span> {label} <span className="cm-contacts-sortby-dir">{sort.desc ? "▼" : "▲"}</span>
      <Icon name="chevron-down" size={8} />
    </button>
  );
}

function onDragStart(e: React.DragEvent) {
  const el = (e.target as HTMLElement).closest?.("[data-id]") as HTMLElement | null;
  if (!el) return;
  const ids = useContactsUi.getState().selected;
  e.dataTransfer.setData("application/x-cm-items", JSON.stringify({ contacts: ids.length ? ids : [Number(el.dataset.id)] }));
  e.dataTransfer.effectAllowed = "move";
}

function groupParam(): Record<string, string | undefined> {
  const g = currentGroupResource()[0];
  return g ? { group: g } : {};
}

// ---- Toolbar -----------------------------------------------------------------------------------

function FindContactBox() {
  const [text, setText] = useState("");
  const [choices, setChoices] = useState<Contact[] | null>(null);
  const [pick, setPick] = useState(0);
  const find = async () => {
    const t = text.trim();
    if (!t) return;
    try {
      const res = await contactsApi.search(t, 100);
      if (res.length === 0) alertBox(`ClassicMail could not find a contact named "${t}".`, "ClassicMail", "info");
      else if (res.length === 1) openContact(res[0]);
      else {
        setPick(0);
        setChoices(res);
      }
    } catch (e) {
      errorBox(e);
    }
  };
  return (
    <>
      <div className="cm-contacts-findbox" title="Find a Contact">
        <input
          placeholder="Find a Contact"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") find();
          }}
        />
        <Icon name="chevron-down" size={10} />
      </div>
      {choices && (
        <Dialog
          title="Choose Contact"
          width={380}
          onClose={() => setChoices(null)}
          footer={
            <>
              <button
                className="cm-button default"
                onClick={() => {
                  openContact(choices[pick]);
                  setChoices(null);
                }}
              >
                OK
              </button>
              <button className="cm-button" onClick={() => setChoices(null)}>
                Cancel
              </button>
            </>
          }
        >
          <div style={{ marginBottom: 6 }}>Choose a contact to open:</div>
          <div className="cm-contacts-choose">
            {choices.map((c, i) => (
              <div
                key={c.id}
                className={`cm-contacts-choose-row${i === pick ? " selected" : ""}`}
                onMouseDown={() => setPick(i)}
                onDoubleClick={() => {
                  openContact(c);
                  setChoices(null);
                }}
              >
                <Icon name="contact" size={14} />
                <span className="cm-ellipsis" dir="auto">
                  {c.fileAs}
                </span>
                <span className="cm-muted cm-ellipsis">{c.company}</span>
              </div>
            ))}
          </div>
        </Dialog>
      )}
    </>
  );
}

function ModToolbar() {
  const hasSel = useContactsUi((s) => s.selected.length > 0);
  return (
    <Toolbar>
      <TbSplit icon="contact" label="New" menu={newItemMenu} onClick={() => contactsModule.newItem()} title="New Contact (Ctrl+N)" />
      <TbSep />
      <TbButton icon="print" title="Print (Ctrl+P)" onClick={() => window.dispatchEvent(new CustomEvent("cm-print"))} />
      <TbDropdown icon="move" title="Move to Folder" disabled={!hasSel} menu={() => moveMenu(selectedContacts())} />
      <TbButton icon="delete" title="Delete (Ctrl+D)" disabled={!hasSel} onClick={() => deleteContacts(selectedContacts())} />
      <TbSep />
      <TbButton icon="send-receive" label="Send/Receive" title="Send/Receive (F9)" onClick={() => rpc("sync.now", {})} />
      <TbSep />
      <FindContactBox />
      <TbButton icon="address-book" title="Address Book (Ctrl+Shift+B)" onClick={() => openAddressBook()} />
    </Toolbar>
  );
}

// ---- menus ------------------------------------------------------------------------------------

// MainWindow calls `mod.useMenus?.()` only while this module is active, so it must not use React
// hooks (the hook order would change when switching modules). The getters build fresh items each
// time a menu opens instead.
// the contacts as rows of name, company, e-mail and phone (Edit › Copy, Ctrl+C)
function copyContacts(sel: Contact[]) {
  navigator.clipboard.writeText(sel.map((c) => [c.displayName, c.company, c.emails[0]?.value, c.phones[0]?.value].filter(Boolean).join("\t")).join("\n")).catch(() => {});
}

function editMenu(): MenuItem[] {
  const n = useContactsUi.getState().selected.length;
  const has = n > 0;
  const edit: MenuItem[] = [
    { label: "Cut", icon: "cut", shortcut: "Ctrl+X", onClick: () => document.execCommand("cut") },
    {
      label: "Copy",
      icon: "copy",
      shortcut: "Ctrl+C",
      onClick: () => {
        const sel = selectedContacts();
        if (!sel.length || document.activeElement?.tagName === "INPUT") return void document.execCommand("copy");
        copyContacts(sel);
      },
    },
    { label: "Paste", icon: "paste", shortcut: "Ctrl+V", onClick: () => document.execCommand("paste") },
    { type: "separator" },
    {
      label: "Select All",
      shortcut: "Ctrl+A",
      onClick: () =>
        useContactsUi.getState().select(
          useContactsUi.getState().items.map((c) => c.id),
          0,
        ),
    },
    { label: "Delete", icon: "delete", shortcut: "Ctrl+D", disabled: !has, onClick: () => deleteContacts(selectedContacts()) },
    { label: "Move to Folder", icon: "move", disabled: !has, submenu: moveMenu(selectedContacts()) },
  ];
  return edit;
}

function viewMenu(): MenuItem[] {
  return [
    { label: "Current View", submenu: currentViewMenu() },
    { label: "Sort By", icon: "sort", submenu: sortMenu() },
    { label: "Display File As", submenu: nameOrderMenu() },
  ];
}

function actionsMenu(): MenuItem[] {
  const n = useContactsUi.getState().selected.length;
  const has = n > 0;
  const one = n === 1;
  return [
    { label: "New Contact", icon: "contact", shortcut: "Ctrl+N", onClick: () => contactsModule.newItem() },
    { label: "New Distribution List", icon: "contact-group", shortcut: "Ctrl+Shift+L", disabled: true },
    {
      label: "New Contact from Same Company",
      icon: "contact",
      disabled: !one,
      onClick: () => {
        const c = selectedContacts()[0];
        if (c) newContact(c.accountId, { fromCompany: String(c.id) });
      },
    },
    { type: "separator" },
    { label: "New Message to Contact", icon: "new-mail", disabled: !has, onClick: () => newMessageTo(selectedContacts()) },
    { label: "New Meeting Request to Contact", icon: "meeting", disabled: !has, onClick: () => newMeetingWith(selectedContacts()) },
    { label: "New Task for Contact", icon: "task", disabled: !one, onClick: () => newTask() },
    { label: "Call Contact", icon: "phone", disabled: !one, submenu: callMenu(selectedContacts()[0]) },
    { type: "separator" },
    { label: "Open", icon: "contact", disabled: !has, onClick: () => selectedContacts().slice(0, 10).forEach(openContact) },
    { label: "Forward as Business Card", icon: "business-card", disabled: true },
  ];
}

const MENUS: ModuleMenus = {
  get edit() {
    return editMenu();
  },
  get view() {
    return viewMenu();
  },
  get actions() {
    return actionsMenu();
  },
};

function useMenus(): ModuleMenus {
  return MENUS;
}

export const contactsModule: ModuleDef = {
  id: "contacts",
  label: "Contacts",
  icon: "contacts",
  NavPane,
  View,
  Toolbar: ModToolbar,
  useMenus,
  newItem: () => newContact(currentAccountId(), groupParam()),
};

// re-exported for other modules / harnesses
export { contextMenuFor };
