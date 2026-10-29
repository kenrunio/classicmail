import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MenuBar, MenuHost, type MenuBarMenu } from "./menu";

const copied = vi.fn();
const pasted = vi.fn();

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const MENUS: MenuBarMenu[] = [
  { label: "&File", items: [{ label: "Exit" }] },
  { label: "&Edit", items: () => [{ label: "Copy", onClick: copied }, { label: "Cut" }, { label: "Paste Special", submenu: [{ label: "Text", onClick: pasted }] }] },
  { label: "&View", items: [{ label: "Refresh" }] },
];

let root: Root;
beforeEach(() => {
  const host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() =>
    root.render(
      <>
        <MenuBar menus={MENUS} />
        <MenuHost />
      </>,
    ),
  );
});
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

const item = (text: string) => [...document.querySelectorAll<HTMLElement>(".cm-menubar-item")].find((e) => e.textContent === text)!;
const shown = () => [...document.querySelectorAll(".cm-menu-label")].map((e) => e.textContent);
const press = (el: HTMLElement) => act(() => void el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
// React makes onMouseEnter from the mouseout of the element the pointer left
const hover = (el: HTMLElement, from: HTMLElement) => act(() => void from.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: el })));

describe("MenuBar", () => {
  it("keeps following the pointer from menu to menu", () => {
    press(item("File"));
    expect(shown()).toEqual(["Exit"]);
    hover(item("Edit"), item("File"));
    expect(shown()).toEqual(["Copy", "Cut", "Paste Special"]);
    expect(item("Edit").classList.contains("open")).toBe(true);
    // the bar still knows a menu is open, so the next hover switches again
    hover(item("View"), item("Edit"));
    expect(shown()).toEqual(["Refresh"]);
    expect(item("View").classList.contains("open")).toBe(true);
  });

  it("closes the open menu when its title is clicked again", () => {
    press(item("File"));
    hover(item("Edit"), item("File"));
    press(item("Edit"));
    expect(shown()).toEqual([]);
  });
});

const key = (k: string, opts: KeyboardEventInit = {}) => act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts })));
const active = () => document.querySelector(".cm-menu-item.active .cm-menu-label")?.textContent;

describe("menus from the keyboard", () => {
  it("Alt+letter opens a menu with its first item highlighted; Left and Right move along the bar", () => {
    key("e", { altKey: true });
    expect(shown()).toEqual(["Copy", "Cut", "Paste Special"]);
    expect(active()).toBe("Copy");
    key("ArrowRight");
    expect(shown()).toEqual(["Refresh"]);
    key("ArrowRight");
    expect(shown()).toEqual(["Exit"]);
    key("ArrowLeft");
    expect(shown()).toEqual(["Refresh"]);
  });

  it("works with a Hebrew keyboard: Alt+ק is Alt+E", () => {
    key("ק", { altKey: true, code: "KeyE" });
    expect(shown()).toEqual(["Copy", "Cut", "Paste Special"]);
  });

  it("a menu item's underlined letter chooses it, and opens a submenu", () => {
    key("e", { altKey: true });
    // Copy takes C, so Cut gets U
    expect(document.querySelectorAll(".cm-menu-label u")[1].textContent).toBe("u");
    key("c");
    expect(copied).toHaveBeenCalledTimes(1);
    expect(shown()).toEqual([]);
    key("e", { altKey: true });
    key("p");
    key("t");
    expect(pasted).toHaveBeenCalledTimes(1);
  });

  it("F10 opens the first menu and closes it again", () => {
    key("F10");
    expect(shown()).toEqual(["Exit"]);
    key("F10");
    expect(shown()).toEqual([]);
  });
});
