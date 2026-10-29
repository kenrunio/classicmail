// Standard toolbar and menus while an archive folder is shown: read-only (Reply / Reply to All /
// Forward open the normal compose window; Move / Delete / Flag are unavailable).
import type { ModuleMenus } from "../../shell/types";
import { newItemMenu } from "../../shell/newItems";
import { TbButton, TbSep, TbSplit, Toolbar, type MenuItem } from "../../ui";
import { compose } from "../mail/actions";
import { useMailSel } from "../mail/store";
import { listRows } from "../mail/MailView";
import { archiveCompose, openArchiveDialog, openArchiveItem, printArchiveItem } from "./actions";

const focused = () => {
  const s = useMailSel.getState();
  return s.focused && s.focused > 0 && s.keys.length === 1 ? s.focused : null;
};

export function ArchiveToolbar() {
  const s = useMailSel();
  const one = s.keys.length === 1 && !!s.focused && s.focused > 0;
  return (
    <Toolbar>
      <TbSplit icon="new-mail" label="New" title="New Mail Message (Ctrl+N)" menu={newItemMenu} onClick={() => compose("new")} />
      <TbSep />
      <TbButton icon="print" title="Print (Ctrl+P)" disabled={!one} onClick={() => focused() && printArchiveItem(focused()!)} />
      <TbButton icon="move" title="Move to Folder (archives are read-only)" disabled onClick={() => {}} />
      <TbButton icon="delete" title="Delete (archives are read-only)" disabled onClick={() => {}} />
      <TbSep />
      <TbButton icon="reply" label="Reply" title="Reply (Ctrl+R)" disabled={!one} onClick={() => focused() && archiveCompose("reply", focused()!)} />
      <TbButton icon="reply-all" label="Reply to All" title="Reply to All (Ctrl+Shift+R)" disabled={!one} onClick={() => focused() && archiveCompose("replyAll", focused()!)} />
      <TbButton icon="forward" label="Forward" title="Forward (Ctrl+F)" disabled={!one} onClick={() => focused() && archiveCompose("forward", focused()!)} />
      <TbSep />
      <TbButton icon="find" label="Find" title="Instant Search (Ctrl+E)" onClick={() => window.dispatchEvent(new CustomEvent("cm-focus-search"))} />
    </Toolbar>
  );
}

/** Edit / Actions menus for archive folders (View › Arrange By comes from the mail module). */
export function archiveMenus(view: MenuItem[] | undefined): ModuleMenus {
  const one = focused();
  const edit: MenuItem[] = [
    { label: "Copy", icon: "copy", shortcut: "Ctrl+C", onClick: () => document.execCommand("copy") },
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
  const actions: MenuItem[] = [
    { label: "New Mail Message", icon: "new-mail", shortcut: "Ctrl+N", onClick: () => compose("new") },
    { type: "separator" },
    { label: "Open", icon: "folder-open", shortcut: "Ctrl+O", disabled: !one, onClick: () => one && listRows.current.get(one) && openArchiveItem(listRows.current.get(one)!) },
    { label: "Reply", icon: "reply", shortcut: "Ctrl+R", disabled: !one, onClick: () => one && archiveCompose("reply", one) },
    { label: "Reply to All", icon: "reply-all", shortcut: "Ctrl+Shift+R", disabled: !one, onClick: () => one && archiveCompose("replyAll", one) },
    { label: "Forward", icon: "forward", shortcut: "Ctrl+F", disabled: !one, onClick: () => one && archiveCompose("forward", one) },
    { type: "separator" },
    { label: "Open Outlook Data File…", onClick: openArchiveDialog },
  ];
  return { edit, view, actions };
}
