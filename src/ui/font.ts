// The app-wide UI font: the new Outlook's (Segoe UI Variable, 13px; theme/office2007.css) or a custom
// family / size / weight from Tools › Options › Preferences › Appearance. Applied as CSS variables
// on <html>, so every window and widget follows. (The old "classic" style — Segoe UI 12px — is gone:
// it was hardly different, and size / font cover it.)
import { useSettings } from "../api/settings";

export interface UiFont {
  family?: unknown;
  size?: unknown;
  weight?: unknown;
}

export const FONT_CHOICES = ["Segoe UI Variable Text", "Segoe UI", "Aptos", "Calibri", "Tahoma", "Arial", "Verdana", "Helvetica", "Georgia"];

export const SIZE_CHOICES = [11, 12, 13, 14, 15, 16];

export const WEIGHT_CHOICES: { value: number; label: string }[] = [
  { value: 300, label: "Light" },
  { value: 400, label: "Regular" },
  { value: 600, label: "Semibold" },
];

/** The UI font size (px) when no custom size is set. */
export const DEFAULT_SIZE = 13;

const quote = (f: string) => (/^[\w-]+$/.test(f) ? f : `"${f.replace(/"/g, "")}"`);

export function applyUiFont(f: UiFont | null) {
  const root = document.documentElement;
  const family = typeof f?.family === "string" ? f.family.trim() : "";
  const size = Number(f?.size) || 0;
  const weight = Number(f?.weight) || 400;
  if (family) root.style.setProperty("--font", `${quote(family)}, "Segoe UI", Tahoma, sans-serif`);
  else root.style.removeProperty("--font");
  if (size >= 9 && size <= 24) {
    root.style.setProperty("--font-size", `${size}px`);
    root.style.setProperty("--font-size-small", `${size - 1}px`);
  } else {
    root.style.removeProperty("--font-size");
    root.style.removeProperty("--font-size-small");
  }
  root.style.setProperty("--font-weight", String(weight));
}

/** The effective UI font size in px (lists use it to size their rows). */
export function useUiFontSize(): number {
  const { values } = useSettings();
  return Number(values["app.fontSize"]) || DEFAULT_SIZE;
}
