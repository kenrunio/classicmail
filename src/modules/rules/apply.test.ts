import { describe, expect, it, vi } from "vitest";
import type { Rule } from "../../api/rules";
import { applyRules, withSavedIds, type ApplyApi, type ApplyProgress } from "./apply";

const rule = (id: number, name = `r${id}`): Rule => ({ id, name, enabled: true, position: 0, accountId: null, conditions: [], exceptions: [], actions: [] });

/** A back end that gives new rules ids from 100 and fails the save of the rule named `failOn`. */
function fakeApi(failOn?: string) {
  let next = 100;
  const api = {
    save: vi.fn(async (r: Rule) => {
      if (r.name === failOn) throw new Error("save failed");
      return { id: r.id || next++ };
    }),
    delete: vi.fn(async () => 1),
    reorder: vi.fn(async () => {}),
  } satisfies ApplyApi;
  return api;
}

const fresh = (): ApplyProgress => ({ ids: new Map(), deletedDone: false });

describe("applyRules", () => {
  it("creates new rules, saves only changed ones, then stores the order", async () => {
    const api = fakeApi();
    const p = fresh();
    const saved = [rule(1), rule(2)];
    await applyRules([rule(1), rule(2, "renamed"), rule(-1)], [3], saved, api, p);
    expect(api.delete).toHaveBeenCalledWith([3]);
    expect(api.save.mock.calls.map(([r]) => r.name)).toEqual(["renamed", "r-1"]);
    expect(api.reorder).toHaveBeenCalledWith([1, 2, 100]);
    expect([...p.ids]).toEqual([
      [1, 1],
      [2, 2],
      [-1, 100],
    ]);
  });

  it("after a failure part way, the next Apply neither creates nor deletes again", async () => {
    const api = fakeApi("bad");
    const p = fresh();
    const list = [rule(-1), rule(-2, "bad"), rule(5)];
    await expect(applyRules(list, [3], [rule(5)], api, p)).rejects.toThrow("save failed");
    expect(p.deletedDone).toBe(true);

    // what the window keeps: -1 is now rule 100; -2 is still new
    const kept = withSavedIds(list, p.ids);
    expect(kept.map((r) => r.id)).toEqual([100, -2, 5]);

    // the user fixes the bad rule and applies again, against the refreshed saved list
    const retry = fakeApi();
    kept[1] = { ...kept[1], name: "good" };
    await applyRules(kept, [], [rule(100, "r-1"), rule(5)], retry, fresh());
    expect(retry.delete).not.toHaveBeenCalled();
    expect(retry.save.mock.calls.map(([r]) => r.name)).toEqual(["good"]);
  });
});

describe("withSavedIds", () => {
  it("leaves rules that were not saved alone", () => {
    const list = [rule(-1), rule(7)];
    expect(withSavedIds(list, new Map())).toEqual(list);
  });
});
