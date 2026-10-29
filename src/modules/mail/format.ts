// Outlook-style formatting of dates, sizes and names for the mail list / reading pane.

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const shortDate = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "numeric", day: "numeric" });
const longDate = new Intl.DateTimeFormat(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" });

/** Two-line rows: "10:42 AM" today, "Wed 10:42 AM" this week, else "10/7/2026". */
export function listDate(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const today = startOfDay(new Date(now));
  if (ms >= today) return timeFmt.format(d);
  if (ms >= today - 6 * 86400000) return `${dayFmt.format(d)} ${timeFmt.format(d)}`;
  return shortDate.format(d);
}

/** Received column: "Wed 10/14/2026 10:42 AM". */
export function columnDate(ms: number): string {
  const d = new Date(ms);
  return `${dayFmt.format(d)} ${shortDate.format(d)} ${timeFmt.format(d)}`;
}

/** Reading pane "Sent:" — "Wednesday, October 14, 2026 10:42 AM". */
export function fullDate(ms: number): string {
  const d = new Date(ms);
  return `${longDate.format(d)} ${timeFmt.format(d)}`;
}

/** "Wednesday, October 14, 2026" from "2026-10-14". */
export function longDay(ymd: string): string {
  const [y, m, d] = ymd.split("-").map(Number);
  return longDate.format(new Date(y, m - 1, d));
}

export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

/** The message list's Size column: Outlook shows whole KB ("1 KB" for small messages). */
export function listSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.ceil(bytes / 1024)).toLocaleString()} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

export function addrText(a: { name: string; email: string }): string {
  return a.name ? a.name : a.email;
}

export function addrList(list: { name: string; email: string }[]): string {
  return list.map(addrText).join("; ");
}

/** Text (or an attribute value) made safe to put in HTML. */
export function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

export function todayYmd(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** true for text that starts with a right-to-left letter (Hebrew/Arabic). */
export function isRtl(s: string): boolean {
  const m = s.match(/[A-Za-z֐-׿؀-ۿ]/);
  return !!m && /[֐-׿؀-ۿ]/.test(m[0]);
}
