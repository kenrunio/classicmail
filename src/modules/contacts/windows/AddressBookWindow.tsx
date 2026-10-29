// "Address Book: Contacts" window (Tools › Address Book, Ctrl+Shift+B).
import { useEffect, useMemo, useState } from "react";
import { MenuBar, Toolbar, TbButton, TbSep, confirmBox, errorBox, showMenu, useHotkeys, type MenuItem } from "../../../ui";
import { closeCurrentWindow, setWindowTitle } from "../../../api/windows";
import { contactsApi } from "../../../api/contacts";
import { AddressPicker, useAddressRows, useContactFolders, type AddrRow } from "../SelectNames";
import { folderLabel, newContact, newMessageTo, openContact } from "../actions";
import "../contacts.css";

export default function AddressBookWindow() {
  const folders = useContactFolders().data ?? [];
  const [accountId, setAccountId] = useState<number | null>(null);
  const acct = accountId ?? folders[0]?.accountId ?? null;
  const [selected, setSelected] = useState<string[]>([]);
  const { rows } = useAddressRows(acct);
  const rowsSel = useMemo(() => {
    const s = new Set(selected);
    return rows.filter((r) => s.has(r.key));
  }, [rows, selected]);
  const idx = Math.max(
    0,
    folders.findIndex((f) => f.accountId === acct),
  );
  const listName = folders[idx] ? folderLabel(folders[idx], idx) : "Contacts";

  useEffect(() => {
    setWindowTitle(`Address Book: ${listName}`);
  }, [listName]);

  const open = (rows: AddrRow[]) => {
    const seen = new Set<number>();
    for (const r of rows.slice(0, 10)) {
      if (seen.has(r.contact.id)) continue;
      seen.add(r.contact.id);
      openContact(r.contact);
    }
  };
  const copy = (rows: AddrRow[]) => navigator.clipboard.writeText(rows.map((r) => `${r.name} <${r.email}>`).join("; ")).catch(() => {});
  const del = async (rows: AddrRow[]) => {
    const ids = [...new Set(rows.map((r) => r.contact.id))];
    if (!ids.length) return;
    if (
      !(await confirmBox(ids.length === 1 ? `Delete "${rows[0].name}" from your Contacts?` : `Delete these ${ids.length} contacts?`, "Address Book", "warning", "contacts.delete"))
    )
      return;
    try {
      await contactsApi.remove(ids, ids.length === 1 ? `Delete "${rows[0].name}"` : `Delete ${ids.length} contacts`);
      setSelected([]);
    } catch (e) {
      errorBox(e);
    }
  };
  const message = (rows: AddrRow[]) => newMessageTo(rows.map((r) => ({ ...r.contact, emails: [{ value: r.email, type: "", displayName: "" }] })));
  const itemMenu = (rows: AddrRow[]): MenuItem[] => [
    { label: "Properties", bold: true, icon: "contact", disabled: !rows.length, onClick: () => open(rows) },
    { label: "New Message", icon: "new-mail", disabled: !rows.length, onClick: () => message(rows) },
    { type: "separator" },
    { label: "Delete", icon: "delete-red", disabled: !rows.length, onClick: () => del(rows) },
  ];

  useHotkeys({
    "ctrl+n": () => newContact(acct),
    "ctrl+c": () => (window.getSelection()?.toString() || !rowsSel.length ? document.execCommand("copy") : copy(rowsSel)),
    "alt+enter": () => open(rowsSel),
    delete: () => del(rowsSel),
    escape: () => closeCurrentWindow(),
  });

  return (
    <div className="cm-contacts-abwin">
      <MenuBar
        menus={[
          {
            label: "&File",
            items: () => [
              { label: "New Entry...", icon: "contact", onClick: () => newContact(acct) },
              { label: "New Message", icon: "new-mail", disabled: !rowsSel.length, onClick: () => message(rowsSel) },
              { type: "separator" },
              { label: "Properties", disabled: !rowsSel.length, shortcut: "Alt+Enter", onClick: () => open(rowsSel) },
              { label: "Delete", icon: "delete-red", disabled: !rowsSel.length, onClick: () => del(rowsSel) },
              { type: "separator" },
              { label: "Close", onClick: () => closeCurrentWindow() },
            ],
          },
          {
            label: "&Edit",
            items: () => [
              {
                label: "Copy",
                icon: "copy",
                shortcut: "Ctrl+C",
                onClick: () => copy(rowsSel),
              },
            ],
          },
          {
            label: "&Tools",
            items: () => [
              { label: "Find...", icon: "find", disabled: true },
              { label: "Options...", icon: "options", disabled: true },
            ],
          },
        ]}
      />
      <Toolbar>
        <TbButton icon="contact" title="New Entry" onClick={() => newContact(acct)} />
        <TbButton icon="find" title="Find Items" disabled />
        <TbSep />
        <TbButton icon="options" title="Properties" disabled={!rowsSel.length} onClick={() => open(rowsSel)} />
        <TbButton icon="delete" title="Delete" disabled={!rowsSel.length} onClick={() => del(rowsSel)} />
        <TbSep />
        <TbButton icon="new-mail" title="New Message" disabled={!rowsSel.length} onClick={() => message(rowsSel)} />
      </Toolbar>
      <div className="cm-contacts-abwin-body">
        <AddressPicker
          accountId={acct}
          setAccountId={(id) => {
            setAccountId(id);
            setSelected([]);
          }}
          selected={selected}
          setSelected={setSelected}
          onActivate={open}
          onContextMenu={(e, rows) => showMenu(e.clientX, e.clientY, itemMenu(rows))}
          autoFocus
        />
      </div>
    </div>
  );
}
