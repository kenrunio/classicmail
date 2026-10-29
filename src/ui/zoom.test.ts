import { describe, expect, it, vi } from "vitest";

vi.mock("../api/settings", () => ({ useSettings: vi.fn(), setSetting: vi.fn() }));
vi.mock("../api/query", () => ({ queryClient: { getQueryData: vi.fn() } }));

import { stepZoom, ZOOM_STEPS } from "./zoom";

describe("stepZoom", () => {
  it("moves through Outlook-like steps and stops at the ends", () => {
    expect(stepZoom(100, 1)).toBe(110);
    expect(stepZoom(100, -1)).toBe(90);
    expect(stepZoom(125, 1)).toBe(150);
    expect(stepZoom(300, 1)).toBe(300);
    expect(stepZoom(50, -1)).toBe(50);
    expect(stepZoom(175, 0)).toBe(100);
  });
  it("snaps an odd stored value to the next step", () => {
    expect(stepZoom(117, 1)).toBe(125);
    expect(stepZoom(117, -1)).toBe(110);
    expect(ZOOM_STEPS).toContain(100);
  });
});
