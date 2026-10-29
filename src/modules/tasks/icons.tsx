// Small glyphs used only by the Tasks module and the To-Do Bar (own SVGs, Outlook 2007 spirit).
import type { Category } from "../../api/calendar";
import { categoryColor, useCategories } from "../calendar/categories";

/** Quick Flag: a pennant in the given shade, or a check mark once complete. */
export function FlagGlyph({ color, complete, size = 14, outline }: { color?: string; complete?: boolean; size?: number; outline?: boolean }) {
  if (complete)
    return (
      <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ display: "block", flex: "none" }}>
        <path d="M3 8.5l3 3 7-7.5" fill="none" stroke="#3d8a2e" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ display: "block", flex: "none" }}>
      <path d="M3.5 1.5v13" stroke="#555" strokeWidth="1.2" />
      {outline ? (
        <path d="M4 2.2h8.5l-2.2 3 2.2 3H4z" fill="none" stroke="#9a9a9a" strokeWidth="1" />
      ) : (
        <path d="M4 2.2h8.5l-2.2 3 2.2 3H4z" fill={color ?? "#c4302b"} stroke="rgba(0,0,0,.35)" strokeWidth=".7" />
      )}
    </svg>
  );
}

/** Task item icon: a clipboard with a tick. */
export function TaskGlyph({ size = 16, done }: { size?: number; done?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ display: "block", flex: "none" }}>
      <rect x="2.5" y="2.5" width="11" height="12.5" rx="1" fill="#c99a52" stroke="#7a5a26" />
      <rect x="4" y="4" width="8" height="9.5" fill="#fff" />
      <rect x="5.5" y="1.2" width="5" height="2.6" rx=".6" fill="#d9d9d9" stroke="#666" strokeWidth=".8" />
      <path d="M5 8.4l1.4 1.4 3.2-3.3" fill="none" stroke={done ? "#8a8a8a" : "#2f9a2f"} strokeWidth="1.3" />
      <path d="M5 12h5" stroke="#9aa8bd" />
    </svg>
  );
}

/** Envelope (flagged mail in the To-Do list). */
export function MailGlyph({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ display: "block", flex: "none" }}>
      <rect x="1.5" y="3.5" width="13" height="9" rx=".8" fill="#fdf6d8" stroke="#a88a3a" />
      <path d="M1.8 4l6.2 5 6.2-5" fill="none" stroke="#a88a3a" />
    </svg>
  );
}

/** Outlook 2007 list check box (complete column). */
export function CheckGlyph({ checked }: { checked: boolean }) {
  return (
    <svg width={13} height={13} viewBox="0 0 13 13" aria-hidden style={{ display: "block", flex: "none" }}>
      <rect x=".5" y=".5" width="12" height="12" fill="#fff" stroke="#8a9bb5" />
      {checked && <path d="M2.8 6.6l2.4 2.4 5-5.3" fill="none" stroke="#1d4f91" strokeWidth="1.7" />}
    </svg>
  );
}

export function BellGlyph({ size = 12 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden style={{ display: "block", flex: "none" }}>
      <path d="M8 2a4 4 0 00-4 4v3.5L2.5 12h11L12 9.5V6a4 4 0 00-4-4z" fill="#f5cc4a" stroke="#9c7b17" />
      <circle cx="8" cy="13.5" r="1.4" fill="#9c7b17" />
    </svg>
  );
}

/** The colour of a category in the user's Color Categories (white when it has none or is unknown). */
const chipColor = (name: string, list: Category[]) => categoryColor(name, list) ?? "#fff";

export function CategoryChips({ names, compact }: { names: string[]; compact?: boolean }) {
  const list = useCategories();
  if (!names.length) return null;
  if (compact)
    return (
      <span className="cm-tasks-catboxes" title={names.join(", ")}>
        {names.map((n) => (
          <span key={n} className="cm-tasks-catbox" style={{ background: chipColor(n, list) }} />
        ))}
      </span>
    );
  return (
    <span className="cm-tasks-cats" title={names.join(", ")}>
      {names.map((n) => (
        <span key={n} className="cm-tasks-cat" style={{ background: chipColor(n, list) }}>
          {n}
        </span>
      ))}
    </span>
  );
}
