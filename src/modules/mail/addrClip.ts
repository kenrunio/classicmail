// Names in To / Cc / Bcc as Outlook puts them on the clipboard: "Name <email>; Name <email>".
// Pasting that text into a recipient box gives the same names back.

interface A {
  name: string;
  email: string;
}

/** One address: a name with a comma, semicolon or angle bracket is quoted ("Green, Kenny"). */
export function addrClipText(a: A): string {
  const name = a.name.trim();
  const email = a.email.trim();
  if (!email) return name;
  if (!name || name.toLowerCase() === email.toLowerCase()) return email;
  return /[,;<>"@]/.test(name) ? `"${name.replace(/"/g, "")}" <${email}>` : `${name} <${email}>`;
}

export function addrsClipText(list: A[]): string {
  return list.map(addrClipText).join("; ");
}

/** Drag & drop of names between To / Cc / Bcc (and from a message being read): a JSON list of
 *  { name, email }, alongside the same names as text for anything else that takes a drop. */
export const NAMES_MIME = "application/x-classicmail-names";

/** Click / Ctrl+click / Shift+click selection over a row of names (sorted indexes). */
export function clickSelect(sel: number[], anchor: number | null, i: number, mods: { ctrl?: boolean; shift?: boolean }): { sel: number[]; anchor: number } {
  if (mods.shift && anchor !== null) return { sel: span(anchor, i), anchor };
  if (mods.ctrl) return { sel: sel.includes(i) ? sel.filter((j) => j !== i) : [...sel, i].sort((a, b) => a - b), anchor: i };
  return { sel: [i], anchor: i };
}

/** The indexes from a to b, either way round. */
export function span(a: number, b: number): number[] {
  const out: number[] = [];
  for (let i = Math.min(a, b); i <= Math.max(a, b); i++) out.push(i);
  return out;
}

/** Puts text on the clipboard at once, through a copy event (works without page focus, unlike
 *  navigator.clipboard); falls back to navigator.clipboard if the copy command is refused. */
export function copyText(text: string) {
  const put = (e: ClipboardEvent) => {
    e.clipboardData?.setData("text/plain", text);
    e.preventDefault();
    e.stopImmediatePropagation();
  };
  document.addEventListener("copy", put, { capture: true, once: true });
  const ok = document.execCommand("copy");
  document.removeEventListener("copy", put, { capture: true });
  if (!ok) navigator.clipboard?.writeText(text).catch(() => {});
}
