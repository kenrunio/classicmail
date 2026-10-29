// Outlook-style selection over the visible contacts: click, Ctrl+click, Shift+click, keyboard,
// type-ahead ("type the first letters of a File As name to jump there").
import { useRef } from "react";
import type { Contact } from "../../api/contacts";
import { keyOf, showMenu } from "../../ui";
import { contextMenuFor, deleteContacts, openContact } from "./actions";
import { selectedContacts, useContactsUi } from "./store";
import { compareText } from "./sort";

/** `opts.textOf`: what type-ahead matches (the active sort's text; File As by default). */
export function useSelection(items: Contact[], scrollTo: (index: number) => void, opts: { textOf?: (c: Contact) => string; desc?: boolean } = {}) {
  const selected = useContactsUi((s) => s.selected);
  const anchor = useContactsUi((s) => s.anchor);
  const select = useContactsUi((s) => s.select);
  const typed = useRef({ text: "", at: 0 });
  const selSet = new Set(selected);
  const focusIdx = (() => {
    const last = selected[selected.length - 1];
    const i = items.findIndex((c) => c.id === last);
    return i;
  })();

  const rangeIds = (a: number, b: number) => items.slice(Math.min(a, b), Math.max(a, b) + 1).map((c) => c.id);

  function onItemMouseDown(index: number, e: React.MouseEvent) {
    const c = items[index];
    if (!c) return;
    if (e.button === 2) {
      if (!selSet.has(c.id)) select([c.id], index);
      return;
    }
    if (e.shiftKey) {
      const a = anchor ?? index;
      const ids = rangeIds(a, index);
      // keep the clicked one last = focused
      select([...ids.filter((id) => id !== c.id), c.id]);
    } else if (e.ctrlKey) {
      if (selSet.has(c.id))
        select(
          selected.filter((id) => id !== c.id),
          index,
        );
      else select([...selected, c.id], index);
    } else select([c.id], index);
  }

  function onItemContextMenu(index: number, e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const c = items[index];
    if (c && !useContactsUi.getState().selected.includes(c.id)) select([c.id], index);
    showMenu(e.clientX, e.clientY, contextMenuFor());
  }

  function moveTo(index: number, extend: boolean) {
    if (!items.length) return;
    const i = Math.max(0, Math.min(items.length - 1, index));
    if (extend) {
      const a = anchor ?? (focusIdx >= 0 ? focusIdx : i);
      const ids = rangeIds(a, i).filter((id) => id !== items[i].id);
      select([...ids, items[i].id], a);
    } else select([items[i].id], i);
    scrollTo(i);
  }

  /** Jump to the first contact whose sort text starts with (or sorts after) `text`. */
  function jumpTo(text: string) {
    const t = text.toLowerCase();
    const textOf = opts.textOf ?? ((c: Contact) => c.fileAs);
    let i = items.findIndex((c) => textOf(c).toLowerCase().startsWith(t));
    if (i < 0) i = items.findIndex((c) => (opts.desc ? -1 : 1) * compareText(textOf(c).toLowerCase(), t) >= 0);
    if (i < 0) i = items.length - 1;
    moveTo(i, false);
  }

  /** Keyboard handling for the list container. `perColumn` = items per column for ←/→ in card views. */
  function onKeyDown(e: React.KeyboardEvent, opts: { horizontalStep?: number; pageStep?: number } = {}) {
    const cur = focusIdx >= 0 ? focusIdx : -1;
    const k = e.key;
    const step = (n: number) => {
      e.preventDefault();
      moveTo(cur < 0 ? 0 : cur + n, e.shiftKey);
    };
    if (k === "ArrowDown") step(1);
    else if (k === "ArrowUp") step(-1);
    else if (k === "ArrowRight" && opts.horizontalStep) step(opts.horizontalStep);
    else if (k === "ArrowLeft" && opts.horizontalStep) step(-opts.horizontalStep);
    else if (k === "PageDown") step(opts.pageStep ?? 10);
    else if (k === "PageUp") step(-(opts.pageStep ?? 10));
    else if (k === "Home") {
      e.preventDefault();
      moveTo(0, e.shiftKey);
    } else if (k === "End") {
      e.preventDefault();
      moveTo(items.length - 1, e.shiftKey);
    } else if (k === "Enter") {
      e.preventDefault();
      selectedContacts().slice(0, 10).forEach(openContact);
    } else if (k === "Delete") {
      e.preventDefault();
      deleteContacts(selectedContacts());
    } else if (e.ctrlKey && keyOf(e) === "a") {
      e.preventDefault();
      select(
        items.map((c) => c.id),
        0,
      );
    } else if (k.length === 1 && !e.ctrlKey && !e.altKey && k.trim()) {
      const now = Date.now();
      typed.current = { text: now - typed.current.at < 1000 ? typed.current.text + k : k, at: now };
      jumpTo(typed.current.text);
    }
  }

  return { selSet, focusIdx, onItemMouseDown, onItemContextMenu, onKeyDown, moveTo, jumpTo };
}
