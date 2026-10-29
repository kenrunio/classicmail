// Outlook 2007 recipient box: resolved names are underlined, separated by "; ", with the
// auto-complete drop-down while typing (Enter/Tab picks, Del removes an entry from the list).
// Names select like Outlook's: click, Ctrl/Shift+click, Shift+arrows or Ctrl+A; then
// Ctrl+X / C / V (or the ribbon / right-click) cut, copy and paste them as "Name <email>; …".
// Drag names to another box to move them there (Ctrl+drag copies), as in Outlook.
// With `group` (the To box): Tab after a name brings in the people usually with them (usualGroup.ts).
import { forwardRef, useEffect, useId, useImperativeHandle, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { alertBox, contextMenu, keyOf } from "../../../ui";
import { type Addr, type Suggestion, composeApi, formatAddr, isValidEmail, parseAddresses, suggestRecipients } from "../../../api/mailCompose";
import { NAMES_MIME, addrsClipText, clickSelect, copyText, span } from "../addrClip";
import type { GroupHooks } from "./usualGroup";

/** The names being dragged out of a box in this window (a drop elsewhere decides move or copy). */
let dragging: { from: string; idx: number[] } | null = null;

export interface Recipient extends Addr {
  /** resolved = checked (underlined); unresolved names are only text until Check Names */
  resolved: boolean;
  /** brought in by a usual group, not yet kept (highlighted; Tab keeps it, anything else drops it) */
  suggested?: boolean;
}

export interface RecipientFieldHandle {
  /** commit whatever is being typed; returns the full list */
  commit: () => Recipient[];
  focus: () => void;
}

export function toRecipients(list: Addr[]): Recipient[] {
  return list.map((a) => ({ ...a, resolved: isValidEmail(a.email) }));
}

function textToRecipients(text: string): Recipient[] {
  return parseAddresses(text).map((a) => ({ ...a, resolved: isValidEmail(a.email) }));
}

// Recent auto-complete answers, so the drop-down is there at once when the same letters are typed
// again (in this window; a minute is plenty for a message being addressed).
const sugCache = new Map<string, { at: number; list: Suggestion[] }>();
async function lookup(q: string): Promise<Suggestion[]> {
  const key = q.toLowerCase();
  const hit = sugCache.get(key);
  if (hit && Date.now() - hit.at < 60_000) return hit.list;
  const list = await suggestRecipients(q, 8);
  sugCache.set(key, { at: Date.now(), list });
  if (sugCache.size > 200) sugCache.delete(sugCache.keys().next().value!);
  return list;
}

/** What Tab moves to after `el` (the next focusable control in the window). */
function focusNext(el: HTMLElement) {
  const all = [...document.querySelectorAll<HTMLElement>("input, textarea, select, button, [contenteditable='true'], [tabindex]")].filter(
    (x) => x.tabIndex >= 0 && !x.hasAttribute("disabled") && x.offsetParent !== null,
  );
  all[all.indexOf(el) + 1]?.focus();
}

export const RecipientField = forwardRef<
  RecipientFieldHandle,
  {
    value: Recipient[];
    onChange: (v: Recipient[]) => void;
    label: string;
    onDirty?: () => void;
    /** the To box: usual groups (usualGroup.ts) */
    group?: GroupHooks;
  }
>(function RecipientField({ value, onChange, label, onDirty, group }, ref) {
  const [text, setText] = useState("");
  const [sugs, setSugs] = useState<Suggestion[]>([]);
  const [hi, setHi] = useState(0);
  // the selected names (indexes into value); anchor / lead are the fixed and the moving end
  const [sel, setSelState] = useState<number[]>([]);
  const anchor = useRef<number | null>(null);
  const lead = useRef<number | null>(null);
  const fieldId = useId();
  const [dropOver, setDropOver] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const seq = useRef(0);
  // the typed text the drop-down's names were found for
  const sugsFor = useRef("");
  const valueRef = useRef(value);
  valueRef.current = value;
  const textRef = useRef(text);
  textRef.current = text;
  const selRef = useRef(sel);
  selRef.current = sel;
  const groupRef = useRef(group);
  groupRef.current = group;

  /** A name was added with Tab: the usual group comes in (the cursor stays), else Tab goes on. */
  const tabbed = (list: Recipient[], el: HTMLElement, moveOn: boolean) => {
    if (!groupRef.current) {
      if (moveOn) focusNext(el);
      return;
    }
    void groupRef.current.afterTab(list).then((found) => {
      if (!found && moveOn) focusNext(el);
    });
  };

  const setSel = (next: number[], a: number | null = next[0] ?? null, l: number | null = next[next.length - 1] ?? null) => {
    setSelState(next);
    selRef.current = next;
    anchor.current = a;
    lead.current = l;
  };
  const clearSel = () => setSel([]);
  const selectAll = () => setSel(span(0, valueRef.current.length - 1), 0, valueRef.current.length - 1);

  const commitText = (): Recipient[] => {
    const t = textRef.current.trim();
    if (!t) return valueRef.current;
    const next = [...valueRef.current, ...textToRecipients(t)];
    setText("");
    setSugs([]);
    onChange(next);
    return next;
  };

  useImperativeHandle(ref, () => ({
    commit: commitText,
    focus: () => inputRef.current?.focus(),
  }));

  // auto-complete (Outlook shows the drop-down after the first character), asked at once
  useEffect(() => {
    const q = text.trim();
    if (!q || q.includes(";") || q.includes(",")) {
      // a lookup still on its way (the name was just added) mustn't bring the drop-down back
      seq.current++;
      setSugs([]);
      sugsFor.current = "";
      return;
    }
    const my = ++seq.current;
    void lookup(q).then((s) => {
      if (my === seq.current) {
        setSugs(s);
        sugsFor.current = q;
        setHi(0);
      }
    });
  }, [text]);

  /** Tab / Enter before the drop-down caught up with the typing: the first name it would have
   *  shown (Outlook's choice), else the text as typed — never a lone "e". */
  const pickTyped = async (el: HTMLElement, moveOn: boolean) => {
    const q = textRef.current.trim();
    const first = (await lookup(q))[0];
    if (textRef.current.trim() !== q) return; // typed on meanwhile
    let list: Recipient[];
    if (first) {
      list = [...valueRef.current, { name: first.name, email: first.email, resolved: true }];
      onChange(list);
      setText("");
      setSugs([]);
    } else list = commitText();
    onDirty?.();
    if (moveOn) tabbed(list, el, true);
  };

  const pick = (s: Suggestion, byTab = false) => {
    const list = [...valueRef.current, { name: s.name, email: s.email, resolved: true }];
    onChange(list);
    setText("");
    setSugs([]);
    onDirty?.();
    inputRef.current?.focus();
    if (byTab && inputRef.current) tabbed(list, inputRef.current, false);
  };

  /** Removes these names. */
  const removeAt = (idx: number[]) => {
    onChange(valueRef.current.filter((_, j) => !idx.includes(j)));
    clearSel();
    onDirty?.();
    inputRef.current?.focus();
  };

  const editEntry = (i: number) => {
    const r = value[i];
    const next = value.filter((_, j) => j !== i);
    onChange(next);
    setText(formatAddr(r));
    clearSel();
    setTimeout(() => inputRef.current?.focus(), 0);
  };

  /** The selected part of the typed text ("" when none). */
  const typedSelection = (): string => {
    const el = inputRef.current;
    if (!el || el.selectionStart === null || el.selectionEnd === null) return "";
    return el.value.slice(el.selectionStart, el.selectionEnd);
  };
  /** What Copy puts on the clipboard: the selected names, then any selected typed text. */
  const clipText = (idx: number[]): string => [addrsClipText(idx.map((i) => valueRef.current[i]).filter(Boolean)), typedSelection().trim()].filter(Boolean).join("; ");
  /** Deletes the selected names and the selected typed text. */
  const deleteSelection = (idx: number[]) => {
    const el = inputRef.current;
    if (el && el.selectionStart !== null && el.selectionEnd !== null && el.selectionStart !== el.selectionEnd) {
      const t = el.value.slice(0, el.selectionStart) + el.value.slice(el.selectionEnd);
      setText(t);
      textRef.current = t;
    }
    removeAt(idx);
  };
  const copyNames = (idx: number[]) => copyText(clipText(idx));
  const cutNames = (idx: number[]) => {
    copyNames(idx);
    deleteSelection(idx);
  };

  /** Pasted text in place of the selection. Names with separators (or one whole address) become
   *  names at once; anything else is left for the browser to type in (auto-complete, Check Names).
   *  Returns true when it was taken as names. */
  const insertPasted = (t: string): boolean => {
    const el = inputRef.current;
    const cur = textRef.current;
    const ss = el?.selectionStart ?? cur.length;
    const se = el?.selectionEnd ?? cur.length;
    const parsed = textToRecipients((cur.slice(0, ss) + t + cur.slice(se)).replace(/\r?\n/g, ";"));
    if (!/[;,\n]/.test(t) && !(parsed.length === 1 && parsed[0].resolved)) {
      if (selRef.current.length) removeAt(selRef.current);
      return false;
    }
    onChange([...valueRef.current.filter((_, j) => !selRef.current.includes(j)), ...parsed]);
    clearSel();
    setText("");
    onDirty?.();
    return true;
  };

  // Cut / Copy from the ribbon, the Edit menu or the WebView's own menu (they fire copy / cut)
  useEffect(() => {
    const onClip = (e: ClipboardEvent) => {
      if (document.activeElement !== inputRef.current || !selRef.current.length) return;
      e.clipboardData?.setData("text/plain", clipText(selRef.current));
      e.preventDefault();
      if (e.type === "cut") deleteSelection(selRef.current);
    };
    document.addEventListener("copy", onClip);
    document.addEventListener("cut", onClip);
    return () => {
      document.removeEventListener("copy", onClip);
      document.removeEventListener("cut", onClip);
    };
  });

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Control" || e.key === "Shift" || e.key === "Alt" || e.key === "Meta") return;
    // a usual group showing: Tab keeps it (and goes to Cc), anything else drops it
    if (group?.pending) {
      if (e.key === "Tab" && !e.shiftKey) {
        e.preventDefault();
        group.accept();
        return;
      }
      group.reject();
      if (e.key === "Delete" || e.key === "Backspace" || e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        return;
      }
    }
    const k = keyOf(e);
    if (e.ctrlKey && !e.altKey && k === "a") {
      // Outlook: every name in the box, and what's typed
      e.preventDefault();
      e.stopPropagation();
      setSugs([]);
      if (value.length) selectAll();
      inputRef.current?.select();
      return;
    }
    if (sel.length > 0) {
      const copyKey = e.ctrlKey && !e.altKey && (k === "c" || k === "insert");
      const cutKey = (e.ctrlKey && !e.altKey && k === "x") || (e.shiftKey && e.key === "Delete");
      if (copyKey || cutKey) {
        e.preventDefault();
        e.stopPropagation();
        if (copyKey) copyNames(sel);
        else cutNames(sel);
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        deleteSelection(sel);
        return;
      }
      // typing replaces the selected names
      if (e.key.length === 1 && !e.ctrlKey && !e.altKey) removeAt(sel);
    }
    const typed = text.trim();
    if ((e.key === "Enter" || (e.key === "Tab" && !e.shiftKey)) && typed && !/[;,@]/.test(typed) && (sugs.length === 0 || sugsFor.current !== typed)) {
      e.preventDefault();
      void pickTyped(e.currentTarget, e.key === "Tab");
      return;
    }
    if (sugs.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setHi((h) => Math.min(sugs.length - 1, h + 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setHi((h) => Math.max(0, h - 1));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        if (e.key === "Enter") e.preventDefault();
        if (e.key === "Tab" && e.shiftKey) return;
        e.preventDefault();
        pick(sugs[hi], e.key === "Tab");
        return;
      }
      if (e.key === "Delete" && sugs[hi]) {
        // Outlook: Del removes the highlighted name from the auto-complete list
        e.preventDefault();
        const gone = sugs[hi];
        composeApi.forgetRecipient(gone.email).catch(() => {});
        setSugs((s) => s.filter((x) => x !== gone));
        return;
      }
      if (e.key === "Escape") {
        e.stopPropagation();
        e.preventDefault();
        setSugs([]);
        return;
      }
    }
    // a whole address typed, then Tab: it's added and its usual group asked for
    if (group && e.key === "Tab" && !e.shiftKey && typed && !/[;,]/.test(typed) && isValidEmail(typed.replace(/^.*<|>$/g, ""))) {
      e.preventDefault();
      tabbed(commitText(), e.currentTarget, true);
      onDirty?.();
      return;
    }
    if (e.key === ";" || e.key === ",") {
      if (!text.includes('"') || (text.match(/"/g)?.length ?? 0) % 2 === 0) {
        e.preventDefault();
        commitText();
        onDirty?.();
      }
      return;
    }
    const last = value.length - 1;
    if (e.key === "Backspace" && text === "" && value.length > 0) {
      // the first Backspace selects the last name, the second removes it
      e.preventDefault();
      setSel([last]);
      return;
    }
    if (e.key === "ArrowLeft" && text === "" && value.length > 0) {
      e.preventDefault();
      const to = !sel.length || lead.current === null ? last : Math.max(0, lead.current - 1);
      if (e.shiftKey && sel.length && anchor.current !== null) setSel(span(anchor.current, to), anchor.current, to);
      else setSel([to]);
      return;
    }
    if (e.key === "ArrowRight" && sel.length > 0) {
      e.preventDefault();
      const from = lead.current ?? last;
      if (e.shiftKey && anchor.current !== null) {
        const to = Math.min(last, from + 1);
        setSel(span(anchor.current, to), anchor.current, to);
      } else if (from >= last) clearSel();
      else setSel([from + 1]);
      return;
    }
    if (sel.length) clearSel();
  };

  const onPaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
    if (insertPasted(e.clipboardData.getData("text/plain"))) e.preventDefault();
  };
  const pasteFromClipboard = async (idx: number[]) => {
    try {
      const t = await navigator.clipboard.readText();
      inputRef.current?.focus();
      setSel(idx);
      if (!insertPasted(t)) document.execCommand("insertText", false, t);
    } catch {
      alertBox("Use Ctrl+V to paste here.", "Paste");
    }
  };

  const rect = boxRef.current?.getBoundingClientRect();
  return (
    <div
      ref={boxRef}
      className={`cm-compose-rcpt${dropOver ? " drop-over" : ""}`}
      // names dropped here: from another box (moved, or copied with Ctrl), a message being read, or text
      onDragOver={(e) => {
        if (dragging?.from === fieldId) return;
        const types = e.dataTransfer.types;
        if (!types.includes(NAMES_MIME) && !types.includes("text/plain")) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = dragging && !e.ctrlKey ? "move" : "copy";
        setDropOver(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropOver(false);
      }}
      onDrop={(e) => {
        setDropOver(false);
        if (dragging?.from === fieldId) return;
        e.preventDefault();
        const raw = e.dataTransfer.getData(NAMES_MIME);
        const items: Recipient[] = raw ? toRecipients(JSON.parse(raw) as Addr[]) : textToRecipients(e.dataTransfer.getData("text/plain").replace(/\r?\n/g, ";"));
        const have = new Set(valueRef.current.map((r) => r.email.toLowerCase()));
        const add = items.filter((r) => r.email && !have.has(r.email.toLowerCase()));
        if (add.length) onChange([...valueRef.current, ...add]);
        onDirty?.();
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          e.preventDefault();
          clearSel();
          inputRef.current?.focus();
        }
      }}
      aria-label={label}
    >
      {value.map((r, i) => (
        <span key={i} className="cm-compose-rcpt-item">
          <span
            className={`cm-compose-rcpt-name${r.resolved ? " resolved" : ""}${sel.includes(i) ? " selected" : ""}${r.suggested ? " suggested" : ""}`}
            title={r.suggested ? `${r.email} — usually with these people: Tab keeps, Delete removes` : r.email}
            // no preventDefault here: it would stop the name from being dragged; the typing area gets
            // the focus back right after
            onMouseDown={(e) => {
              setTimeout(() => inputRef.current?.focus(), 0);
              if (e.button === 2 && sel.includes(i)) return; // right-click keeps a multi-selection
              if (e.button === 0 && sel.includes(i) && !e.ctrlKey && !e.shiftKey) return; // may be a drag of the selection
              const c = clickSelect(sel, anchor.current, i, { ctrl: e.ctrlKey, shift: e.shiftKey });
              setSel(c.sel, c.anchor, i);
            }}
            onClick={(e) => {
              // a click (not a drag) on one of several selected names selects just it
              if (!e.ctrlKey && !e.shiftKey && sel.length > 1 && sel.includes(i)) setSel([i]);
            }}
            draggable
            onDragStart={(e) => {
              const idx = sel.includes(i) ? sel : [i];
              if (!sel.includes(i)) setSel([i]);
              const items = idx.map((j) => valueRef.current[j]).filter(Boolean);
              dragging = { from: fieldId, idx };
              e.dataTransfer.setData(NAMES_MIME, JSON.stringify(items.map(({ name, email }) => ({ name, email }))));
              e.dataTransfer.setData("text/plain", addrsClipText(items));
              e.dataTransfer.effectAllowed = "copyMove";
            }}
            onDragEnd={(e) => {
              const d = dragging;
              dragging = null;
              if (d && e.dataTransfer.dropEffect === "move") removeAt(d.idx);
            }}
            onDoubleClick={() => editEntry(i)}
            onContextMenu={contextMenu(() => {
              const idx = sel.includes(i) ? sel : [i];
              return [
                { label: r.email, type: "header" as const },
                { label: "Cut", icon: "cut", shortcut: "Ctrl+X", onClick: () => cutNames(idx) },
                { label: "Copy", icon: "copy", shortcut: "Ctrl+C", onClick: () => copyNames(idx) },
                { label: "Paste", icon: "paste", shortcut: "Ctrl+V", onClick: () => void pasteFromClipboard(idx) },
                { type: "separator" as const },
                { label: "Edit", disabled: idx.length > 1, onClick: () => editEntry(i) },
                { label: "Remove", icon: "delete", shortcut: "Del", onClick: () => removeAt(idx) },
                { label: "Select All", shortcut: "Ctrl+A", onClick: selectAll },
              ];
            })}
          >
            {r.name || r.email}
          </span>
          <span className="cm-compose-rcpt-sep">; </span>
        </span>
      ))}
      <input
        ref={inputRef}
        className="cm-compose-rcpt-input"
        value={text}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => {
          setText(e.target.value);
          onDirty?.();
        }}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onBlur={() => {
          setTimeout(() => {
            if (document.activeElement !== inputRef.current) {
              setSugs([]);
              clearSel();
              if (textRef.current.trim()) commitText();
              // left the box without Tab: a usual group showing isn't kept
              if (groupRef.current?.pending) groupRef.current.reject();
            }
          }, 150);
        }}
      />
      {sugs.length > 0 &&
        rect &&
        createPortal(
          <div className="cm-compose-ac" style={{ left: rect.left + 2, top: rect.bottom, minWidth: Math.max(280, rect.width * 0.6) }}>
            {sugs.map((s, i) => (
              <div
                key={s.email}
                className={`cm-compose-ac-row${i === hi ? " hi" : ""}`}
                onMouseEnter={() => setHi(i)}
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(s);
                }}
              >
                {s.name ? (
                  <>
                    {s.name} &lt;{s.email}&gt;
                  </>
                ) : (
                  s.email
                )}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
});
