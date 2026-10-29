// "Going to more than one company" check before Send: a message to several people at orbit.example and
// one at random.com may have picked the wrong address. Your own domains don't count (Cc a colleague
// freely), and a reply only warns when it adds someone from a company that wasn't already in the
// conversation (Reply All to a client and their broker is normal).

export interface DomainCount {
  domain: string;
  count: number;
}

export function domainOf(email: string): string {
  const at = email.lastIndexOf("@");
  return at < 0
    ? ""
    : email
        .slice(at + 1)
        .trim()
        .toLowerCase();
}

/** `d` is one of `own`, or a subdomain of one (mail.example.com for example.com). */
function isOwn(d: string, own: Set<string>): boolean {
  for (const o of own) if (d === o || d.endsWith(`.${o}`)) return true;
  return false;
}

/**
 * The companies (e-mail domains) the message goes to, most people first — or null when there is
 * nothing to warn about: fewer than two outside companies, or (a reply) no company that wasn't in
 * the conversation already.
 * @param recipients To, Cc and Bcc addresses
 * @param ownEmails your own addresses (every account and Send-as address)
 * @param conversation the addresses already in the conversation being answered (none for a new message)
 */
export function companiesWarning(recipients: string[], ownEmails: string[], conversation: string[] = []): DomainCount[] | null {
  const own = new Set(ownEmails.map(domainOf).filter(Boolean));
  const counts = new Map<string, number>();
  for (const r of new Set(recipients.map((e) => e.trim().toLowerCase()))) {
    const d = domainOf(r);
    if (!d || isOwn(d, own)) continue;
    counts.set(d, (counts.get(d) ?? 0) + 1);
  }
  if (counts.size < 2) return null;
  const known = new Set(conversation.map(domainOf));
  if ([...counts.keys()].every((d) => known.has(d))) return null;
  return [...counts].map(([domain, count]) => ({ domain, count })).sort((a, b) => b.count - a.count || a.domain.localeCompare(b.domain));
}
