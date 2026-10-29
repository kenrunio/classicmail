import { beforeEach, describe, expect, it, vi } from "vitest";

const emitTo = vi.fn(() => Promise.resolve());
const rpc = vi.fn((..._a: unknown[]) => Promise.resolve());
vi.mock("@tauri-apps/api/event", () => ({ emitTo, listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("../../api/rpc", () => ({ rpc }));

const { findAll } = await import("./findAll");
const { useShell } = await import("../../shell/store");
const { searchHandoff, useMailSel } = await import("./store");

describe("findAll", () => {
  beforeEach(() => {
    emitTo.mockClear();
    rpc.mockClear();
    searchHandoff.keep = false;
    useMailSel.getState().setSearch("");
  });

  it("in the main window: shows Mail with the search in All Mail Items", () => {
    history.replaceState(null, "", "/");
    useShell.getState().setModule("calendar");
    findAll('from:"a@b.com"');
    expect(useShell.getState().module).toBe("mail");
    expect(useMailSel.getState()).toMatchObject({ search: 'from:"a@b.com"', scope: "all" });
    // the mail view appears now and keeps the search
    expect(searchHandoff.keep).toBe(true);
    expect(emitTo).not.toHaveBeenCalled();
  });

  it("in a message window: hands the search to the main window and brings it up", () => {
    history.replaceState(null, "", "/?window=message&id=5");
    findAll('from:"a@b.com"');
    expect(emitTo).toHaveBeenCalledWith("main", "cm-mail-find", 'from:"a@b.com"');
    expect(rpc).toHaveBeenCalledWith("app.showMain", {});
    expect(useMailSel.getState().search).toBe("");
  });
});
