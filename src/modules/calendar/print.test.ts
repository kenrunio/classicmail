import { afterEach, describe, expect, it, vi } from "vitest";
import { printPage } from "./print";

describe("printPage", () => {
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("writes an escaped, script-free page into a hidden frame", () => {
    vi.useFakeTimers();
    printPage('Q3 <results> & "notes"', "body{margin:0}", "<h1>Hi</h1>");
    const f = document.querySelector("iframe")!;
    const doc = f.contentDocument!;
    expect(doc.title).toBe('Q3 <results> & "notes"');
    expect(doc.body.innerHTML).toBe("<h1>Hi</h1>");
    expect(doc.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute("content")).toBe("default-src 'none'; style-src 'unsafe-inline'");
  });
});
