import { describe, expect, it, vi } from "vitest";

const calls: string[] = [];
vi.mock("./rpc", () => ({ rpc: vi.fn(async (method: string) => (calls.push(`rpc ${method}`), { found: false })) }));
vi.mock("./aiConsent", () => ({
  withAiConsent: vi.fn(async (call: () => Promise<unknown>) => {
    calls.push("consent");
    return call();
  }),
}));

import { fromMessage } from "./aiSchedule";

describe("fromMessage", () => {
  it("asks for AI consent first, like the other AI buttons", async () => {
    await fromMessage(42);
    expect(calls).toEqual(["consent", "rpc ai.schedule.fromMessage"]);
  });
});
