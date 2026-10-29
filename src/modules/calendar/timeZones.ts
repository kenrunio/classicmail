// Time zones for an appointment's start and end (Time Zones in the appointment window): the times
// are typed as a clock in that zone (10:00 in New York) and stored as the instant they mean; the
// calendar shows them in your own time. IANA names ("America/New_York"), as Google Calendar uses.

export function localZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

const parts = new Map<string, Intl.DateTimeFormat>();
function fmt(tz: string): Intl.DateTimeFormat {
  let f = parts.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    parts.set(tz, f);
  }
  return f;
}

function fields(ms: number, tz: string) {
  const o: Record<string, number> = {};
  for (const p of fmt(tz).formatToParts(new Date(ms))) if (p.type !== "literal") o[p.type] = Number(p.value);
  return { y: o.year, mo: o.month, d: o.day, h: o.hour % 24, mi: o.minute, s: o.second };
}

/** How far `tz`'s clock is ahead of UTC at `ms` (ms; New York in winter: −5 h). */
export function offsetAt(ms: number, tz: string): number {
  const f = fields(ms, tz);
  return Date.UTC(f.y, f.mo - 1, f.d, f.h, f.mi, f.s) - Math.floor(ms / 1000) * 1000;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** The clock in `tz` at `ms`: its day ("2026-10-26") and minutes after midnight. */
export function wallOf(ms: number, tz: string): { date: string; min: number } {
  const f = fields(ms, tz);
  return { date: `${f.y}-${pad(f.mo)}-${pad(f.d)}`, min: f.h * 60 + f.mi };
}

/** The instant `tz`'s clock shows `date` at `min` minutes. A time skipped by a clock change (02:30
 *  on a spring-forward night) comes out an hour later, as calendars do. */
export function epochOf(date: string, min: number, tz: string): number {
  const [y, mo, d] = date.split("-").map(Number);
  const asUtc = Date.UTC(y, mo - 1, d, Math.floor(min / 60), min % 60);
  let t = asUtc - offsetAt(asUtc, tz);
  t = asUtc - offsetAt(t, tz); // the offset on the day itself (around a clock change)
  return t;
}

/** The zones offered, by region as Outlook names them (one IANA zone stands for each). */
const ZONES: [string, string][] = [
  ["Pacific/Honolulu", "Hawaii"],
  ["America/Anchorage", "Alaska"],
  ["America/Los_Angeles", "Pacific Time (US & Canada)"],
  ["America/Phoenix", "Arizona"],
  ["America/Denver", "Mountain Time (US & Canada)"],
  ["America/Chicago", "Central Time (US & Canada)"],
  ["America/Mexico_City", "Mexico City"],
  ["America/New_York", "Eastern Time (US & Canada)"],
  ["America/Bogota", "Bogota, Lima"],
  ["America/Halifax", "Atlantic Time (Canada)"],
  ["America/Sao_Paulo", "Brasilia, Sao Paulo"],
  ["America/Argentina/Buenos_Aires", "Buenos Aires"],
  ["UTC", "Coordinated Universal Time"],
  ["Europe/London", "London, Dublin, Lisbon"],
  ["Europe/Paris", "Central Europe (Paris, Berlin, Zurich, Rome, Madrid)"],
  ["Africa/Lagos", "West Central Africa"],
  ["Europe/Athens", "Eastern Europe (Athens, Helsinki, Kyiv)"],
  ["Asia/Jerusalem", "Israel"],
  ["Africa/Cairo", "Cairo"],
  ["Africa/Johannesburg", "South Africa"],
  ["Europe/Istanbul", "Istanbul"],
  ["Europe/Moscow", "Moscow"],
  ["Asia/Riyadh", "Arabia (Riyadh, Kuwait, Qatar)"],
  ["Asia/Dubai", "Gulf (Dubai, Abu Dhabi)"],
  ["Asia/Karachi", "Pakistan"],
  ["Asia/Kolkata", "India"],
  ["Asia/Bangkok", "Bangkok, Hanoi, Jakarta"],
  ["Asia/Shanghai", "China, Hong Kong, Singapore, Taiwan"],
  ["Australia/Perth", "Perth"],
  ["Asia/Tokyo", "Japan, Korea"],
  ["Australia/Brisbane", "Brisbane"],
  ["Australia/Sydney", "Sydney, Melbourne, Canberra"],
  ["Pacific/Auckland", "New Zealand"],
];
const NAMES = new Map(ZONES);

/** "Eastern Time (US & Canada)"; a zone not in the list by its city ("Toronto"). */
export function zoneName(tz: string): string {
  return NAMES.get(tz) ?? (tz.split("/").pop() ?? tz).replace(/_/g, " ");
}

/** "(UTC-05:00) Eastern Time (US & Canada)" — the offset at `at`, then the zone's name. */
export function zoneLabel(tz: string, at = Date.now()): string {
  const off = Math.round(offsetAt(at, tz) / 60000);
  const sign = off < 0 ? "-" : "+";
  return `(UTC${sign}${pad(Math.floor(Math.abs(off) / 60))}:${pad(Math.abs(off) % 60)}) ${zoneName(tz)}`;
}

/** The regions to choose from, west to east, labelled for `at` — plus your own zone and `extra`
 *  (an item set in a zone the list doesn't name) when they aren't in it. */
export function zoneList(at = Date.now(), extra: string[] = []): { tz: string; label: string }[] {
  const all = new Set([...ZONES.map(([tz]) => tz), localZone(), ...extra]);
  return [...all]
    .map((tz) => ({ tz, off: offsetAt(at, tz), label: zoneLabel(tz, at) }))
    .sort((a, b) => a.off - b.off || a.label.localeCompare(b.label))
    .map(({ tz, label }) => ({ tz, label }));
}
