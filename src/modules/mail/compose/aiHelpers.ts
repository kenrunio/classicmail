// Pure helpers behind the compose window's Reply to One / Reply All toggle, Translate my reply
// and AI Review (docs/superpowers/specs/2026-10-13-reply-ai-design.md). No editor or rpc here.

export interface AddrLike {
  name: string;
  email: string;
}

export interface ReplyTargets {
  one: { to: AddrLike[]; cc: AddrLike[] };
  all: { to: AddrLike[]; cc: AddrLike[] };
}

const key = (a: AddrLike) => a.email.trim().toLowerCase();

function dedupe<T extends AddrLike>(list: T[], skip = new Set<string>()): T[] {
  const seen = new Set(skip);
  return list.filter((a) => {
    const k = key(a);
    if (!k || seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

/**
 * Recipients after pressing Reply to One (`toOne`) or Reply All: the target set plus any names
 * the user added that are in neither set (they stay where they were).
 */
export function toggleReply<T extends AddrLike>(cur: { to: T[]; cc: T[] }, t: ReplyTargets, toOne: boolean, make: (a: AddrLike) => T): { to: T[]; cc: T[] } {
  const known = new Set([...t.one.to, ...t.one.cc, ...t.all.to, ...t.all.cc].map(key));
  const extraTo = cur.to.filter((a) => !known.has(key(a)));
  const extraCc = cur.cc.filter((a) => !known.has(key(a)));
  const base = toOne ? t.one : t.all;
  // keep the user's own spelling of a name that is in the target set
  const pick = (a: AddrLike) => [...cur.to, ...cur.cc].find((c) => key(c) === key(a)) ?? make(a);
  const to = dedupe([...base.to.map(pick), ...extraTo]);
  const cc = dedupe([...base.cc.map(pick), ...extraCc], new Set(to.map(key)));
  return { to, cc };
}

/** Is there anything to toggle (more people on Reply All than on Reply to One)? */
export function canToggle(t: ReplyTargets | null | undefined): boolean {
  if (!t) return false;
  const one = new Set([...t.one.to, ...t.one.cc].map(key));
  return [...t.all.to, ...t.all.cc].some((a) => !one.has(key(a)));
}

/** The toggle shows "Reply to One" while more than one address is on the message. */
export function showsReplyAll(cur: { to: AddrLike[]; cc: AddrLike[] }): boolean {
  return new Set([...cur.to, ...cur.cc].map(key).filter(Boolean)).size > 1;
}

/** A mail-merge key the way the merge compares them: `First Name` = `first_name` = `FIRSTNAME`. */
export const keyNorm = (s: string) => s.replace(/[ _\-\u00a0]/g, "").toLowerCase();

/**
 * The `{…}` instructions in a text, matching the back end's `ai::compose::instructions`; `fields`
 * are mail-merge column names, whose `{Field}` placeholders are not instructions. Square brackets
 * are ordinary text (Outlook's "[mailto:…]", "[EXTERNAL]", "[1]"…).
 */
export function instructions(text: string, fields: string[] = []): string[] {
  const skip = new Set(fields.map(keyNorm));
  const out: string[] = [];
  for (const m of text.matchAll(/\{([^{}\n]*)\}/g)) {
    const inner = m[1].trim();
    if (inner && inner !== "?" && !skip.has(keyNorm(inner))) out.push(inner);
  }
  return out;
}

/** "RE: FW: Q3 call" → { prefix: "RE: FW: ", rest: "Q3 call" }: the reply / forward markers
 *  (also Hebrew and other mail clients' ones) aren't translated. */
export function splitSubject(subject: string): { prefix: string; rest: string } {
  const m = /^((?:\s*(?:re|fw|fwd|aw|wg|tr|sv|vs|rv|השב|העבר|תשובה)\s*:\s*)*)([\s\S]*)$/i.exec(subject)!;
  return { prefix: m[1], rest: m[2] };
}

const norm = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * Index of the first top-level block of the trailing signature, given each block's text, or null
 * when the text after the reply isn't the signature (none, or the user typed below it).
 */
export function signatureStart(blockTexts: string[], signatureText: string): number | null {
  const sig = norm(signatureText);
  if (!sig) return null;
  for (let i = blockTexts.length - 1; i >= 0; i--) {
    const tail = norm(blockTexts.slice(i).join("\n"));
    if (tail === sig) return i;
    if (tail.length > sig.length) return null;
  }
  return null;
}
