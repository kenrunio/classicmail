import { describe, expect, it } from "vitest";
import { findGrids, findLists, findTables, GRID_MAX_COL, notesParts, tableKinds, tableToBlocks, tableToGrid, tidyNotes } from "./notesTable";

// a meeting-notes table (tab-separated, as pasted)
const TSV = [
  "#\tFirm\tInvestor\tLast met Orion\tCRM mtgs\t$ ORIN at Q2-end",
  "1\tNorthgate Advisors\tMichael Brennan\t14 Jan 26 (Atlas Growth Conf)\t12\t—",
  "2\tBluewater Fund (Boston)\tJames Whitaker\tNot in CRM\t0\t$0.04M (280 sh)",
  "3\tSeine\tSophie Laurent\t\t0\t",
].join("\n");

describe("tableToBlocks", () => {
  it("numbers rows, titles them with the first two columns and drops blank / dash cells", () => {
    const grid = TSV.split("\n").map((l) => l.split("\t"));
    expect(tableToBlocks(grid)).toBe(
      [
        "Table header: # | Firm | Investor | Last met Orion | CRM mtgs | $ ORIN at Q2-end",
        "",
        "1. Northgate Advisors — Michael Brennan",
        "   Last met Orion: 14 Jan 26 (Atlas Growth Conf)",
        "   CRM mtgs: 12",
        "",
        "2. Bluewater Fund (Boston) — James Whitaker",
        "   Last met Orion: Not in CRM",
        "   CRM mtgs: 0",
        "   $ ORIN at Q2-end: $0.04M (280 sh)",
        "",
        "3. Seine — Sophie Laurent",
        "   CRM mtgs: 0",
      ].join("\n"),
    );
  });

  it("uses only the first column as the title for narrow tables, numbers the rows, and keeps Hebrew", () => {
    expect(
      tableToBlocks([
        ["משקיע", "קרן"],
        ["דני", "מגדל"],
        ["רות", ""],
      ]),
    ).toBe("Table header: משקיע | קרן\n\n1. דני\n   קרן: מגדל\n\n2. רות");
  });

  it("treats a numeric data column as data, an untitled 1-2-3 column as numbering", () => {
    expect(
      tableToBlocks([
        ["Year", "Revenue"],
        ["2025", "$10M"],
        ["2026", "$12M"],
      ]),
    ).toBe("Table header: Year | Revenue\n\n1. 2025\n   Revenue: $10M\n\n2. 2026\n   Revenue: $12M");
    expect(
      tableToBlocks([
        ["", "Firm"],
        ["1", "Northgate"],
        ["2", "Seine"],
      ]),
    ).toBe("Table header: # | Firm\n\n1. Northgate\n\n2. Seine");
  });

  it("skips empty rows and shows an empty title as a dash", () => {
    expect(
      tableToBlocks([
        ["Name", "Note"],
        ["", ""],
        ["—", "follow up"],
      ]),
    ).toBe("Table header: Name | Note\n\n1. —\n   Note: follow up");
  });
});

describe("findTables / tidyNotes", () => {
  it("converts a tab-separated table and leaves the notes around it alone", () => {
    const notes = `Attendees:\n${TSV}\n*\nBluewater is the big story.`;
    const r = tidyNotes(notes);
    expect(r.count).toBe(1);
    expect(r.text.startsWith("Attendees:\n\nTable header: # | Firm | Investor")).toBe(true);
    expect(r.text).toContain("\n\n1. Northgate Advisors — Michael Brennan\n");
    expect(r.text.endsWith("   CRM mtgs: 0\n\n*\nBluewater is the big story.")).toBe(true);
  });

  it("converts Markdown tables", () => {
    const md = "| Firm | Investor | Shares | Change |\n|---|:---:|---|---|\n| Rhein AM | Thomas Meyer | 972.2k | — |";
    expect(tidyNotes(md).text).toBe("Table header: Firm | Investor | Shares | Change\n\n1. Rhein AM — Thomas Meyer\n   Shares: 972.2k");
  });

  it("ignores ordinary text, a single tabbed line and footnote lines", () => {
    expect(tidyNotes("Dial in:\t+972 3 555\nThanks").count).toBe(0);
    expect(findTables(["just text", "* note", "| not | a table |"])).toEqual([]);
  });

  it("keeps Windows line endings", () => {
    expect(tidyNotes("a\tb\r\n1\t2").text).toBe("Table header: a | b\r\n\r\n1. 1\r\n   b: 2");
  });
});

describe("switching between list and grid", () => {
  it("list → grid → list gives the same text, and keeps the notes around it", () => {
    const notes = `Attendees:\n${WIDE_TSV()}\n\nBluewater is the big story.`;
    const list = tidyNotes(notes, "list").text;
    expect(tableKinds(list)).toEqual({ raw: 0, grid: 0, list: 1 });
    const grid = tidyNotes(list, "grid");
    expect(grid.count).toBe(1);
    expect(tableKinds(grid.text)).toEqual({ raw: 0, grid: 1, list: 0 });
    expect(grid.text).toBe(tidyNotes(notes, "grid").text);
    expect(tidyNotes(grid.text, "list").text).toBe(list);
    expect(grid.text.endsWith("\n\nBluewater is the big story.")).toBe(true);
  });

  it("reads a list back into the same cells (incl. a row with a blank title)", () => {
    const g = [
      ["Name", "Firm", "Role", "Note"],
      ["Dan", "Harbor — East", "CFO", ""],
      ["", "", "", "only a note"],
    ];
    const found = findLists(tableToBlocks(g).split("\n"));
    expect(found[0].grid).toEqual([g[0], ["Dan", "Harbor — East", "CFO", ""], ["", "", "", "only a note"]]);
  });

  it("a blank # stays blank when switching list → grid", () => {
    const g = [
      ["#", "Firm", "Investor", "Note", "More"],
      ["1", "Northgate", "Michael", "x", "y"],
      ["", "Seine", "Sophie", "z", "w"],
    ];
    const list = tableToBlocks(g);
    expect(list).toContain("\n—. Seine — Sophie\n");
    expect(findLists(list.split("\n"))[0].grid).toEqual(g);
  });

  it("a numbered paragraph after a blank line ends nothing it shouldn't", () => {
    const text = `${tableToBlocks([
      ["A", "B"],
      ["x", "1"],
    ])}\n\nThanks for the meeting.`;
    const f = findLists(text.split("\n"));
    expect(f[0].grid).toEqual([
      ["A", "B"],
      ["x", "1"],
    ]);
    expect(text.split("\n").slice(f[0].to).join("\n")).toBe("\nThanks for the meeting.");
  });
});

function WIDE_TSV() {
  return [
    "#\tFirm\tInvestor\tLast met Orion\tCRM mtgs\t$ ORIN at Q2-end\tShare change since Q1-end\t$ NGEV at Q2-end\t",
    "1\tWestpark\tDaniel Grant\tNot in CRM. Firm: 16 Sep 26 (Apex TMT)\t0\t$0, sold out\tExited\t$41.3M (120.4k sh, down from 168.9k)\t",
    "2\tWestpark\tPeter Lindqvist\t16 Sep 26 (Apex TMT)\t17\tSame firm as row 1\t\t\t",
    "3\tKingsbridge Capital\tRavi Menon\tNot in CRM. Firm: 27 Aug 26\t0\t— (this entity holds no Orion)\t—\t$96.8M (251.2k sh, down from 310.5k)\t",
    "4\tEastgate\tNoah Feldman\t15 Sep 26 (Atlas Tech Conf)\t2\t—\t—\t$63.4M (172.1k sh, down from 240.3k)",
  ].join("\n");
}

// a wider meeting table (tab-separated, with the trailing tab Excel / Word leave)
const WIDE = [
  "#\tFirm\tInvestor\tLast met Orion\tCRM mtgs\t$ ORIN at Q2-end\tShare change since Q1-end\t$ NGEV at Q2-end\t",
  "1\tWestpark\tDaniel Grant\tNot in CRM. Firm: 16 Sep 26 (Apex TMT)\t0\t$0, sold out\tExited\t$41.3M (120.4k sh, down from 168.9k)\t",
  "2\tWestpark\tPeter Lindqvist\t16 Sep 26 (Apex TMT)\t17\tSame firm as row 1\t\t\t",
  "3\tKingsbridge Capital\tRavi Menon\tNot in CRM. Firm: 27 Aug 26\t0\t— (this entity holds no Orion)\t—\t$96.8M (251.2k sh, down from 310.5k)\t",
  "4\tEastgate\tNoah Feldman\t15 Sep 26 (Atlas Tech Conf)\t2\t—\t—\t$63.4M (172.1k sh, down from 240.3k)",
].join("\n");

describe("grid", () => {
  it("lays a small table out in fixed-width columns, numbers right-aligned", () => {
    expect(
      tableToGrid([
        ["#", "Firm", "CRM mtgs"],
        ["1", "Westpark", "0"],
        ["2", "Eastgate", "17"],
      ]),
    ).toBe(["# | Firm     | CRM mtgs", "--+----------+---------", "1 | Westpark |        0", "2 | Eastgate |       17"].join("\n"));
  });

  it("wraps long cells inside their column and keeps every line within the column positions", () => {
    const r = tidyNotes(`Attendees:\n${WIDE}\nBluewater is the big story.`, "grid");
    expect(r.count).toBe(1);
    const lines = r.text.split("\n");
    expect(lines[0]).toBe("Attendees:");
    expect(lines[lines.length - 1]).toBe("Bluewater is the big story.");
    const grid = lines.slice(2, -2);
    expect(Math.max(...grid.map((l) => l.length))).toBeLessThanOrEqual(8 * GRID_MAX_COL + 7 * 3);
    // the empty trailing column is gone, and long text wrapped onto continuation lines
    expect(grid[0].startsWith("# | Firm")).toBe(true);
    expect(grid.some((l) => l.startsWith("  | ") && l.includes("Apex TMT)"))).toBe(true);
  });

  it("reads its own grid back into the same cells", () => {
    const text = tidyNotes(WIDE, "grid").text;
    const found = findGrids(text.split("\n"));
    expect(found).toHaveLength(1);
    const g = found[0].grid;
    expect(g[0]).toEqual(["#", "Firm", "Investor", "Last met Orion", "CRM mtgs", "$ ORIN at Q2-end", "Share change since Q1-end", "$ NGEV at Q2-end"]);
    expect(g[1]).toEqual(["1", "Westpark", "Daniel Grant", "Not in CRM. Firm: 16 Sep 26 (Apex TMT)", "0", "$0, sold out", "Exited", "$41.3M (120.4k sh, down from 168.9k)"]);
    expect(g[2]).toEqual(["2", "Westpark", "Peter Lindqvist", "16 Sep 26 (Apex TMT)", "17", "Same firm as row 1", "", ""]);
    expect(g).toHaveLength(5);
    expect(found[0].numeric[4]).toBe(true); // CRM mtgs
    expect(found[0].numeric[1]).toBe(false);
  });

  it("keeps a row whose first cell is empty as its own row", () => {
    const text = tableToGrid([
      ["Name", "Note"],
      ["", "first"],
      ["Dan", "second"],
    ]);
    expect(findGrids(text.split("\n"))[0].grid).toEqual([
      ["Name", "Note"],
      ["", "first"],
      ["Dan", "second"],
    ]);
  });

  it("splits notes into text and tables", () => {
    const text = `Before\n${tableToGrid([
      ["A", "B"],
      ["1", "x"],
    ])}\nAfter`;
    const parts = notesParts(text);
    expect(parts.map((p) => p.kind)).toEqual(["text", "table", "text"]);
    expect(parts[0]).toEqual({ kind: "text", text: "Before" });
    expect(notesParts("just | some text\n--- not a rule")).toEqual([{ kind: "text", text: "just | some text\n--- not a rule" }]);
  });

  it("list style drops an empty trailing column too", () => {
    expect(tidyNotes("A\tB\t\n1\tx\t").text).toBe("Table header: A | B\n\n1. 1\n   B: x");
  });
});
