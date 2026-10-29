import { useCallback, useEffect, useRef, useState } from "react";
import { keyOf } from "./keys";

/** useState persisted in localStorage (per window origin). Falls back silently if storage fails. */
export function usePersistentState<T>(key: string, initial: T): [T, (v: T | ((p: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(`cm:${key}`);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (v: T | ((p: T) => T)) => {
      setValue((prev) => {
        const next = typeof v === "function" ? (v as (p: T) => T)(prev) : v;
        try {
          localStorage.setItem(`cm:${key}`, JSON.stringify(next));
        } catch {
          /* ignore */
        }
        return next;
      });
    },
    [key],
  );
  return [value, set];
}

// keys that edit text: in a text field they always do that
const TEXT_KEYS = new Set(
  ["a", "c", "v", "x", "z", "y", "shift+z", "shift+v", "backspace", "delete", "arrowleft", "arrowright", "shift+arrowleft", "shift+arrowright", "home", "end"].map(
    (k) => `ctrl+${k}`,
  ),
);

/**
 * Keyboard shortcuts: { "ctrl+r": fn, "delete": fn, "f9": fn, "ctrl+shift+v": fn }.
 * Ignored while typing in inputs unless the combo includes ctrl/alt or is an F-key (or
 * `allowInInputs`), and never takes a text field's editing keys (Ctrl+C, Ctrl+Shift+V…).
 * A key something else already handled (an open menu or popup, a list) is left alone.
 */
export function useHotkeys(map: Record<string, (e: KeyboardEvent) => void>, opts: { allowInInputs?: boolean; enabled?: boolean } = {}) {
  const ref = useRef(map);
  ref.current = map;
  useEffect(() => {
    if (opts.enabled === false) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      if (document.querySelector(".cm-dialog-overlay")) return; // a modal dialog owns the keyboard
      if (document.querySelector(".cm-menu")) return; // so does an open menu
      const parts: string[] = [];
      if (e.ctrlKey) parts.push("ctrl");
      if (e.altKey) parts.push("alt");
      if (e.shiftKey) parts.push("shift");
      const k = keyOf(e);
      parts.push(k === " " ? "space" : k);
      const combo = parts.join("+");
      const fn = ref.current[combo];
      if (!fn) return;
      const t = e.target as HTMLElement;
      const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
      if (typing && !opts.allowInInputs && !(e.ctrlKey || e.altKey || /^f\d+$/.test(k))) return;
      // AltGr arrives as Ctrl+Alt: in a text field it types a character (€, @ on some keyboards)
      if (typing && (TEXT_KEYS.has(combo) || e.getModifierState?.("AltGraph"))) return;
      // Enter and Space on a focused button press that button
      if ((combo === "enter" || combo === "space") && t?.closest?.("button, a, input[type=checkbox], input[type=radio]")) return;
      e.preventDefault();
      fn(e);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [opts.allowInInputs, opts.enabled]);
}

/** Debounced value. */
export function useDebounced<T>(value: T, ms = 250): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
