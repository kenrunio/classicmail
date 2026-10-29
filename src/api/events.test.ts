import { afterEach, describe, expect, it, vi } from "vitest";

const invalidate = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("./query", () => ({ queryClient: { invalidateQueries: (o: unknown) => invalidate(o) } }));

import { invalidateSoon } from "./events";

describe("invalidateSoon", () => {
  afterEach(() => {
    vi.useRealTimers();
    invalidate.mockReset();
  });

  it("merges a burst into one invalidation per key", () => {
    vi.useFakeTimers();
    for (let i = 0; i < 20; i++) invalidateSoon(["mail"]);
    invalidateSoon(["todo"]);
    expect(invalidate).not.toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(invalidate.mock.calls.map((c) => c[0].queryKey)).toEqual([["mail"], ["todo"]]);
  });

  it("never waits more than a second during a long burst", () => {
    vi.useFakeTimers();
    for (let t = 0; t < 1500; t += 100) {
      invalidateSoon(["mail"]);
      vi.advanceTimersByTime(100);
    }
    // a steady stream every 100 ms still refreshes about once a second, not never
    expect(invalidate.mock.calls.length).toBeGreaterThanOrEqual(1);
    expect(invalidate.mock.calls.length).toBeLessThanOrEqual(2);
  });
});
