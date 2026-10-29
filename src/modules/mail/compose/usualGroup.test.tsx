import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";

const calls: { method: string; params: unknown }[] = [];
vi.mock("../../../api/rpc", () => ({
  rpc: async (method: string, params: { query?: string; emails?: string[] }) => {
    calls.push({ method, params });
    if (method === "contacts.autocomplete") return [];
    if (method === "mail.recipients.suggest") return params.query?.toLowerCase().startsWith("yo") ? [{ name: "Yoav Shalev", email: "yoav@orbit.example", source: "recent" }] : [];
    if (method === "mail.recipients.usualGroup")
      return params.emails?.length === 1 && params.emails[0] === "yoav@orbit.example"
        ? { to: [{ name: "Dana Levi", email: "dana@orbit.example" }], cc: [{ name: "Ron Adler", email: "ron@duvz.demo" }] }
        : { to: [], cc: [] };
    return null;
  },
  RpcError: class extends Error {},
}));

import { RecipientField, type Recipient, type RecipientFieldHandle } from "./RecipientField";
import { useUsualGroup } from "./usualGroup";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Harness() {
  const [to, setTo] = useState<Recipient[]>([]);
  const [cc, setCc] = useState<Recipient[]>([]);
  const ccRef = useRef<RecipientFieldHandle>(null);
  const group = useUsualGroup({ to, setTo, cc, setCc, next: () => ccRef.current?.focus() });
  return (
    <div>
      <div data-box="to">
        <RecipientField label="To" value={to} onChange={setTo} group={group} />
      </div>
      <div data-box="cc">
        <RecipientField ref={ccRef} label="Cc" value={cc} onChange={setCc} />
      </div>
    </div>
  );
}

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  calls.length = 0;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(<Harness />));
});
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

const input = (box: string) => host.querySelector<HTMLInputElement>(`[data-box="${box}"] input`)!;
const names = (box: string) =>
  [...host.querySelectorAll(`[data-box="${box}"] .cm-compose-rcpt-name`)].map((e) => `${e.textContent}${e.classList.contains("suggested") ? "*" : ""}`);
const settle = () => act(() => new Promise((r) => setTimeout(r, 30)));

async function type(el: HTMLInputElement, text: string) {
  el.focus();
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    set.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await settle();
}
async function key(el: HTMLInputElement, k: string) {
  await act(async () => {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
  });
  await settle();
}

describe("usual groups in the To box", () => {
  it("Tab after a name brings the usual people (To and Cc, highlighted); Tab again keeps them and goes to Cc", async () => {
    await type(input("to"), "Yoav");
    await key(input("to"), "Tab");
    expect(names("to")).toEqual(["Yoav Shalev", "Dana Levi*"]);
    expect(names("cc")).toEqual(["Ron Adler*"]);
    expect(document.activeElement).toBe(input("to"));
    await key(input("to"), "Tab");
    expect(names("to")).toEqual(["Yoav Shalev", "Dana Levi"]);
    expect(names("cc")).toEqual(["Ron Adler"]);
    expect(document.activeElement).toBe(input("cc"));
  });

  it("Delete drops them; the next name asks again", async () => {
    await type(input("to"), "Yoav");
    await key(input("to"), "Tab");
    await key(input("to"), "Delete");
    expect(names("to")).toEqual(["Yoav Shalev"]);
    expect(names("cc")).toEqual([]);
    expect(document.activeElement).toBe(input("to"));
    await type(input("to"), "someone@other.example");
    await key(input("to"), "Tab");
    const asked = calls.filter((c) => c.method === "mail.recipients.usualGroup").map((c) => (c.params as { emails: string[] }).emails);
    expect(asked).toEqual([["yoav@orbit.example"], ["yoav@orbit.example", "someone@other.example"]]);
    // no group for those two: Tab simply moved on
    expect(names("to")).toEqual(["Yoav Shalev", "someone@other.example"]);
  });

  it("typing a letter drops them and starts the next name", async () => {
    await type(input("to"), "Yoav");
    await key(input("to"), "Tab");
    await key(input("to"), "m");
    expect(names("to")).toEqual(["Yoav Shalev"]);
    expect(names("cc")).toEqual([]);
  });
});
