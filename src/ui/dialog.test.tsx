import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";

const settings = new Map<string, unknown>();
vi.mock("../api/rpc", () => ({
  rpc: async (method: string, p: { key: string; value?: unknown }) => {
    if (method === "settings.get") return settings.get(p.key) ?? null;
    if (method === "settings.set") settings.set(p.key, p.value);
    return null;
  },
}));

import { Dialog, MessageBoxHost, confirmBox, messageBox } from "./dialog";
import { accessKeys } from "./keys";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  settings.clear();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(<MessageBoxHost />));
});
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

const tick = () => act(async () => new Promise((r) => setTimeout(r, 0)));
const button = (label: string) => [...document.querySelectorAll("button")].find((b) => b.textContent === label)!;

async function answer(p: Promise<unknown>, label: string, dontAsk: boolean) {
  await tick();
  const box = document.querySelector<HTMLInputElement>(".cm-dialog-dontask input");
  expect(box, "the box offers Don't ask me again").not.toBeNull();
  if (dontAsk) act(() => box!.click());
  act(() => button(label).click());
  return p;
}

describe("Don't ask me again", () => {
  it("remembers the go-ahead answer and skips the box from then on", async () => {
    expect(await answer(confirmBox("Empty?", "ClassicMail", "warning", "mail.emptyFolder"), "Yes", true)).toBe(true);
    await tick();
    expect(settings.get("ui.dontAsk")).toEqual(["mail.emptyFolder"]);
    // next time: no box, straight to Yes
    expect(await confirmBox("Empty?", "ClassicMail", "warning", "mail.emptyFolder")).toBe(true);
    expect(document.querySelector(".cm-dialog-dontask")).toBeNull();
  });

  it("a ticked box answered No is not remembered", async () => {
    expect(await answer(confirmBox("Empty?", "ClassicMail", "warning", "mail.emptyFolder"), "No", true)).toBe(false);
    await tick();
    expect(settings.get("ui.dontAsk") ?? []).toEqual([]);
  });

  it("a warning with its own option turns that option off", async () => {
    const ask = () => messageBox({ title: "t", text: "Delete?", buttons: ["Yes", "No"], dontAsk: { setting: "mail.confirmPermanentDelete", answer: "Yes" } });
    expect(await answer(ask(), "Yes", true)).toBe("Yes");
    await tick();
    expect(settings.get("mail.confirmPermanentDelete")).toBe(false);
    expect(await ask()).toBe("Yes");
  });

  it("boxes without a key don't offer it", async () => {
    const p = confirmBox("Remove account?");
    await tick();
    expect(document.querySelector(".cm-dialog-dontask")).toBeNull();
    act(() => button("No").click());
    expect(await p).toBe(false);
  });
});

const press = (el: Element, key: string, opts: KeyboardEventInit = {}) =>
  act(() => void el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...opts })));

describe("answering with keys", () => {
  it("gives each button its own letter, underlined", async () => {
    expect(accessKeys(["Yes", "No", "Cancel"])).toEqual(["y", "n", "c"]);
    expect(accessKeys(["Send Anyway", "Save Drafts", "Don't Send"])).toEqual(["s", "a", "d"]);
    const p = messageBox({ title: "t", text: "Save?", buttons: ["Yes", "No", "Cancel"], defaultButton: "Yes" });
    await tick();
    expect(button("No").innerHTML).toBe("<u>N</u>o");
    press(document.activeElement!, "c");
    expect(await p).toBe("Cancel");
  });

  it("Y and N answer a Yes/No box, Enter takes the default", async () => {
    let p = confirmBox("Delete?");
    await tick();
    press(document.activeElement!, "N");
    expect(await p).toBe(false);
    p = confirmBox("Delete?");
    await tick();
    press(document.activeElement!, "y");
    expect(await p).toBe(true);
    p = confirmBox("Delete?");
    await tick();
    press(document.querySelector(".cm-dialog")!, "Enter");
    expect(await p).toBe(true);
  });

  it("arrow keys move between the buttons", async () => {
    const p = confirmBox("Delete?");
    await tick();
    expect(document.activeElement).toBe(button("Yes"));
    press(document.activeElement!, "ArrowRight");
    expect(document.activeElement).toBe(button("No"));
    press(document.activeElement!, "ArrowRight");
    expect(document.activeElement).toBe(button("Yes"));
    press(document.activeElement!, "ArrowLeft");
    act(() => (document.activeElement as HTMLButtonElement).click());
    expect(await p).toBe(false);
  });
});

describe("dialog keys", () => {
  function Form({ onOk, onCancel, own }: { onOk: () => void; onCancel: () => void; own?: () => void }) {
    return (
      <Dialog
        title="Name"
        onClose={onCancel}
        footer={
          <>
            <button className="cm-button default" onClick={onOk}>
              OK
            </button>
            <button className="cm-button" onClick={onCancel}>
              Cancel
            </button>
          </>
        }
      >
        <input
          className="cm-input"
          onKeyDown={(e) => {
            if (own && e.key === "Enter") {
              e.preventDefault();
              own();
            }
          }}
        />
        <textarea />
      </Dialog>
    );
  }

  it("Enter in a field presses OK; letters type, Alt+letter presses a button", async () => {
    const ok = vi.fn();
    const cancel = vi.fn();
    act(() => root.render(<Form onOk={ok} onCancel={cancel} />));
    await tick();
    const input = document.querySelector("input")!;
    expect(document.activeElement).toBe(input);
    press(input, "c");
    expect(cancel).not.toHaveBeenCalled();
    press(input, "c", { altKey: true });
    expect(cancel).toHaveBeenCalledTimes(1);
    press(input, "Enter");
    expect(ok).toHaveBeenCalledTimes(1);
    // a new line in a text box isn't OK
    press(document.querySelector("textarea")!, "Enter");
    expect(ok).toHaveBeenCalledTimes(1);
  });

  it("a field that handles Enter itself isn't pressed twice", async () => {
    const ok = vi.fn();
    const own = vi.fn();
    act(() => root.render(<Form onOk={ok} onCancel={() => {}} own={own} />));
    await tick();
    press(document.querySelector("input")!, "Enter");
    expect(own).toHaveBeenCalledTimes(1);
    expect(ok).not.toHaveBeenCalled();
  });
});
