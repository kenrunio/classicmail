import { describe, expect, it } from "vitest";
import { mailboxNode } from "./folders";
import type { MailFolder, Mailbox } from "../../api/mail";

const f = (id: number, name: string, role: string, parentId: number | null = null): MailFolder =>
  ({ id, accountId: 1, gmailLabelId: role === "placeholder" ? null : `L${id}`, name, path: name, parentId, role, hidden: false, unread: 0, total: 0 }) as MailFolder;

describe("mailboxNode", () => {
  it("nests Inbox/… labels under the real Inbox instead of a second Inbox folder", () => {
    const mb = {
      accountId: 1,
      email: "kenny@duvz.demo",
      displayName: "Kenny Green",
      unreadMail: 0,
      followUp: 0,
      largeMail: 0,
      folders: [
        f(1, "Inbox", "inbox"),
        f(2, "Sent Items", "sent"),
        f(10, "Inbox", "placeholder"),
        f(11, "AA - Research", "user", 10),
        f(12, "Receipts", "user", 10),
        f(20, "Clients", "user"),
      ],
    } as Mailbox;
    const tree = mailboxNode(mb, { withSearch: true });
    const kids = tree.children!;
    const inbox = kids.find((n) => n.id === "f1")!;
    expect(inbox.children?.map((n) => n.label)).toEqual(["AA - Research", "Receipts"]);
    expect(kids.some((n) => n.id === "f10")).toBe(false); // the extra "Inbox" node is gone
    expect(kids.some((n) => n.id === "f20")).toBe(true);
    const search = kids.find((n) => n.label === "Search Folders")!;
    expect(search.children?.map((n) => n.label)).not.toContain("Unread Mail");
  });
});

import { applyFolderOrder } from "./NavPane";

describe("applyFolderOrder", () => {
  it("puts saved ids first in order, keeps new ones after, and recurses", () => {
    const n = (id: string, children?: { id: string; label: string }[]) => ({ id, label: id, children });
    const tree = [
      n("mb1", [
        { id: "f1", label: "Inbox" },
        { id: "f2", label: "Sent" },
        { id: "f3", label: "Clients" },
      ]),
      n("mb2"),
    ];
    const out = applyFolderOrder(tree, { root: ["mb2", "mb1"], mb1: ["f3", "f1"] });
    expect(out.map((x) => x.id)).toEqual(["mb2", "mb1"]);
    expect(out[1].children!.map((x) => x.id)).toEqual(["f3", "f1", "f2"]);
  });
});

import { accountInitials, favoriteNodes } from "./NavPane";

describe("favorites", () => {
  it("makes initials from account names", () => {
    expect(accountInitials("QX Global IR")).toBe("QXGIR");
    expect(accountInitials("Kenny Green")).toBe("KG");
    expect(accountInitials("", "info@duvz.demo")).toBe("INFO");
  });
  it("labels a duplicate folder name from a non-main mailbox with the account initials", () => {
    const mk = (accountId: number, displayName: string, ids: number[]) =>
      ({
        accountId,
        email: `a${accountId}@x.com`,
        displayName,
        unreadMail: 0,
        followUp: 0,
        largeMail: 0,
        folders: [
          { id: ids[0], accountId, gmailLabelId: "INBOX", name: "Inbox", path: "Inbox", parentId: null, role: "inbox", hidden: false, unread: 0, total: 0 },
          { id: ids[1], accountId, gmailLabelId: "L1", name: accountId === 1 ? "Clients" : "Press", path: "x", parentId: null, role: "user", hidden: false, unread: 0, total: 0 },
        ],
      }) as never;
    const mbs = [mk(1, "Kenny Green", [1, 2]), mk(2, "Duvz Software", [11, 12])];
    const nodes = favoriteNodes(["f1", "f11", "f12", "sfollowUp:2", "f999"], mbs);
    expect(nodes.map((n) => n.label)).toEqual(["Inbox", "Inbox [DS]", "Press", "For Follow Up [DS]"]);
  });
});

import { folderTarget } from "./NavPane";

describe("dragging a folder", () => {
  const at = (id: number, path: string, role: string, parentId: number | null = null, accountId = 1): MailFolder =>
    ({ ...f(id, path.split("/").pop()!, role, parentId), path, accountId }) as MailFolder;
  const inbox = at(1, "Inbox", "inbox");
  const saved = at(20, "Saved", "user");
  const sub = at(21, "Saved/Old", "user", 20);
  const inInbox = at(30, "Inbox/Passwords", "user", 10);
  const other = at(40, "Elsewhere", "user", null, 2);
  const folder = (x: MailFolder) => ({ kind: "folder", folder: x, mailbox: {} }) as never;
  const mailbox = (accountId: number) => ({ kind: "mailbox", mailbox: { accountId } }) as never;

  it("goes into the Inbox, another folder or the top of its mailbox", () => {
    expect(folderTarget(saved, folder(inbox))).toBe(true);
    expect(folderTarget(inInbox, folder(saved))).toBe(true);
    expect(folderTarget(inInbox, mailbox(1))).toBe(true);
    expect(folderTarget(sub, folder(inbox))).toBe(true);
  });

  it("not where it already is, into itself, another mailbox or a system folder", () => {
    expect(folderTarget(inInbox, folder(inbox))).toBe(false);
    expect(folderTarget(saved, mailbox(1))).toBe(false);
    expect(folderTarget(sub, folder(saved))).toBe(false);
    expect(folderTarget(saved, folder(sub))).toBe(false);
    expect(folderTarget(saved, folder(saved))).toBe(false);
    expect(folderTarget(saved, folder(other))).toBe(false);
    expect(folderTarget(saved, mailbox(2))).toBe(false);
    expect(folderTarget(saved, folder(at(2, "Sent Items", "sent")))).toBe(false);
  });
});
