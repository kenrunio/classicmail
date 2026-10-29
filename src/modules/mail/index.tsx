// Mail module: Navigation Pane folders, message list + Reading Pane, Standard toolbar commands
// and the Edit / View / Actions menus.
import type { ModuleDef, ModuleMenus } from "../../shell/types";
import { newItemMenu } from "../../shell/newItems";
import { rpc } from "../../api/rpc";
import { openWindow } from "../../api/windows";
import { TbButton, TbDropdown, TbSep, TbSplit, Toolbar, type MenuItem } from "../../ui";
import { useAccounts } from "./useAccounts";
import { NavPane } from "./NavPane";
import { MailView, selectedRows } from "./MailView";
import { DEFAULT_ARRANGE, conversationsOn, folderKey, useMailSel, useMailUi } from "./store";
import { arrangeMenu, conversationsMenuItem, layoutMenu } from "./MessageList";
import { archiveItems, compose, replyWithAi, deleteItems, findFromSender, followUpMenu, junkMenu, markAllRead, markRead, moveToFolder, printMessage } from "./actions";
import { findFolder, useMailboxes } from "./folders";
import { startMailMerge } from "./merge";
import { useAiSettings } from "../ai/useTranslation";
import { queryClient } from "../../api/query";
import type { Mailbox } from "../../api/mail";
import { ArchiveToolbar, archiveMenus } from "../archive/toolbar";
import "./mail.css";

const sel = () => useMailSel.getState().selected.filter((x) => x > 0);
/** the message of a single selected row (a conversation row: its newest message) */
const focused = () => {
  const s = useMailSel.getState();
  return s.focused && s.focused > 0 && s.keys.length === 1 ? s.focused : null;
};

/** The Standard toolbar: mail commands, or the read-only set while an archive folder is shown. */
function ModToolbar() {
  const inArchive = useMailUi((u) => u.folder?.kind === "archive");
  return inArchive ? <ArchiveToolbar /> : <MailToolbar />;
}

function MailToolbar() {
  const s = useMailSel();
  const accounts = useAccounts();
  const one = s.keys.length === 1 && !!s.focused && s.focused > 0;
  const any = s.selected.some((x) => x > 0);
  const aiReady = !!useAiSettings().data?.configured;
  return (
    <Toolbar>
      <TbSplit icon="new-mail" label="New" title="New Mail Message (Ctrl+N)" menu={newItemMenu} onClick={() => compose("new")} />
      <TbButton icon="mail-merge" label="Mail Merge" title="Mail Merge: one personalised message per row of an Excel or CSV list" onClick={() => startMailMerge()} />
      <TbSep />
      <TbButton icon="print" title="Print (Ctrl+P)" disabled={!one} onClick={() => focused() && printMessage(focused()!)} />
      <TbDropdown
        icon="move"
        title="Move to Folder"
        disabled={!any}
        menu={() => [
          { label: "Move to Folder…", icon: "move", shortcut: "Ctrl+Shift+V", onClick: () => moveToFolder(sel()) },
          { label: "Copy to Folder…", icon: "copy", onClick: () => moveToFolder(sel(), true) },
        ]}
      />
      <TbButton icon="delete" title="Delete (Ctrl+D)" disabled={!any} onClick={() => deleteItems(sel())} />
      <TbButton icon="archive" label="Archive" title="Archive (Backspace)" disabled={!any} onClick={() => archiveItems(sel())} />
      <TbSep />
      <TbButton
        icon="reply"
        label="Reply"
        title="Reply to everyone on the message (Ctrl+R) — Reply to One is in the message window"
        disabled={!one}
        onClick={() => focused() && compose("replyAll", focused()!)}
      />
      <TbButton
        icon="reply-ai"
        label="Reply with AI"
        title={aiReady ? "Reply with a short draft written by AI" : "Set up AI in Tools › Options › AI"}
        disabled={!one || !aiReady}
        onClick={() => focused() && replyWithAi(focused()!)}
      />
      <TbButton icon="forward" label="Forward" title="Forward (Ctrl+F)" disabled={!one} onClick={() => focused() && compose("forward", focused()!)} />
      <TbSep />
      <TbSplit
        icon="send-receive"
        label="Send/Receive"
        title="Send/Receive All Folders (F9)"
        onClick={() => rpc("sync.now", {})}
        menu={() => [
          { label: "Send/Receive All", icon: "send-receive", shortcut: "F9", onClick: () => rpc("sync.now", {}) },
          { type: "separator" },
          ...accounts.map((a) => ({ label: `"${a.email}"`, onClick: () => rpc("sync.now", { accountId: a.id }) }) as MenuItem),
        ]}
      />
      <TbSep />
      <TbButton icon="find" label="Find" title="Instant Search (Ctrl+E)" onClick={() => window.dispatchEvent(new CustomEvent("cm-focus-search"))} />
      <AddressBox />
      <TbSep />
      <TbDropdown icon="flag" title="Follow Up" disabled={!any} menu={() => followUpMenu(sel)} />
    </Toolbar>
  );
}

function AddressBox() {
  return (
    <span className="cm-mail-addrbox">
      <input
        className="cm-input"
        placeholder="Type a contact to find"
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.currentTarget.value.trim()) {
            openWindow("address-book", { query: e.currentTarget.value.trim() }, { label: "address-book", title: "Address Book" });
            e.currentTarget.value = "";
          }
        }}
      />
      <TbButton icon="address-book" title="Address Book (Ctrl+Shift+B)" onClick={() => openWindow("address-book", {}, { label: "address-book", title: "Address Book" })} />
    </span>
  );
}

function currentFolder() {
  const f = useMailUi.getState().folder;
  if (f?.kind !== "folder") return null;
  return findFolder(queryClient.getQueryData<Mailbox[]>(["mail", "folders"]), f.id)?.folder ?? null;
}

function useMenus(): ModuleMenus {
  const m = useMailMenus();
  const inArchive = useMailUi((u) => u.folder?.kind === "archive");
  // archives: no Show as Conversations, nothing that changes items
  return inArchive ? archiveMenus(m.view?.filter((i) => !("label" in i) || i.label !== "Show as Conversations")) : m;
}

function useMailMenus(): ModuleMenus {
  // subscribe so menus reflect the selection / arrangement
  useMailSel((s) => s.selected);
  useMailSel((s) => s.keys);
  const key = useMailUi((u) => folderKey(u.folder));
  const arr = useMailUi((u) => u.arrangements[key]) ?? DEFAULT_ARRANGE;
  // the conversations check mark (read from the store when the menu is built)
  useMailUi((u) => conversationsOn(u, key));
  useMailboxes();
  const any = () => sel().length > 0;
  const edit: MenuItem[] = [
    { label: "Cut", icon: "cut", shortcut: "Ctrl+X", onClick: () => document.execCommand("cut") },
    { label: "Copy", icon: "copy", shortcut: "Ctrl+C", onClick: () => document.execCommand("copy") },
    { label: "Paste", icon: "paste", shortcut: "Ctrl+V", onClick: () => document.execCommand("paste") },
    { type: "separator" },
    { label: "Archive", icon: "archive", shortcut: "Backspace", disabled: !any(), onClick: () => archiveItems(sel()) },
    { label: "Delete", icon: "delete", shortcut: "Ctrl+D", disabled: !any(), onClick: () => deleteItems(sel()) },
    { label: "Move to Folder…", icon: "move", shortcut: "Ctrl+Shift+V", disabled: !any(), onClick: () => moveToFolder(sel()) },
    { label: "Copy to Folder…", disabled: !any(), onClick: () => moveToFolder(sel(), true) },
    { type: "separator" },
    { label: "Mark as Read", icon: "mark-read", shortcut: "Ctrl+Q", disabled: !any(), onClick: () => markRead(sel(), true) },
    { label: "Mark as Unread", icon: "mark-unread", shortcut: "Ctrl+U", disabled: !any(), onClick: () => markRead(sel(), false) },
    {
      label: "Mark All as Read",
      disabled: !currentFolder(),
      onClick: () => {
        const f = currentFolder();
        if (f) markAllRead(f);
      },
    },
    { type: "separator" },
    {
      label: "Select All",
      shortcut: "Ctrl+A",
      onClick: () => {
        const l = document.querySelector(".cm-mail-list") as HTMLElement | null;
        l?.focus();
        l?.dispatchEvent(new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true }));
      },
    },
  ];
  const view: MenuItem[] = [
    { label: "Arrange By", icon: "sort", submenu: arrangeMenu(arr, (p) => useMailUi.getState().setArrange(key, p)) },
    conversationsMenuItem(),
    { label: "Message List", submenu: layoutMenu() },
  ];
  const one = () => focused();
  const actions: MenuItem[] = [
    { label: "New Mail Message", icon: "new-mail", shortcut: "Ctrl+N", onClick: () => compose("new") },
    { type: "separator" },
    { label: "Reply", icon: "reply", shortcut: "Ctrl+R", disabled: !one(), onClick: () => one() && compose("replyAll", one()!) },
    { label: "Reply to All", icon: "reply-all", shortcut: "Ctrl+Shift+R", disabled: !one(), onClick: () => one() && compose("replyAll", one()!) },
    { label: "Reply with AI", icon: "reply-ai", disabled: !one(), onClick: () => one() && replyWithAi(one()!) },
    { label: "Forward", icon: "forward", shortcut: "Ctrl+F", disabled: !one(), onClick: () => one() && compose("forward", one()!) },
    { type: "separator" },
    { label: "Follow Up", icon: "flag", disabled: !any(), submenu: followUpMenu(sel) },
    { label: "Junk E-mail", icon: "junk", disabled: !any(), submenu: junkMenu(sel, () => selectedRows()[0]?.fromEmail) },
    {
      label: "Find All",
      icon: "find",
      disabled: !one(),
      submenu: [{ label: "Messages from Sender…", onClick: () => selectedRows()[0] && findFromSender(selectedRows()[0].fromEmail) }],
    },
  ];
  return { edit, view, actions };
}

export const mailModule: ModuleDef = {
  id: "mail",
  label: "Mail",
  icon: "mail",
  NavPane,
  View: MailView,
  Toolbar: ModToolbar,
  useMenus,
  newItem: () => compose("new"),
};
