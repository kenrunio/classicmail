import { describe, expect, it } from "vitest";
import { useShell } from "../../shell/store";
import { dayStart } from "./dates";
import { useCal } from "./state";

describe("opening the Calendar", () => {
  it("shows this week in the Week view, wherever it was left", () => {
    useShell.getState().setModule("calendar");
    useCal.getState().goTo(Date.UTC(2025, 0, 15), "month");
    useCal.getState().set({ search: "board" });
    useShell.getState().setModule("mail");
    useShell.getState().setModule("calendar");
    expect(useCal.getState()).toMatchObject({ view: "week", anchor: dayStart(Date.now()).getTime(), search: "" });
  });

  it("a date picked in the To-Do Bar still opens that day", () => {
    useShell.getState().setModule("tasks");
    const day = dayStart(Date.UTC(2026, 11, 3)).getTime();
    useShell.getState().setModule("calendar");
    useCal.getState().goTo(day, "day");
    expect(useCal.getState()).toMatchObject({ view: "day", anchor: day });
  });

  it("doesn't keep the view between starts", () => {
    const saved = useCal.persist.getOptions().partialize?.(useCal.getState()) as Record<string, unknown>;
    expect(saved).not.toHaveProperty("view");
  });
});
