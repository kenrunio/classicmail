// Appointment colours derived from each calendar's own colour (Google calendarList colour), the
// way Outlook 2007 tints appointments per calendar. Fixed colours come from theme variables.

function parse(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  const full =
    h.length === 3
      ? h
          .split("")
          .map((c) => c + c)
          .join("")
      : h.padEnd(6, "0");
  const n = parseInt(full.slice(0, 6), 16);
  return isNaN(n) ? [159, 198, 231] : [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function mix(hex: string, target: number, amount: number): string {
  const [r, g, b] = parse(hex);
  const m = (c: number) => Math.round(c + (target - c) * amount);
  return `rgb(${m(r)}, ${m(g)}, ${m(b)})`;
}

export const tint = (hex: string, amount: number) => mix(hex, 255, amount);
export const shade = (hex: string, amount: number) => mix(hex, 0, amount);

export interface ApptColors {
  fillTop: string;
  fillBottom: string;
  border: string;
  stripe: string;
}

export function apptColors(hex: string): ApptColors {
  return { fillTop: tint(hex, 0.82), fillBottom: tint(hex, 0.45), border: shade(hex, 0.38), stripe: shade(hex, 0.22) };
}

/** CSS background for the busy-status stripe on an appointment's left edge. */
export function stripeBackground(status: string, hex: string): string {
  const c = apptColors(hex).stripe;
  switch (status) {
    case "free":
      return "var(--busy-free)";
    case "oof":
      return "var(--busy-oof)";
    case "tentative":
      return `repeating-linear-gradient(45deg, ${c} 0 3px, var(--busy-free) 3px 6px)`;
    default:
      return c;
  }
}
