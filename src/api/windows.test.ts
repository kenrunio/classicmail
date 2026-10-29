import { beforeEach, describe, expect, it, vi } from "vitest";

const created: string[] = [];
const registered = new Set<string>();

vi.mock("@tauri-apps/api/webviewWindow", () => {
  class WebviewWindow {
    label: string;
    private handlers: Record<string, () => void> = {};
    constructor(label: string) {
      this.label = label;
      created.push(label);
      // Tauri registers the window a little later, then fires tauri://created
      setTimeout(() => {
        registered.add(label);
        this.handlers["tauri://created"]?.();
      }, 20);
    }
    static async getByLabel(label: string) {
      return registered.has(label) ? new FakeExisting(label) : null;
    }
    async once(ev: string, cb: () => void) {
      this.handlers[ev] = cb;
      return () => {};
    }
    async setFocus() {}
  }
  class FakeExisting {
    constructor(public label: string) {}
    async unminimize() {}
    async show() {}
    async setFocus() {}
  }
  return { WebviewWindow, getCurrentWebviewWindow: () => ({}) };
});

import { openWindow } from "./windows";

describe("openWindow", () => {
  beforeEach(() => {
    created.length = 0;
    registered.clear();
  });

  it("creates one window when the same label is opened twice at once", async () => {
    await Promise.all([openWindow("reminders", {}, { label: "reminders" }), openWindow("reminders", {}, { label: "reminders" })]);
    expect(created).toEqual(["reminders"]);
    // and a later open just focuses it
    await openWindow("reminders", {}, { label: "reminders" });
    expect(created).toEqual(["reminders"]);
  });

  it("still opens separate windows for different labels", async () => {
    await Promise.all([openWindow("compose"), openWindow("compose")]);
    expect(created.length).toBe(2);
  });
});
