import { beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.fn();
vi.mock("./rpc", () => ({ rpc: (...a: unknown[]) => rpc(...a) }));

import { queryClient } from "./query";
import { setSetting } from "./settings";

const cached = () => queryClient.getQueryData<Record<string, unknown>>(["settings", "all"]);

describe("setSetting", () => {
  beforeEach(() => {
    rpc.mockReset();
    queryClient.clear();
    queryClient.setQueryData(["settings", "all"], { "mail.pollSeconds": 60 });
  });

  it("shows the new value at once and keeps it when saved", async () => {
    rpc.mockResolvedValue(null);
    const p = setSetting("mail.pollSeconds", 120);
    expect(cached()?.["mail.pollSeconds"]).toBe(120);
    await p;
    expect(rpc).toHaveBeenCalledWith("settings.set", { key: "mail.pollSeconds", value: 120 });
    expect(cached()?.["mail.pollSeconds"]).toBe(120);
  });

  it("puts the stored value back when the save fails", async () => {
    rpc.mockImplementation(async (method: string) => {
      if (method === "settings.set") throw new Error("disk full");
      return { "mail.pollSeconds": 60 };
    });
    await expect(setSetting("mail.pollSeconds", 120)).rejects.toThrow("disk full");
    expect(cached()?.["mail.pollSeconds"]).toBe(60);
  });
});
