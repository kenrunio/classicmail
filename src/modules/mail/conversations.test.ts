import { describe, expect, it } from "vitest";
import type { MessageRow } from "../../api/mail";
import { buildLines, childKey, groupsWithout, idsOfRows, resolveSelection, rowFlagged, rowKey } from "./conversations";

function row(id: number, conv?: Partial<NonNullable<MessageRow["conv"]>>): MessageRow {
  return {
    id,
    accountId: 1,
    threadId: `t${id}`,
    subject: "s",
    fromName: "A",
    fromEmail: "a@x",
    toDisplay: "",
    date: 0,
    size: 1,
    isRead: true,
    isFlagged: false,
    flagStatus: null,
    flagDue: null,
    hasAttachments: false,
    importance: 1,
    lastVerb: null,
    isDraft: false,
    isInvite: false,
    preview: "",
    folder: null,
    outboxId: null,
    outboxStatus: null,
    conv: conv ? { key: `1:t${id}`, count: 1, total: 1, unread: 0, size: 1, hasAttachments: false, importance: 1, flag: 0, participants: "A", ids: [id], ...conv } : undefined,
  };
}

describe("buildLines", () => {
  it("lists rows with expanded children", () => {
    const lines = buildLines(3, null, [], new Map([[1, 2]]));
    expect(lines).toEqual([
      { kind: "row", index: 0 },
      { kind: "row", index: 1 },
      { kind: "child", index: 1, ci: 0 },
      { kind: "child", index: 1, ci: 1 },
      { kind: "row", index: 2 },
    ]);
  });
  it("puts rows in groups and hides collapsed groups (and their children)", () => {
    const groups = [
      { key: "0", label: "Today", count: 2 },
      { key: "1", label: "Yesterday", count: 1 },
      { key: "5", label: "Older", count: 2 },
    ];
    const lines = buildLines(
      5,
      groups,
      ["1"],
      new Map([
        [0, 1],
        [2, 3],
        [4, 1],
      ]),
    );
    expect(lines.map((l) => (l.kind === "group" ? `g${l.g.key}${l.collapsed ? "-" : "+"}` : l.kind === "row" ? `r${l.index}` : `c${l.index}.${l.ci}`))).toEqual([
      "g0+",
      "r0",
      "c0.0",
      "r1",
      "g1-",
      "g5+",
      "r3",
      "r4",
      "c4.0",
    ]);
  });
});

describe("groupsWithout", () => {
  const g = (key: string, count: number) => ({ key, label: key, count });
  it("lowers the count of the group each removed row was in, and drops an emptied group", () => {
    const groups = [g("today", 3), g("yesterday", 1), g("lastWeek", 4)];
    // row 1 (today) and row 3 (yesterday's only row) archived
    expect(groupsWithout(groups, [1, 3])).toEqual([g("today", 2), g("lastWeek", 4)]);
    expect(groupsWithout(groups, [7])).toEqual([g("today", 3), g("yesterday", 1), g("lastWeek", 3)]);
    expect(groupsWithout(groups, [])).toEqual(groups);
  });
});

describe("selection", () => {
  const conv = row(10, { ids: [10, 7, 3], count: 3, total: 4, flag: 2 });
  const plain = row(20);
  const kid = row(7);
  const byKey = new Map<string, MessageRow>([
    [rowKey(conv), conv],
    [rowKey(plain), plain],
    [childKey(kid), kid],
  ]);
  it("keys", () => {
    expect(rowKey(conv)).toBe("c:1:t10");
    expect(rowKey(plain)).toBe("m:20");
    expect(childKey(kid)).toBe("t:7");
  });
  it("a conversation header acts on all its messages in the view and shows the newest", () => {
    expect(resolveSelection(["c:1:t10"], "c:1:t10", byKey)).toEqual({ selected: [10, 7, 3], focused: 10 });
  });
  it("a child row is just that message; overlaps are counted once", () => {
    expect(resolveSelection(["t:7"], "t:7", byKey)).toEqual({ selected: [7], focused: 7 });
    expect(resolveSelection(["c:1:t10", "t:7", "m:20"], "m:20", byKey)).toEqual({ selected: [10, 7, 3, 20], focused: 20 });
  });
  it("unloaded plain keys still resolve; unknown conversations don't", () => {
    expect(resolveSelection(["m:99", "c:1:zz"], "m:99", byKey)).toEqual({ selected: [99], focused: 99 });
  });
  it("flags and ids of rows", () => {
    expect(rowFlagged(conv)).toBe(true);
    expect(rowFlagged(plain)).toBe(false);
    expect(idsOfRows([conv, plain, kid])).toEqual([10, 7, 3, 20]);
  });
});
