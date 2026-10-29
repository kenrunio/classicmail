// Contacts DTOs (mirror src-tauri/src/contacts) and small helpers shared by the contacts module,
// its windows and other domains (Select Names, autocomplete).
import { rpc } from "./rpc";
import { maybeUndoable as via } from "./undo";

export interface ContactEmail {
  value: string;
  /** Google type: work | home | other | custom */
  type: string;
  /** Outlook "Display as" */
  displayName: string;
}

export interface Typed {
  /** Google type, e.g. phones: work | home | mobile | workFax | homeFax | otherFax | pager | main | other */
  type: string;
  value: string;
}

export interface PostalAddress {
  /** work | home | other */
  type: string;
  street: string;
  city: string;
  region: string;
  postalCode: string;
  country: string;
}

export interface Im {
  protocol: string;
  username: string;
}

export interface Contact {
  id: number;
  accountId: number;
  resourceName: string | null;
  etag: string | null;
  displayName: string;
  givenName: string;
  middleName: string;
  familyName: string;
  prefix: string;
  suffix: string;
  nickname: string;
  fileAs: string;
  company: string;
  jobTitle: string;
  department: string;
  office: string;
  profession: string;
  manager: string;
  assistant: string;
  spouse: string;
  emails: ContactEmail[];
  phones: Typed[];
  addresses: PostalAddress[];
  /** type of the address marked "This is the mailing address" */
  mailingAddress: string;
  urls: Typed[];
  ims: Im[];
  /** YYYY-MM-DD, or --MM-DD without a year */
  birthday: string | null;
  anniversary: string | null;
  notes: string;
  photoUrl: string | null;
  /** contact group resource names */
  groups: string[];
  starred: boolean;
  /** last change on Google (People metadata updateTime) or local edit, epoch ms ("Date Modified") */
  updated: number;
  /** when the contact first appeared in ClassicMail, epoch ms ("Date Added") */
  firstSeenAt: number;
  /** local changes not yet on Google */
  dirty: boolean;
}

export type ContactInput = Partial<Contact>;

export interface ContactGroup {
  id: number;
  accountId: number;
  resourceName: string;
  name: string;
  count: number;
}

export interface ContactFolder {
  accountId: number;
  email: string;
  displayName: string;
  color: string;
  count: number;
  groups: ContactGroup[];
}

export interface Suggestion {
  name: string;
  email: string;
  source: "contact" | "other" | "recent";
}

/** contacts.findByEmail result; phone `type` here is already the Outlook label ("Business"...). */
export interface ContactCard {
  id: number;
  accountId: number;
  displayName: string;
  company: string;
  jobTitle: string;
  phones: Typed[];
  photoUrl?: string;
}

export interface ListParams {
  accountId?: number | null;
  groupId?: number | null;
  query?: string;
  letter?: string;
  /** fileAs (default) | firstName | lastName | company | added | modified */
  sortBy?: string;
  descending?: boolean;
}

export const emptyContact = (): Contact => ({
  id: 0,
  accountId: 0,
  resourceName: null,
  etag: null,
  displayName: "",
  givenName: "",
  middleName: "",
  familyName: "",
  prefix: "",
  suffix: "",
  nickname: "",
  fileAs: "",
  company: "",
  jobTitle: "",
  department: "",
  office: "",
  profession: "",
  manager: "",
  assistant: "",
  spouse: "",
  emails: [],
  phones: [],
  addresses: [],
  mailingAddress: "",
  urls: [],
  ims: [],
  birthday: null,
  anniversary: null,
  notes: "",
  photoUrl: null,
  groups: [],
  starred: false,
  updated: 0,
  firstSeenAt: 0,
  dirty: false,
});

// ---- rpc wrappers ---------------------------------------------------------------------------

export const contactsApi = {
  list: (p: ListParams) => rpc<Contact[]>("contacts.list", p),
  get: (id: number) => rpc<Contact>("contacts.get", { id }),
  // with an undo `label` the change is recorded in the Undo history (Ctrl+Z)
  create: (contact: ContactInput, accountId?: number | null, label?: string) => via<Contact>(label, "contacts.create", { contact, accountId: accountId ?? undefined }),
  update: (id: number, contact: ContactInput, label?: string) => via<Contact>(label, "contacts.update", { id, contact }),
  remove: (ids: number[], label?: string) => via<number>(label, "contacts.delete", { ids }),
  move: (ids: number[], accountId: number, label?: string) => via<number[]>(label, "contacts.move", { ids, accountId }),
  setPhoto: (id: number, dataUrl: string) => rpc<Contact>("contacts.setPhoto", { id, dataUrl }),
  folders: () => rpc<ContactFolder[]>("contacts.folders", {}),
  autocomplete: (query: string, limit = 10) => rpc<Suggestion[]>("contacts.autocomplete", { query, limit }),
  findByEmail: (email: string) => rpc<ContactCard | null>("contacts.findByEmail", { email }),
  search: (query: string, limit = 50) => rpc<Contact[]>("contacts.search", { query, limit }),
};

// ---- phones ---------------------------------------------------------------------------------

/** Outlook phone labels in the order of Outlook's drop-down, with the Google type each maps to. */
export const PHONE_TYPES: { label: string; type: string }[] = [
  { label: "Assistant", type: "assistant" },
  { label: "Business", type: "work" },
  { label: "Business 2", type: "work" },
  { label: "Business Fax", type: "workFax" },
  { label: "Callback", type: "callback" },
  { label: "Car", type: "car" },
  { label: "Company Main", type: "main" },
  { label: "Home", type: "home" },
  { label: "Home 2", type: "home" },
  { label: "Home Fax", type: "homeFax" },
  { label: "ISDN", type: "isdn" },
  { label: "Mobile", type: "mobile" },
  { label: "Other", type: "other" },
  { label: "Other Fax", type: "otherFax" },
  { label: "Pager", type: "pager" },
  { label: "Primary", type: "primary" },
  { label: "Radio", type: "radio" },
  { label: "Telex", type: "telex" },
  { label: "TTY/TDD", type: "ttyTdd" },
];

const LABEL_FOR_TYPE: Record<string, string> = {
  work: "Business",
  workMobile: "Mobile",
  workPager: "Pager",
  googleVoice: "Other",
};
for (const p of PHONE_TYPES) if (!LABEL_FOR_TYPE[p.type]) LABEL_FOR_TYPE[p.type] = p.label;

export function phoneLabel(type: string): string {
  return LABEL_FOR_TYPE[type] ?? (type ? type[0].toUpperCase() + type.slice(1) : "Other");
}

export function phoneTypeForLabel(label: string): string {
  return PHONE_TYPES.find((p) => p.label === label)?.type ?? "other";
}

/** Phones keyed by Outlook label ("Business", "Business 2", "Home"...), as the contact form edits them. */
export function phonesByLabel(phones: Typed[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of phones) {
    let label = phoneLabel(p.type);
    if (out[label] !== undefined) {
      if (out[`${label} 2`] === undefined && PHONE_TYPES.some((t) => t.label === `${label} 2`)) label = `${label} 2`;
      else if (out.Other === undefined) label = "Other";
      else continue;
    }
    out[label] = p.value;
  }
  return out;
}

export function phonesFromLabels(map: Record<string, string>): Typed[] {
  return PHONE_TYPES.filter((t) => (map[t.label] ?? "").trim()).map((t) => ({ type: t.type, value: map[t.label].trim() }));
}

export function phone(c: Contact, label: string): string {
  return phonesByLabel(c.phones)[label] ?? "";
}

// ---- names, file as -------------------------------------------------------------------------

export const TITLES = ["Dr.", "Miss", "Mr.", "Mrs.", "Ms.", "Prof."];
export const SUFFIXES = ["I", "II", "III", "Jr.", "Sr."];

/** Split "Dr. John Q. Public Jr." into Outlook's name parts. "Last, First" is understood too. */
export function parseFullName(full: string): Pick<Contact, "prefix" | "givenName" | "middleName" | "familyName" | "suffix"> {
  const out = { prefix: "", givenName: "", middleName: "", familyName: "", suffix: "" };
  const s = full.trim().replace(/\s+/g, " ");
  if (!s) return out;
  if (s.includes(",")) {
    // "Last, First Middle"
    const last = s.slice(0, s.indexOf(",")).trim();
    const r = parseFullName(s.slice(s.indexOf(",") + 1));
    return { ...r, familyName: last, middleName: [r.middleName, r.familyName].filter(Boolean).join(" ") };
  }
  const words = s.split(" ");
  const lowerTitles = TITLES.map((t) => t.toLowerCase().replace(".", ""));
  if (words.length > 1 && lowerTitles.includes(words[0].toLowerCase().replace(".", ""))) out.prefix = words.shift()!;
  const lowerSuffixes = SUFFIXES.map((t) => t.toLowerCase().replace(".", ""));
  if (words.length > 1 && lowerSuffixes.includes(words[words.length - 1].toLowerCase().replace(".", ""))) out.suffix = words.pop()!;
  if (words.length === 1) out.givenName = words[0];
  else if (words.length === 2) [out.givenName, out.familyName] = words;
  else {
    out.givenName = words[0];
    out.familyName = words[words.length - 1];
    out.middleName = words.slice(1, -1).join(" ");
  }
  return out;
}

export function fullName(c: Pick<Contact, "prefix" | "givenName" | "middleName" | "familyName" | "suffix">): string {
  return [c.prefix, c.givenName, c.middleName, c.familyName, c.suffix]
    .map((x) => x.trim())
    .filter(Boolean)
    .join(" ");
}

/** The "File as" choices Outlook offers for the current name and company. */
export function fileAsOptions(c: Pick<Contact, "givenName" | "middleName" | "familyName" | "suffix" | "company">): string[] {
  const first = [c.givenName, c.middleName]
    .map((x) => x.trim())
    .filter(Boolean)
    .join(" ");
  const last = [c.familyName, c.suffix]
    .map((x) => x.trim())
    .filter(Boolean)
    .join(" ");
  const company = c.company.trim();
  const opts: string[] = [];
  const add = (s: string) => s && !opts.includes(s) && opts.push(s);
  if (first && last) {
    add(`${last}, ${first}`);
    add(`${first} ${last}`);
    if (company) {
      add(company);
      add(`${last}, ${first} (${company})`);
      add(`${company} (${last}, ${first})`);
    }
  } else if (first || last) {
    add(first || last);
    if (company) {
      add(company);
      add(`${first || last} (${company})`);
      add(`${company} (${first || last})`);
    }
  } else if (company) add(company);
  return opts;
}

export function defaultFileAs(c: Pick<Contact, "givenName" | "middleName" | "familyName" | "suffix" | "company">): string {
  return fileAsOptions(c)[0] ?? "";
}

/** Outlook's default "Display as": "Name (address)". */
export function emailDisplayAs(name: string, email: string): string {
  if (!email) return "";
  return name ? `${name} (${email})` : email;
}

// ---- addresses ------------------------------------------------------------------------------

export const ADDRESS_TYPES: { label: string; type: string }[] = [
  { label: "Business", type: "work" },
  { label: "Home", type: "home" },
  { label: "Other", type: "other" },
];

export function addressLabel(type: string): string {
  return ADDRESS_TYPES.find((a) => a.type === type)?.label ?? "Other";
}

export function addressLines(a: PostalAddress | undefined): string[] {
  if (!a) return [];
  const cityLine = [[a.city, a.region].filter(Boolean).join(", "), a.postalCode].filter(Boolean).join(" ");
  return [...a.street.split("\n"), cityLine, a.country].map((l) => l.trim()).filter(Boolean);
}

export function formatAddress(a: PostalAddress | undefined): string {
  return addressLines(a).join("\n");
}

/** Best-effort parse of a multi-line address typed into the contact form. */
export function parseAddress(text: string, type: string, previous?: PostalAddress): PostalAddress {
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
  const a: PostalAddress = { type, street: "", city: "", region: "", postalCode: "", country: "" };
  if (previous && formatAddress(previous) === lines.join("\n")) return { ...previous, type };
  if (lines.length === 0) return a;
  if (lines.length === 1) {
    a.street = lines[0];
    return a;
  }
  let cityIdx = lines.length - 1;
  if (lines.length >= 3 || (lines.length === 2 && !/\d/.test(lines[1]) && !lines[1].includes(","))) {
    // last line is the country when it has no digits / comma
    if (!/\d/.test(lines[lines.length - 1]) && !lines[lines.length - 1].includes(",")) {
      a.country = lines[lines.length - 1];
      cityIdx = lines.length - 2;
    }
  }
  if (cityIdx <= 0) {
    a.street = lines.slice(0, Math.max(1, cityIdx + (cityIdx === 0 ? 1 : 0))).join("\n");
    return a;
  }
  a.street = lines.slice(0, cityIdx).join("\n");
  const cl = lines[cityIdx];
  // "City, Region 12345" | "City 12345" | "City"
  const m = cl.match(/^(.*?)(?:,\s*([^,]*?))?\s+([0-9][0-9A-Za-z -]{2,9}|[A-Z]{1,2}\d[A-Z\d]? ?\d[A-Z]{2})$/);
  if (m) {
    a.city = m[1].trim();
    a.region = (m[2] ?? "").trim();
    a.postalCode = m[3].trim();
  } else if (cl.includes(",")) {
    a.city = cl.slice(0, cl.indexOf(",")).trim();
    a.region = cl.slice(cl.indexOf(",") + 1).trim();
  } else a.city = cl;
  return a;
}

/** The address cards show the mailing address (else business, home, other). */
export function mainAddress(c: Contact): PostalAddress | undefined {
  return c.addresses.find((a) => a.type === c.mailingAddress) ?? c.addresses.find((a) => a.type === "work") ?? c.addresses.find((a) => a.type === "home") ?? c.addresses[0];
}

// ---- misc -------------------------------------------------------------------------------------

/** Index-tab bucket of a "File as" value ("123", "a".."z", "א"); mirrors contacts::index_key. */
export function indexKey(fileAs: string): string {
  const ch = fileAs.trim().charAt(0);
  if (!ch) return "123";
  const base = ch.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
  if (/^[a-z]$/.test(base)) return base;
  if (/^[֐-׿]$/.test(ch)) return "א";
  return "123";
}

export const INDEX_ORDER = ["123", ..."abcdefghijklmnopqrstuvwxyz".split(""), "א"];

export function primaryEmail(c: Contact): string {
  return c.emails[0]?.value ?? "";
}

/** "Name <email>" for the compose window's To line. */
export function addressFor(c: Contact, email = primaryEmail(c)): string {
  if (!email) return "";
  const name = c.displayName.replace(/[<>"]/g, "");
  if (!name || name === email) return email;
  // "Green, Kenny" must stay one address when the line is split on , and ;
  return /[,;]/.test(name) ? `"${name}" <${email}>` : `${name} <${email}>`;
}

export function formatDate(d: string | null): string {
  if (!d) return "";
  const m = d.match(/^(\d{4}|-)-?(\d{2})-(\d{2})$/);
  if (!m) return d;
  const year = m[1] === "-" ? 2000 : Number(m[1]);
  const date = new Date(year, Number(m[2]) - 1, Number(m[3]));
  return date.toLocaleDateString(undefined, m[1] === "-" ? { day: "numeric", month: "long" } : { day: "numeric", month: "long", year: "numeric" });
}
