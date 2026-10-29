import { describe, expect, it } from "vitest";
import type { MenuItem } from "../../ui";
import { categorizeMenu } from "./actions";
import { taskItem } from "./store";
import type { Task } from "../../api/tasks";

const task = (id: number, categories: string[]) => taskItem({ id, title: `T${id}`, categories, listTitle: "Tasks", status: "notStarted" } as unknown as Task);
const item = (m: MenuItem[], label: string) => m.find((x) => x.type !== "separator" && x.type !== "header" && x.label === label) as Extract<MenuItem, { icon?: string }>;

describe("tasks Categorize menu", () => {
  it("offers the Color Categories with their colours, without the Calendar's shortcut keys", () => {
    const m = categorizeMenu([task(1, ["red category"])]);
    const red = item(m, "Red Category");
    expect(red.icon).toBe("cm-cat-1");
    expect(red.checked).toBe(true); // names compare without case
    expect(m.every((x) => x.type === "separator" || x.type === "header" || !x.shortcut)).toBe(true);
    expect(item(m, "All Categories…")).toBeTruthy();
  });

  it("ticks a category for several tasks only when all of them have it", () => {
    const m = categorizeMenu([task(1, ["Blue Category"]), task(2, ["Blue Category", "Green Category"])]);
    expect(item(m, "Blue Category").checked).toBe(true);
    expect(item(m, "Green Category").checked).toBe(false);
  });
});
