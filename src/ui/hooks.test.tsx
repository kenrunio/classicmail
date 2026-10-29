import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useHotkeys } from "./hooks";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const hit = vi.fn();
function Keys() {
  useHotkeys({ f9: () => hit("f9"), "ctrl+r": () => hit("ctrl+r"), "ctrl+shift+v": () => hit("move"), delete: () => hit("delete"), enter: () => hit("enter") });
  return (
    <>
      <input />
      <button>OK</button>
    </>
  );
}

let root: Root;
beforeEach(() => {
  hit.mockClear();
  const host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(<Keys />));
});
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

const press = (el: Element, key: string, opts: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...opts });
  el.dispatchEvent(e);
  return e;
};
const input = () => document.querySelector("input")!;

describe("useHotkeys", () => {
  it("F-keys and Ctrl shortcuts work while typing in a box; plain keys don't", () => {
    press(input(), "F9");
    press(input(), "r", { ctrlKey: true, code: "KeyR" });
    press(input(), "Delete");
    expect(hit.mock.calls.map((c) => c[0])).toEqual(["f9", "ctrl+r"]);
  });

  it("Ctrl+R works with a Hebrew keyboard", () => {
    press(document.body, "ר", { ctrlKey: true, code: "KeyR" });
    expect(hit).toHaveBeenCalledWith("ctrl+r");
  });

  it("a text box keeps its editing keys", () => {
    const e = press(input(), "V", { ctrlKey: true, shiftKey: true, code: "KeyV" });
    expect(hit).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
    press(document.body, "V", { ctrlKey: true, shiftKey: true, code: "KeyV" });
    expect(hit).toHaveBeenCalledWith("move");
  });

  it("leaves a key alone that something else handled, or while a menu is open", () => {
    const e = new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true });
    e.preventDefault();
    document.body.dispatchEvent(e);
    const menu = document.createElement("div");
    menu.className = "cm-menu";
    document.body.appendChild(menu);
    press(document.body, "Delete");
    menu.remove();
    expect(hit).not.toHaveBeenCalled();
    press(document.body, "Delete");
    expect(hit).toHaveBeenCalledWith("delete");
  });

  it("Enter on a button presses the button", () => {
    press(document.querySelector("button")!, "Enter");
    expect(hit).not.toHaveBeenCalled();
    press(document.body, "Enter");
    expect(hit).toHaveBeenCalledWith("enter");
  });
});
