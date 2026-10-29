// Keys as Windows sees them: shortcuts by key, whatever the keyboard layout, and access keys
// (the underlined letter on a button or menu item).
import type { ReactNode } from "react";

// a punctuation key's US character, for layouts where it types something else
const PUNCT: Record<string, string> = {
  Period: ".",
  Comma: ",",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
};

/**
 * The key pressed, lower-case, for matching shortcuts. With a non-Latin layout on (Hebrew, say)
 * the R key types "ר", but Ctrl+R is still Ctrl+R in Windows, so a key that types a non-Latin
 * character falls back to its place on the keyboard. A digit key is always its digit (Ctrl+Shift+1,
 * not "!"; on a French keyboard Ctrl+1, not "&").
 */
export function keyOf(e: { key: string; code?: string }): string {
  const k = e.key;
  if (k.length !== 1) return k.toLowerCase();
  const code = e.code ?? "";
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  if (k.charCodeAt(0) > 0x7e) {
    const letter = /^Key([A-Z])$/.exec(code);
    if (letter) return letter[1].toLowerCase();
    if (PUNCT[code]) return PUNCT[code];
  }
  return k.toLowerCase();
}

/** Each label's access key: the first letter or digit of it that an earlier label hasn't taken. */
export function accessKeys(labels: string[]): (string | null)[] {
  const taken = new Set<string>();
  return labels.map((l) => {
    const k = [...l.toLowerCase()].find((c) => /[a-z0-9]/.test(c) && !taken.has(c)) ?? null;
    if (k) taken.add(k);
    return k;
  });
}

/** The label with its access key underlined. */
export function KeyLabel({ label, k }: { label: string; k: string | null }): ReactNode {
  const i = k ? label.toLowerCase().indexOf(k) : -1;
  if (i < 0) return label;
  return (
    <>
      {label.slice(0, i)}
      <u>{label[i]}</u>
      {label.slice(i + 1)}
    </>
  );
}
