// View › Sort By for the contacts views: the sort orders, a locale-aware comparison (Hebrew after
// Latin, like contacts::collate on the Rust side), the index-tab bucket of each contact for the
// active order, and the "File As" display order (Last, First / First Last).
import { indexKey, type Contact } from "../../api/contacts";

export type SortKey = "fileAs" | "firstName" | "lastName" | "company" | "added" | "modified";

/** `key` is a SortKey, or a table column id after clicking a column header in a table view. */
export interface SortSpec {
  key: string;
  desc: boolean;
}

export const SORT_KEYS: { id: SortKey; label: string }[] = [
  { id: "firstName", label: "First Name" },
  { id: "lastName", label: "Last Name" },
  { id: "company", label: "Company" },
  { id: "fileAs", label: "File As" },
  { id: "added", label: "Date Added" },
  { id: "modified", label: "Date Modified" },
];

export function isSortKey(k: string): k is SortKey {
  return SORT_KEYS.some((s) => s.id === k);
}

export function sortLabel(k: string): string {
  return SORT_KEYS.find((s) => s.id === k)?.label ?? "File As";
}

export const isDateKey = (k: string) => k === "added" || k === "modified";

export type NameOrder = "lastFirst" | "firstLast";

/**
 * One fixed collation (not the system locale) so the order is the same everywhere: case- and
 * accent-insensitive, Hebrew vowel points ignored, Hebrew after Latin.
 */
export const collator = new Intl.Collator("en", { sensitivity: "base", usage: "sort" });

export function compareText(a: string, b: string): number {
  return collator.compare(a.trim(), b.trim()) || (a < b ? -1 : a > b ? 1 : 0);
}

/**
 * The File As text to show. With "First Last" the default "Last, First" filing shows as
 * "First Last"; a custom File As ("Harbor Capital (Cohen, Sarah)") is shown as typed.
 */
export function shownFileAs(c: Contact, order: NameOrder): string {
  if (order === "firstLast") {
    const first = [c.givenName, c.middleName]
      .map((x) => x.trim())
      .filter(Boolean)
      .join(" ");
    const last = [c.familyName, c.suffix]
      .map((x) => x.trim())
      .filter(Boolean)
      .join(" ");
    if (first && last && c.fileAs === `${last}, ${first}`) return `${first} ${last}`;
  }
  return c.fileAs || c.displayName;
}

const firstOf = (c: Contact) => c.givenName.trim() || c.fileAs || c.displayName;
const lastOf = (c: Contact) => c.familyName.trim() || c.fileAs || c.displayName;

/** The text a contact is sorted / indexed by (null for the date orders). */
export function sortText(c: Contact, key: string, order: NameOrder): string | null {
  switch (key) {
    case "firstName":
      return firstOf(c);
    case "lastName":
      return lastOf(c);
    case "company":
      return c.company.trim();
    case "added":
    case "modified":
      return null;
    default:
      return shownFileAs(c, order);
  }
}

/** Index-tab bucket for the active order ("123", "a".."z", "א"); null = no index tabs (dates, or no company). */
export function indexKeyFor(c: Contact, key: string, order: NameOrder): string | null {
  const t = sortText(c, key, order);
  if (t === null || (key === "company" && !t)) return null;
  return indexKey(t);
}

/** Text for the "Abramson – Cohen" range in the card views' title bar. */
export function rangeText(c: Contact, key: string, order: NameOrder): string {
  if (key === "added") return shortDate(c.firstSeenAt);
  if (key === "modified") return shortDate(c.updated);
  if (key === "firstName") return [c.givenName, c.familyName].filter(Boolean).join(" ") || shownFileAs(c, order);
  if (key === "company") return c.company || "(none)";
  return sortText(c, key, order) ?? "";
}

/** Ascending order of two contacts for `key`; ties fall back to File As. */
export function compareContacts(a: Contact, b: Contact, key: string, order: NameOrder): number {
  const byFileAs = () => compareText(shownFileAs(a, order), shownFileAs(b, order)) || a.id - b.id;
  switch (key) {
    case "firstName":
      return compareText(firstOf(a), firstOf(b)) || compareText(lastOf(a), lastOf(b)) || byFileAs();
    case "lastName":
      return compareText(lastOf(a), lastOf(b)) || compareText(firstOf(a), firstOf(b)) || byFileAs();
    case "company":
      return compareText(a.company, b.company) || byFileAs();
    case "added":
      return a.firstSeenAt - b.firstSeenAt || byFileAs();
    case "modified":
      return a.updated - b.updated || byFileAs();
    default:
      return byFileAs();
  }
}

/**
 * Sort a copy of `items`. Unknown keys (table-only columns) sort by File As here; the table views
 * handle their own columns. Contacts without a company stay last when sorting by company, in
 * either direction.
 */
export function sortContacts(items: Contact[], spec: SortSpec, order: NameOrder): Contact[] {
  const key = isSortKey(spec.key) ? spec.key : "fileAs";
  const sign = spec.desc ? -1 : 1;
  return [...items].sort((a, b) => {
    if (key === "company") {
      const ea = !a.company.trim();
      const eb = !b.company.trim();
      if (ea !== eb) return ea ? 1 : -1;
    }
    return sign * compareContacts(a, b, key, order);
  });
}

const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dateFmt = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "numeric", day: "numeric" });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/** Outlook's table date: "Sun 10/11/2026 3:04 PM". */
export function columnDate(ms: number): string {
  if (!ms) return "None";
  const d = new Date(ms);
  return `${dayFmt.format(d)} ${dateFmt.format(d)} ${timeFmt.format(d)}`;
}

export function shortDate(ms: number): string {
  return ms ? dateFmt.format(new Date(ms)) : "";
}
