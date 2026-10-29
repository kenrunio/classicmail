// Usual groups: type a name in To and press Tab, and the people you usually write to with that
// person appear — in To or Cc, where they usually were (src-tauri mail/compose/recipients.rs,
// learnt from your sent mail and the meetings you organised) — highlighted, the cursor still in
// To. Tab keeps them and moves to Cc; Delete, Backspace, Esc, typing or leaving the box drops
// them. Rejected, the next name you add asks again (the group those people share); kept, the rest
// of the message works as usual.
import { useRef, useState } from "react";
import { rpc } from "../../../api/rpc";
import type { Addr } from "../../../api/mailCompose";
import type { Recipient } from "./RecipientField";

/** What the To box calls (RecipientField `group`). */
export interface GroupHooks {
  /** a name was just added with Tab (`list`: the box now): true when people were suggested */
  afterTab: (list: Recipient[]) => Promise<boolean>;
  /** suggestions are showing */
  pending: boolean;
  /** Tab: keep them, then the next box */
  accept: () => void;
  /** anything else: drop them */
  reject: () => void;
}

export interface UsualGroup {
  to: Addr[];
  cc: Addr[];
}

export const usualGroup = (emails: string[]) => rpc<UsualGroup>("mail.recipients.usualGroup", { emails });

/** The suggestions to add: people not already in any box, as highlighted recipients. */
export function newSuggestions(g: UsualGroup, present: Recipient[]): { to: Recipient[]; cc: Recipient[] } {
  const have = new Set(present.map((r) => r.email.toLowerCase()));
  const fresh = (list: Addr[]) =>
    list
      .filter((a) => {
        const k = a.email.toLowerCase();
        if (have.has(k)) return false;
        have.add(k);
        return true;
      })
      .map((a) => ({ name: a.name, email: a.email, resolved: true, suggested: true }));
  return { to: fresh(g.to), cc: fresh(g.cc) };
}

/**
 * The To box's group suggestions. `setCc` absent (a meeting's one attendee box): the usual Cc
 * people go into To too. `next` focuses the box after To (Cc).
 */
export function useUsualGroup(o: {
  to: Recipient[];
  setTo: (v: Recipient[]) => void;
  cc?: Recipient[];
  setCc?: (v: Recipient[]) => void;
  bcc?: Recipient[];
  next: () => void;
}): GroupHooks & { drop: () => void } {
  const [pending, setPending] = useState(false);
  const done = useRef(false);
  const ask = useRef(0);
  const cur = useRef(o);
  cur.current = o;

  const reject = () => {
    const c = cur.current;
    ask.current++;
    if (c.to.some((r) => r.suggested)) c.setTo(c.to.filter((r) => !r.suggested));
    if (c.cc?.some((r) => r.suggested)) c.setCc?.(c.cc.filter((r) => !r.suggested));
    setPending(false);
  };

  return {
    pending,
    afterTab: async (list) => {
      if (done.current) return false;
      const emails = list.filter((r) => r.resolved && !r.suggested).map((r) => r.email);
      if (!emails.length) return false;
      const my = ++ask.current;
      const g = await usualGroup(emails).catch(() => null);
      const c = cur.current;
      if (!g || my !== ask.current) return false;
      const s = newSuggestions(g, [...list, ...(c.cc ?? []), ...(c.bcc ?? [])]);
      const intoTo = c.setCc ? s.to : [...s.to, ...s.cc];
      const intoCc = c.setCc ? s.cc : [];
      if (!intoTo.length && !intoCc.length) return false;
      c.setTo([...list, ...intoTo]);
      if (intoCc.length) c.setCc!([...(c.cc ?? []), ...intoCc]);
      setPending(true);
      return true;
    },
    accept: () => {
      const c = cur.current;
      c.setTo(c.to.map((r) => (r.suggested ? { ...r, suggested: false } : r)));
      if (c.cc?.some((r) => r.suggested)) c.setCc?.(c.cc.map((r) => (r.suggested ? { ...r, suggested: false } : r)));
      done.current = true;
      setPending(false);
      c.next();
    },
    reject,
    /** before Send / Save: suggestions nobody kept don't go out */
    drop: () => {
      if (pending || cur.current.to.some((r) => r.suggested) || cur.current.cc?.some((r) => r.suggested)) reject();
    },
  };
}
