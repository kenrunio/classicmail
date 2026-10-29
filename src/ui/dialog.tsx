// In-window modal dialogs and promise-based message boxes (Outlook-style "Are you sure?" boxes).
import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { create } from "zustand";
import { Icon } from "./icons";
import { KeyLabel, accessKeys, keyOf } from "./keys";
import { rpc } from "../api/rpc";

export function Dialog({
  title,
  children,
  footer,
  onClose,
  width = 420,
  height,
}: {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
  width?: number;
  height?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // the element focused before the dialog opened (captured at first render: StrictMode re-runs effects)
  const [prev] = useState(() => document.activeElement as HTMLElement | null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // only the topmost dialog closes
        const all = document.querySelectorAll(".cm-dialog");
        if (all[all.length - 1] !== ref.current) return;
        e.stopPropagation();
        closeRef.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  // focus the first field once (not on every re-render), and give focus back on close
  useEffect(() => {
    const body = ref.current?.querySelector(".cm-dialog-body, .cm-dialog-footer");
    const el =
      ref.current?.querySelector<HTMLElement>(".cm-dialog-body [data-autofocus]") ??
      ref.current?.querySelector<HTMLElement>(".cm-dialog-body :is(input, select, textarea):not([disabled])") ??
      ref.current?.querySelector<HTMLElement>(".cm-dialog-footer button.default") ??
      body?.querySelector<HTMLElement>("button");
    if (!ref.current?.contains(document.activeElement)) el?.focus();
    return () => {
      if (prev && prev !== document.body && prev.isConnected) setTimeout(() => prev.isConnected && !document.querySelector(".cm-dialog") && prev.focus(), 0);
    };
  }, [prev]);
  // Windows dialog keys: Enter = the default button, arrows move between the buttons, Alt+letter
  // presses the button with that access key (a message box takes the plain letter too, see
  // data-key). A field's own Enter handler calls preventDefault to keep it.
  const onKeyDown = (e: ReactKeyboardEvent) => {
    const box = ref.current;
    const t = e.target as HTMLElement;
    // React bubbles through portals: keys from a dialog opened on top of this one aren't ours
    if (e.defaultPrevented || e.nativeEvent.isComposing || !box?.contains(t)) return;
    const buttons = [...box.querySelectorAll<HTMLButtonElement>(".cm-dialog-footer button:not(:disabled)")];
    if (e.key === "Enter") {
      if (e.altKey || t.isContentEditable || ["TEXTAREA", "BUTTON", "A"].includes(t.tagName)) return;
      const def = buttons.find((b) => b.classList.contains("default"));
      if (!def) return;
      e.preventDefault();
      def.click();
      return;
    }
    const i = buttons.indexOf(t as HTMLButtonElement);
    if (i >= 0 && /^Arrow(Left|Right|Up|Down)$/.test(e.key)) {
      e.preventDefault();
      const step = e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 1;
      buttons[(i + step + buttons.length) % buttons.length].focus();
      return;
    }
    if (e.key.length !== 1 || e.ctrlKey || e.metaKey) return;
    const typing =
      t.isContentEditable || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || (t.tagName === "INPUT" && !["checkbox", "radio"].includes((t as HTMLInputElement).type));
    const k = keyOf(e);
    const keys = accessKeys(buttons.map((b) => b.textContent ?? ""));
    const hit = e.altKey ? buttons[keys.indexOf(k)] : typing ? undefined : buttons.find((b) => b.dataset.key === k);
    if (!hit) return;
    e.preventDefault();
    if (!e.repeat) hit.click();
  };
  return createPortal(
    <div className="cm-dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && e.preventDefault()}>
      <div ref={ref} className="cm-dialog" style={{ width, height }} role="dialog" aria-label={title} onKeyDown={onKeyDown}>
        <div className="cm-dialog-title">
          <span className="cm-fill cm-ellipsis">{title}</span>
          <button className="cm-dialog-x" onClick={onClose} title="Close">
            <Icon name="close" size={12} />
          </button>
        </div>
        <div className="cm-dialog-body">{children}</div>
        {footer && <div className="cm-dialog-footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}

/**
 * "Don't ask me again" on a warning: ticked and answered with `answer` (the go-ahead button), the
 * box is skipped from then on and answers `answer` by itself. Remembered by `key` in the setting
 * ui.dontAsk, or — for a warning that already has its own on/off option — as that boolean
 * setting turned off. Tools › Options › Other › "Show all warnings again" undoes them all.
 */
export type DontAsk = { key: string; answer: string } | { setting: string; answer: string };

type Box = {
  title: string;
  text: ReactNode;
  icon?: "info" | "warning" | "error" | "help";
  buttons: string[];
  defaultButton?: string;
  dontAsk?: DontAsk;
  resolve: (b: string | null) => void;
};

async function dontAskList(): Promise<string[]> {
  const v = await rpc<unknown>("settings.get", { key: "ui.dontAsk" }).catch(() => null);
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
}

async function skipped(d: DontAsk): Promise<boolean> {
  if ("setting" in d) return (await rpc<unknown>("settings.get", { key: d.setting }).catch(() => null)) === false;
  return (await dontAskList()).includes(d.key);
}

async function remember(d: DontAsk) {
  if ("setting" in d) await rpc("settings.set", { key: d.setting, value: false });
  else {
    const list = await dontAskList();
    if (!list.includes(d.key)) await rpc("settings.set", { key: "ui.dontAsk", value: [...list, d.key] });
  }
}

const useBoxes = create<{ boxes: Box[]; push: (b: Box) => void; pop: (b: Box) => void }>((set) => ({
  boxes: [],
  push: (b) => set((s) => ({ boxes: [...s.boxes, b] })),
  pop: (b) => set((s) => ({ boxes: s.boxes.filter((x) => x !== b) })),
}));

/** Resolves with the clicked button's label (or null on Esc/close). */
export async function messageBox(opts: Omit<Box, "resolve">): Promise<string | null> {
  if (opts.dontAsk && (await skipped(opts.dontAsk))) return opts.dontAsk.answer;
  return new Promise((resolve) => useBoxes.getState().push({ ...opts, resolve }));
}

/** Yes / No. `dontAsk` (a stable key) offers "Don't ask me again" for Yes. */
export async function confirmBox(text: ReactNode, title = "ClassicMail", icon: Box["icon"] = "warning", dontAsk?: string): Promise<boolean> {
  return (await messageBox({ title, text, icon, buttons: ["Yes", "No"], defaultButton: "Yes", dontAsk: dontAsk ? { key: dontAsk, answer: "Yes" } : undefined })) === "Yes";
}

export async function alertBox(text: ReactNode, title = "ClassicMail", icon: Box["icon"] = "info"): Promise<void> {
  await messageBox({ title, text, icon, buttons: ["OK"], defaultButton: "OK" });
}

/** Show an error from a failed rpc call. */
export function errorBox(e: unknown, title = "ClassicMail") {
  // the user said no to the AI notice (src/api/aiConsent.tsx): nothing to report
  if ((e as { code?: string } | null)?.code === "ai_declined") return Promise.resolve();
  const msg = e instanceof Error ? e.message : String(e);
  return alertBox(msg, title, "error");
}

/** Set while the app is quitting and item windows are asking "save changes?" (see shell.rs). */
let quitPending = false;
export function setQuitPending(v: boolean) {
  quitPending = v;
}

export function MessageBoxHost() {
  const boxes = useBoxes((s) => s.boxes);
  const box = boxes[boxes.length - 1];
  if (!box) return null;
  return <MessageBoxView key={boxes.length} box={box} />;
}

function MessageBoxView({ box }: { box: Box }) {
  const [noMore, setNoMore] = useState(false);
  const keys = accessKeys(box.buttons);
  const done = (b: string | null) => {
    if (noMore && box.dontAsk && b === box.dontAsk.answer) void remember(box.dontAsk).catch(() => {});
    useBoxes.getState().pop(box);
    if (quitPending && (b === null || b === "Cancel")) {
      quitPending = false;
      import("@tauri-apps/api/event").then(({ emit }) => emit("cm-quit-cancelled")).catch(() => {});
    }
    box.resolve(b);
  };
  return (
    <Dialog
      title={box.title}
      onClose={() => done(null)}
      width={400}
      footer={
        <>
          {box.dontAsk && (
            <label className="cm-dialog-dontask" title={`Next time, go straight to "${box.dontAsk.answer}"`}>
              <input type="checkbox" checked={noMore} onChange={(e) => setNoMore(e.target.checked)} />
              Don't ask me again
            </label>
          )}
          {box.buttons.map((b, i) => (
            <button key={b} className={`cm-button${b === box.defaultButton ? " default" : ""}`} data-key={keys[i] ?? undefined} onClick={() => done(b)}>
              <KeyLabel label={b} k={keys[i]} />
            </button>
          ))}
        </>
      }
    >
      <div style={{ display: "flex", gap: 12, alignItems: "flex-start", padding: "4px 2px" }}>
        {box.icon && <Icon name={box.icon} size={32} />}
        <div style={{ whiteSpace: "pre-wrap", userSelect: "text" }}>{box.text}</div>
      </div>
    </Dialog>
  );
}
