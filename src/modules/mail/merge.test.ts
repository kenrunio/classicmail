import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../api/windows", () => ({ openWindow: vi.fn() }));
vi.mock("../../ui", () => ({ errorBox: vi.fn() }));
vi.mock("../../api/rpc", () => ({ rpc: vi.fn() }));

import { preflightText } from "./merge";

const base = { messages: 3, skipped: [], duplicates: [], unknownFields: [], blankFields: [], overLimit: false };

describe("preflightText", () => {
  it("summarises a clean send", () => {
    expect(preflightText({ ...base, messages: 300 }, "send")).toBe("300 messages will be sent, one every few seconds (about 30 min). You can follow them in the Outbox.");
    expect(preflightText({ ...base, messages: 1 }, "drafts")).toBe("1 draft will be saved to Drafts.");
  });

  it("lists problems, capped at ten rows", () => {
    const skipped = Array.from({ length: 12 }, (_, i) => ({ row: i + 2, email: i ? "bad" : "" }));
    const t = preflightText(
      { ...base, skipped, duplicates: [{ row: 20, email: "a@b.com" }], blankFields: [{ row: 21, email: "c@d.com", fields: ["First Name", "Company"] }], overLimit: true },
      "send",
    );
    expect(t).toContain("about 2,000 recipients a day");
    expect(t).toContain("  Row 2: (blank)\n  Row 3: bad");
    expect(t).toContain("  …and 2 more");
    expect(t).not.toContain("Row 12:");
    expect(t).toContain("Row 20: a@b.com");
    expect(t).toContain("Row 21: c@d.com — First Name, Company");
  });
});
