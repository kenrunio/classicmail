import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("../../api/rpc", () => ({ rpc: async () => ({ categories: [{ name: "Red Category", color: 1, shortcut: "" }] }) }));

import { ColorCategoriesHost, openColorCategories } from "./categories";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  document.body.innerHTML = "";
});

describe("ColorCategoriesHost", () => {
  it("shows one dialog when several hosts are mounted (Calendar view + To-Do Bar)", async () => {
    const qc = new QueryClient();
    const mount = () => {
      const el = document.createElement("div");
      document.body.appendChild(el);
      const r = createRoot(el);
      act(() =>
        r.render(
          <QueryClientProvider client={qc}>
            <ColorCategoriesHost />
          </QueryClientProvider>,
        ),
      );
      return r;
    };
    const a = mount();
    const b = mount();
    await act(async () => {
      void openColorCategories(null);
    });
    expect(document.querySelectorAll(".cm-cal-catdlg").length).toBe(1);
    // the first host goes away: the other one takes the open dialog over
    act(() => a.unmount());
    expect(document.querySelectorAll(".cm-cal-catdlg").length).toBe(1);
    act(() => b.unmount());
  });
});
