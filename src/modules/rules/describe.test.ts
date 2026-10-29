import { describe, expect, it } from "vitest";
import type { Rule } from "../../api/rules";
import { describeRule, isUnset, lineText, suggestName, valueText } from "./describe";
import { normalizeFolders, parsePeople } from "./editors";

const rule: Rule = {
  id: 1,
  name: "Harbor",
  enabled: true,
  position: 0,
  accountId: 1,
  conditions: [
    {
      type: "from",
      people: [
        { name: "Sarah Cohen", email: "sarah@h.example" },
        { name: "", email: "david@m.example" },
      ],
    },
    { type: "subjectContains", words: ["Q3", "earnings"] },
  ],
  actions: [{ type: "moveToFolder", folderId: 4, folderPath: "Clients/Orion" }, { type: "markRead" }, { type: "stopProcessing" }],
  exceptions: [{ type: "hasAttachment" }, { type: "importance", level: "low" }],
};

describe("rule description", () => {
  it("reads like Outlook", () => {
    const lines = describeRule(rule).map(lineText);
    expect(lines).toEqual([
      "Apply this rule after the message arrives",
      "from Sarah Cohen or david@m.example",
      'and with "Q3" or "earnings" in the subject',
      "move it to the Orion folder",
      "and mark it as read",
      "and stop processing more rules",
      "except if it has an attachment",
      "or except if it is marked as Low",
    ]);
  });

  it("marks unset values and uses lookups", () => {
    const blank = describeRule({ conditions: [{ type: "from", people: [] }], actions: [{ type: "copyToFolder", folderId: 0, folderPath: "" }], exceptions: [] });
    const seg = blank[1][1];
    expect("value" in seg && seg.value).toBe("people or distribution list");
    expect("unset" in seg && seg.unset).toBe(true);
    expect(isUnset({ type: "copyToFolder", folderId: 0, folderPath: "" })).toBe(true);
    expect(valueText({ type: "moveToFolder", folderId: 4, folderPath: "x" }, { folderName: () => "Orion" })).toBe("Orion");
    expect(valueText({ type: "throughAccount", accountId: 2 }, { accountEmail: () => "info@duvz.demo" })).toBe("info@duvz.demo");
  });

  it("suggests a name", () => {
    expect(suggestName(rule)).toBe("Sarah Cohen");
    expect(suggestName({ conditions: [], actions: [] })).toBe("New rule");
  });
});

describe("editors helpers", () => {
  it("parses people", () => {
    expect(parsePeople('Sarah Cohen <sarah@h.example>; david@m.example, "Levi, D" <d@x.example>\nMaya')).toEqual([
      { name: "Sarah Cohen", email: "sarah@h.example" },
      { name: "", email: "david@m.example" },
      { name: "Levi, D", email: "d@x.example" },
      { name: "Maya", email: "" },
    ]);
  });

  it("normalises folder lists of any shape", () => {
    const flat = normalizeFolders([{ id: 1, accountId: 1, name: "Inbox", path: "INBOX", role: "inbox", parentId: null }]);
    expect(flat).toHaveLength(1);
    const grouped = normalizeFolders([
      { accountId: 2, email: "info@x", folders: [{ id: 5, name: "Clients", path: "Clients", role: "user", children: [{ id: 6, name: "Orion", path: "Clients/Orion" }] }] },
    ]);
    expect(grouped.map((f) => [f.id, f.accountId, f.parentId])).toEqual([
      [5, 2, null],
      [6, 2, 5],
    ]);
  });
});
