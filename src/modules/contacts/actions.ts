// Commands on contacts, shared by the module view, toolbar, menus and the contact window.
import type { MenuItem } from "../../ui";
import { alertBox, confirmBox, errorBox } from "../../ui";
import { openWindow } from "../../api/windows";
import { contactsApi, addressFor, phoneLabel, type Contact, type ContactFolder } from "../../api/contacts";
import { queryClient } from "../../api/query";
import { selectedContacts, useContactsUi } from "./store";
import { printContacts } from "./print";

export function openContact(c: Pick<Contact, "id" | "displayName">) {
  return openWindow("contact", { id: c.id }, { title: `${c.displayName || "Untitled"} - Contact`, label: `contact-${c.id}` });
}

export function newContact(accountId?: number | null, extra: Record<string, string | undefined> = {}) {
  return openWindow("contact", { accountId: accountId ?? undefined, ...extra }, { title: "Untitled - Contact" });
}

/** The account whose Contacts folder is selected (falls back to the first account). */
export function currentAccountId(): number | null {
  const f = useContactsUi.getState().folder;
  if (f.accountId != null) return f.accountId;
  const folders = queryClient.getQueryData<ContactFolder[]>(["contacts", "folders"]);
  return folders?.[0]?.accountId ?? null;
}

/** New contacts made while a group is selected join that group. */
export function currentGroupResource(): string[] {
  const f = useContactsUi.getState().folder;
  if (f.groupId == null) return [];
  const folders = queryClient.getQueryData<ContactFolder[]>(["contacts", "folders"]) ?? [];
  const g = folders.flatMap((x) => x.groups).find((x) => x.id === f.groupId);
  return g ? [g.resourceName] : [];
}

export function newMessageTo(contacts: Contact[]) {
  const to = contacts.map((c) => addressFor(c)).filter(Boolean);
  if (!to.length) {
    alertBox("The selected contact doesn't have an e-mail address.", "ClassicMail", "info");
    return;
  }
  openWindow("compose", { mode: "new", to: to.join("; ") }, { title: "Untitled - Message" });
}

export function newMeetingWith(contacts: Contact[]) {
  const to = contacts.map((c) => addressFor(c)).filter(Boolean);
  openWindow("appointment", { meeting: 1, to: to.join("; ") || undefined }, { title: "Untitled - Meeting" });
}

export async function callNumber(c: Contact, number: string, label: string) {
  try {
    await navigator.clipboard.writeText(number);
  } catch {
    /* clipboard may be unavailable */
  }
  alertBox(`Call ${c.displayName} — ${label}:\n\n${number}\n\nThe number was copied to the clipboard.`, "New Call", "info");
}

export function callMenu(c: Contact | undefined): MenuItem[] {
  if (!c || c.phones.length === 0) return [{ label: "(no phone numbers)", disabled: true }];
  return c.phones.map((p) => {
    const label = phoneLabel(p.type);
    return { label: `${p.value}  ${label}`, icon: "phone", onClick: () => callNumber(c, p.value, label) };
  });
}

export async function deleteContacts(contacts: Contact[]): Promise<boolean> {
  if (!contacts.length) return false;
  const what = contacts.length === 1 ? `"${contacts[0].displayName || contacts[0].fileAs || "Untitled"}"` : `these ${contacts.length} contacts`;
  const ok = await confirmBox(
    `Are you sure you want to delete ${what}?\n\nGoogle keeps deleted contacts in the Contacts Trash for 30 days.`,
    "ClassicMail",
    "warning",
    "contacts.delete",
  );
  if (!ok) return false;
  try {
    await contactsApi.remove(
      contacts.map((c) => c.id),
      contacts.length === 1 ? `Delete ${what}` : `Delete ${contacts.length} contacts`,
    );
    useContactsUi.getState().select([], null);
    return true;
  } catch (e) {
    errorBox(e);
    return false;
  }
}

export async function moveContacts(contacts: Contact[], accountId: number) {
  const ids = contacts.filter((c) => c.accountId !== accountId).map((c) => c.id);
  if (!ids.length) return;
  try {
    await contactsApi.move(ids, accountId, ids.length === 1 ? "Move 1 contact" : `Move ${ids.length} contacts`);
  } catch (e) {
    errorBox(e);
  }
}

export function moveMenu(contacts: Contact[]): MenuItem[] {
  const folders = queryClient.getQueryData<ContactFolder[]>(["contacts", "folders"]) ?? [];
  if (folders.length < 2) return [{ label: "(no other Contacts folders)", disabled: true }];
  return folders.map((f, i) => ({
    label: folderLabel(f, i),
    icon: "contacts",
    disabled: !contacts.length || contacts.every((c) => c.accountId === f.accountId),
    onClick: () => moveContacts(contacts, f.accountId),
  }));
}

export function folderLabel(f: Pick<ContactFolder, "email">, index: number): string {
  return index === 0 ? "Contacts" : `Contacts in ${f.email}`;
}

export function openAddressBook() {
  return openWindow("address-book", {}, { label: "address-book", title: "Address Book: Contacts" });
}

/** Right-click menu for the selected contacts. */
export function contextMenuFor(): MenuItem[] {
  const sel = selectedContacts();
  const one = sel.length === 1 ? sel[0] : undefined;
  return [
    { label: "Open", bold: true, icon: "contact", disabled: !sel.length, onClick: () => sel.slice(0, 10).forEach(openContact) },
    { label: "Print", icon: "print", disabled: !sel.length, onClick: () => printContacts(sel) },
    { type: "separator" },
    { label: "New Message to Contact", icon: "new-mail", disabled: !sel.length, onClick: () => newMessageTo(sel) },
    { label: "New Meeting Request to Contact", icon: "meeting", disabled: !sel.length, onClick: () => newMeetingWith(sel) },
    { label: "Call Contact", icon: "phone", disabled: !one, submenu: callMenu(one) },
    { type: "separator" },
    { label: "Move to Folder", icon: "move", disabled: !sel.length, submenu: moveMenu(sel) },
    { label: "Delete", icon: "delete-red", disabled: !sel.length, onClick: () => deleteContacts(sel) },
  ];
}
