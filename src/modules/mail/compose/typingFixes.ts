// Word-style fixes while typing in the compose body:
// * Caps Lock left on: "eLI" (a small letter, then capitals, Caps Lock on) → "Eli" and Caps Lock
//   is turned off — from the third letter, so the rest of the word comes out right;
// * Hebrew keyboard left on: a word typed as an English capital and then Hebrew letters ("Eךן",
//   Shift gives Latin capitals on the Hebrew layout) → "Eli" when the word ends, and the keyboard
//   switches to English;
// * paragraph direction follows the keyboard: an empty paragraph (in the reply, not the quoted
//   original) runs right-to-left while a Hebrew (or other right-to-left) keyboard is on and
//   left-to-right otherwise, so the cursor starts on the right side (Word's ¶ direction buttons,
//   automatically);
// * a web address typed and followed by a space or Enter becomes a link;
// * "- ", "* " or "1. " at the start of a line in the reply starts a bulleted / numbered list.
// The keyboard side (layout, Caps Lock, switching) is src-tauri/src/keyboard.rs.
import { rpc } from "../../../api/rpc";

/** Hebrew-layout characters → the English letter on the same key (SI-1452). */
const HE_TO_EN: Record<string, string> = {
  ק: "e", ר: "r", א: "t", ט: "y", ו: "u", ן: "i", ם: "o", פ: "p",
  ש: "a", ד: "s", ג: "d", כ: "f", ע: "g", י: "h", ח: "j", ל: "k", ך: "l", ף: ";",
  ז: "z", ס: "x", ב: "c", ה: "v", נ: "b", מ: "n", צ: "m", ת: ",", ץ: ".",
  "'": "w", "/": "q", ",": "'", ".": "/",
}; // prettier-ignore

/** "eLI" → "Eli": a small letter then two or more capitals is Caps Lock left on. */
export function capsFix(word: string): string | null {
  if (!/^\p{Ll}\p{Lu}{2,}$/u.test(word)) return null;
  return word[0].toUpperCase() + word.slice(1).toLowerCase();
}

/** "Eךן" → "Eli": an English capital and then Hebrew-layout letters is English typed with the
 *  Hebrew keyboard on (Shift still gives Latin capitals there). Needs two Hebrew letters. */
export function fromHebrewLayout(word: string): string | null {
  const m = /^([A-Z])([א-ת'/,.]+)$/.exec(word);
  if (!m || (m[2].match(/[א-ת]/g)?.length ?? 0) < 2) return null;
  return m[1] + [...m[2]].map((c) => HE_TO_EN[c] ?? c).join("");
}

/** The word right before the cursor: letters, and the punctuation keys Hebrew letters sit on. */
export function wordBefore(text: string): string {
  return /[\p{L}'/,.]*$/u.exec(text)?.[0].replace(/^[',./]+/, "") ?? "";
}

/** The web address ending `text` (http(s)://… or www.…), the link it makes, and how many
 *  characters of punctuation after it (`trail`, not part of the link), or null. A zero-width space
 *  (a font size picked at the caret) counts as a word break. */
export function urlBefore(text: string): { url: string; href: string; trail: number } | null {
  const m = /(?:^|[\s\u200b])((?:https?:\/\/|www\.)[^\s\u200b]+)$/i.exec(text);
  const url = m?.[1].replace(/[.,;:!?)\]'"]+$/, "");
  if (!m || !url || /^(https?:\/\/|www\.)$/i.test(url)) return null;
  return { url, href: /^www\./i.test(url) ? `http://${url}` : url, trail: m[1].length - url.length };
}

/** The list command for a line that so far holds only `text` and gets a space: "-" or "*" for
 *  bullets, "1." for numbering (zero-width spaces aside), else null. */
export function listMarker(text: string): "insertUnorderedList" | "insertOrderedList" | null {
  const t = text.replace(/\u200b/g, "");
  return t === "-" || t === "*" ? "insertUnorderedList" : t === "1." ? "insertOrderedList" : null;
}

const RTL_TEXT = /[֐-׿؀-ۿ]/;
const BLOCK = "p,h1,h2,h3,h4,h5,h6,li,blockquote,pre,td,th,div";

/** The block the caret is in, under `root` (never `root` itself). */
function blockAt(root: HTMLElement, node: Node | null): HTMLElement | null {
  const el = node instanceof Element ? node : (node?.parentElement ?? null);
  const b = el?.closest<HTMLElement>(BLOCK) ?? null;
  return b && b !== root && root.contains(b) ? b : null;
}

/** The text of `block` up to (`node`, `offset`). */
function textBefore(block: HTMLElement, node: Node, offset: number): string {
  const r = block.ownerDocument.createRange();
  r.setStart(block, 0);
  r.setEnd(node, offset);
  return r.toString();
}

/** The DOM range covering the `n` characters of text just before (`node`, `offset`) in `block`. */
function rangeBefore(block: HTMLElement, node: Node, offset: number, n: number): Range | null {
  const doc = block.ownerDocument;
  const upTo = doc.createRange();
  upTo.setStart(block, 0);
  upTo.setEnd(node, offset);
  const segs: { t: Text; len: number }[] = [];
  const walker = doc.createTreeWalker(block, NodeFilter.SHOW_TEXT);
  for (let t = walker.nextNode() as Text | null; t; t = walker.nextNode() as Text | null) {
    if (upTo.comparePoint(t, 0) > 0) break;
    const len = upTo.comparePoint(t, t.length) <= 0 ? t.length : t === node ? offset : 0;
    if (len) segs.push({ t, len });
  }
  let skip = segs.reduce((a, s) => a + s.len, 0) - n;
  if (skip < 0) return null;
  const r = doc.createRange();
  for (const s of segs) {
    if (skip >= s.len && s !== segs[segs.length - 1]) {
      skip -= s.len;
      continue;
    }
    r.setStart(s.t, Math.min(skip, s.len));
    r.setEnd(node, offset);
    return r;
  }
  return null;
}

/** Replace `range` with plain `text` through execCommand("insertText"), so Ctrl+Z undoes it. False
 *  if it could not (the selection is put back). Not body.ts's replaceUndoably(range, html): that one
 *  inserts HTML (insertHTML) and may fall back to a non-undoable edit; a typed word must stay text. */
function insertTextOver(root: HTMLElement, range: Range, text: string): boolean {
  const doc = root.ownerDocument;
  const sel = doc.getSelection();
  if (!sel || typeof doc.execCommand !== "function") return false;
  const saved = sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
  sel.removeAllRanges();
  sel.addRange(range);
  if (doc.execCommand("insertText", false, text)) return true;
  sel.removeAllRanges();
  if (saved) sel.addRange(saved);
  return false;
}

/** Attach the typing fixes above to the compose page; returns the function that detaches them.
 *  `replyOnly(block)` says where a direction may be set (not in the quoted original). */
export function attachTypingFixes(root: HTMLElement, opts: { replyOnly: (block: Element) => boolean }): () => void {
  const doc = root.ownerDocument;
  let capsOn = false;
  // the keyboard's direction (null = not known yet)
  let rtl: boolean | null = null;

  const caret = () => {
    const sel = doc.getSelection();
    if (!sel || !sel.rangeCount || !root.contains(sel.anchorNode)) return null;
    return sel.getRangeAt(0);
  };

  /** The empty block at the caret (in the reply) takes the keyboard's direction. */
  const directEmpty = () => {
    const r = caret();
    if (rtl === null || !r || !r.collapsed) return;
    const b = blockAt(root, r.startContainer);
    // (a zero-width space is the font size picked on an empty line, see ComposeWindow fontSize)
    if (!b || (b.textContent ?? "").replace(/\u200b/g, "") !== "" || b.querySelector("img,table") || !opts.replyOnly(b)) return;
    const want = rtl ? "rtl" : "ltr";
    if (b.getAttribute("dir") !== want) b.setAttribute("dir", want);
  };
  const setRtl = (v: boolean) => {
    rtl = v;
    directEmpty();
  };
  // (only a change of keyboard turns the empty line at the caret: an unchanged one would rewrite a
  // line Ctrl+Z just put back)
  const poll = () =>
    rpc<{ lang: string; rtl: boolean } | null>("app.keyboard.layout", {})
      .then((l) => l && l.rtl !== rtl && setRtl(l.rtl))
      .catch(() => {});

  /** Hebrew-layout word before the caret (selection start) → English; the selection and `after`
   *  are replaced too. */
  const fixLayout = (after: string): boolean => {
    const r = caret();
    if (!r) return false;
    const block = blockAt(root, r.startContainer);
    if (!block) return false;
    const word = wordBefore(textBefore(block, r.startContainer, r.startOffset));
    const fixed = word && fromHebrewLayout(word);
    if (!fixed) return false;
    const range = rangeBefore(block, r.startContainer, r.startOffset, word.length);
    if (!range) return false;
    range.setEnd(r.endContainer, r.endOffset);
    if (!insertTextOver(root, range, fixed + after)) return false;
    // a paragraph that went right-to-left for the Hebrew keyboard turns back
    if (block.getAttribute("dir") === "rtl" && !RTL_TEXT.test(block.textContent ?? "") && opts.replyOnly(block)) block.setAttribute("dir", "ltr");
    rtl = false;
    void rpc("app.keyboard.switchToEnglish", {}).catch(() => {});
    return true;
  };

  /** The web address just typed before the caret becomes a link (an undo step of its own). */
  const autoLink = () => {
    const r = caret();
    const block = r && r.collapsed ? blockAt(root, r.startContainer) : null;
    if (!r || !block || r.startContainer.parentElement?.closest("a")) return;
    const u = urlBefore(textBefore(block, r.startContainer, r.startOffset));
    // the address and the punctuation after it, then the end moved back to the address's end
    const range = u && rangeBefore(block, r.startContainer, r.startOffset, u.url.length + u.trail);
    const trail = u?.trail ? rangeBefore(block, r.startContainer, r.startOffset, u.trail) : null;
    const sel = doc.getSelection();
    if (!u || !range || !sel || (u.trail && !trail)) return;
    if (trail) range.setEnd(trail.startContainer, trail.startOffset);
    const back = r.cloneRange();
    sel.removeAllRanges();
    sel.addRange(range);
    doc.execCommand("createLink", false, u.href);
    // the caret back where it was (after any punctuation)
    if (back.startContainer.isConnected) {
      sel.removeAllRanges();
      sel.addRange(back);
    } else sel.collapseToEnd();
  };

  /** "- ", "* " or "1. " typed at the start of a line in the reply (not in a list or a table) turns
   *  the line into a list: the marker is deleted and the list command run (two undo steps: Ctrl+Z
   *  gives a plain line, then the marker back). True when it did. */
  const listShortcut = (): boolean => {
    const r = caret();
    const block = r && r.collapsed ? blockAt(root, r.startContainer) : null;
    if (!r || !block || block.closest("li, td, th") || !opts.replyOnly(block)) return false;
    const text = textBefore(block, r.startContainer, r.startOffset);
    const cmd = listMarker(text);
    const marker = cmd && rangeBefore(block, r.startContainer, r.startOffset, text.length);
    const sel = doc.getSelection();
    if (!cmd || !marker || !sel) return false;
    sel.removeAllRanges();
    sel.addRange(marker);
    doc.execCommand("delete");
    doc.execCommand(cmd);
    return true;
  };

  const onKeyDown = (e: KeyboardEvent) => {
    capsOn = e.getModifierState?.("CapsLock") ?? false;
    if (e.key === " " && !e.ctrlKey && !e.altKey && !e.shiftKey && listShortcut()) return e.preventDefault();
    if ((e.key === " " || e.key === "Enter") && !e.ctrlKey && !e.altKey) autoLink();
    // the keyboard in use, from what a letter key types (Shift / Caps Lock give Latin capitals on
    // the Hebrew layout, so only plain keys count)
    if (/^Key[A-Z]$/.test(e.code) && e.key.length === 1 && !e.shiftKey && !capsOn && !e.ctrlKey && !e.altKey) {
      if (RTL_TEXT.test(e.key)) setRtl(true);
      else if (/[a-z]/.test(e.key)) setRtl(false);
    }
    if (e.key === "Enter" && !e.shiftKey && caret()?.collapsed) fixLayout("");
    // the new empty paragraph Enter makes (or the one Backspace empties) takes the keyboard's direction
    if (e.key === "Enter" || e.key === "Backspace" || e.key === "Delete") setTimeout(directEmpty, 0);
  };

  const onBeforeInput = (e: InputEvent) => {
    if (e.inputType !== "insertText" || e.isComposing || !e.data || e.data.length !== 1) return;
    const text = e.data;
    if (capsOn && /\p{L}/u.test(text)) {
      const r = caret();
      const block = r && blockAt(root, r.startContainer);
      if (r && block) {
        const word = /\p{L}*$/u.exec(textBefore(block, r.startContainer, r.startOffset))![0] + text;
        const fixed = capsFix(word);
        const range = fixed && rangeBefore(block, r.startContainer, r.startOffset, word.length - 1);
        if (fixed && range) {
          range.setEnd(r.endContainer, r.endOffset);
          if (insertTextOver(root, range, fixed)) {
            e.preventDefault();
            capsOn = false;
            void rpc("app.keyboard.capsLockOff", {}).catch(() => {});
            return;
          }
        }
      }
    }
    if (/[\s?!:;)\]"]/.test(text) && fixLayout(text)) e.preventDefault();
  };

  const onKeyUp = (e: KeyboardEvent) => {
    // Alt+Shift / Win+Space / Ctrl+Shift switch the keyboard
    if (["Shift", "Alt", "Control", "Meta"].includes(e.key)) setTimeout(() => void poll(), 60);
  };
  const onFocus = () => void poll();
  // a new empty paragraph (Enter, deleting a line's text) takes the keyboard's direction; not a
  // line Ctrl+Z / Ctrl+Y emptied, which must come back exactly as it was
  const onInput = (e: Event) => {
    if (!/^history/.test((e as InputEvent).inputType ?? "")) directEmpty();
  };
  // the language bar can switch the keyboard too
  const timer = setInterval(() => {
    if (root.contains(doc.activeElement)) void poll();
  }, 1500);

  root.addEventListener("keydown", onKeyDown);
  root.addEventListener("beforeinput", onBeforeInput);
  root.addEventListener("keyup", onKeyUp);
  root.addEventListener("focusin", onFocus);
  root.addEventListener("input", onInput);
  return () => {
    clearInterval(timer);
    root.removeEventListener("keydown", onKeyDown);
    root.removeEventListener("beforeinput", onBeforeInput);
    root.removeEventListener("keyup", onKeyUp);
    root.removeEventListener("focusin", onFocus);
    root.removeEventListener("input", onInput);
  };
}
