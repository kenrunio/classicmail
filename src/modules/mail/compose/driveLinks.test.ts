import { describe, expect, it } from "vitest";
import { LINK_ABOVE, planLinks, uploadNote } from "./driveLinks";

const MB = 1024 * 1024;

describe("planLinks", () => {
  it("sends a file over 25 MB as a link and attaches the rest", () => {
    expect([...planLinks(0, [700 * MB, 2 * MB])]).toEqual([0]);
  });
  it("leaves files that fit together as attachments", () => {
    expect(planLinks(5 * MB, [10 * MB, 10 * MB]).size).toBe(0);
    expect(planLinks(0, [LINK_ABOVE]).size).toBe(0);
  });
  it("sends the largest as links until the rest fits beside what is attached", () => {
    // 3 × 10 MB: one goes as a link; with 20 MB attached already, two do — the largest first
    expect(planLinks(0, [10 * MB, 10 * MB, 10 * MB]).size).toBe(1);
    expect([...planLinks(20 * MB, [3 * MB, 9 * MB, 4 * MB])].sort()).toEqual([1, 2]);
  });
});

describe("uploadNote", () => {
  const u = { id: 1, accountId: 1, path: "", name: "a", size: 200, sent: 50, status: "uploading" as const };
  it("shows progress, then link, or the failure", () => {
    expect(uploadNote(u, 1)).toBe("25%");
    expect(uploadNote({ ...u, status: "done" }, 1)).toBe("link");
    expect(uploadNote({ ...u, status: "failed" }, 1)).toBe("upload failed");
    expect(uploadNote(undefined, null)).toBe("link");
  });
});
