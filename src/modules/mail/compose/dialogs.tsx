// Compose dialogs and pop-ups: Message Options (importance, tracking, replies, delay delivery),
// Insert Hyperlink, Insert Table grid, Symbol, colour palettes, Check Names.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "../../../ui";
import { type Addr, type Importance, type Suggestion, formatAddr, parseAddresses } from "../../../api/mailCompose";

// ---- generic pop-up anchored under a button ----------------------------------------------------

export function Popover({ anchor, onClose, children }: { anchor: DOMRect; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);
  const left = Math.min(anchor.left, window.innerWidth - 260);
  return createPortal(
    <div ref={ref} className="cm-compose-pop" style={{ left, top: anchor.bottom + 1 }} onMouseDown={(e) => e.preventDefault()}>
      {children}
    </div>,
    document.body,
  );
}

// Office 2007 colour picker: theme colours row + standard colours.
const THEME = ["#FFFFFF", "#000000", "#EEECE1", "#1F497D", "#4F81BD", "#C0504D", "#9BBB59", "#8064A2", "#4BACC6", "#F79646"];
const STANDARD = ["#C00000", "#FF0000", "#FFC000", "#FFFF00", "#92D050", "#00B050", "#00B0F0", "#0070C0", "#002060", "#7030A0"];
const HIGHLIGHTS = [
  "#FFFF00",
  "#00FF00",
  "#00FFFF",
  "#FF00FF",
  "#0000FF",
  "#FF0000",
  "#000080",
  "#008080",
  "#008000",
  "#800080",
  "#800000",
  "#808000",
  "#808080",
  "#C0C0C0",
  "#000000",
];

export type PaletteKind = "font" | "highlight" | "shading" | "border";

export function ColorPalette({ kind, onPick, onClose, anchor }: { kind: PaletteKind; onPick: (c: string | null) => void; onClose: () => void; anchor: DOMRect }) {
  const sw = (c: string) => (
    <button
      key={c}
      className="cm-compose-swatch"
      style={{ background: c }}
      title={c}
      onClick={() => {
        onPick(c);
        onClose();
      }}
    />
  );
  return (
    <Popover anchor={anchor} onClose={onClose}>
      {kind !== "highlight" ? (
        <div style={{ width: 196 }}>
          <button
            className="cm-compose-pop-item"
            onClick={() => {
              onPick(null);
              onClose();
            }}
          >
            <span className="cm-compose-swatch" style={{ background: kind === "shading" ? "#fff" : "#000" }} /> {kind === "shading" ? "No Color" : "Automatic"}
          </button>
          <div className="cm-compose-pop-head">Theme Colors</div>
          <div className="cm-compose-swatches">{THEME.map(sw)}</div>
          <div className="cm-compose-pop-head">Standard Colors</div>
          <div className="cm-compose-swatches">{STANDARD.map(sw)}</div>
        </div>
      ) : (
        <div style={{ width: 120 }}>
          <div className="cm-compose-swatches" style={{ gridTemplateColumns: "repeat(5, 18px)" }}>
            {HIGHLIGHTS.map(sw)}
          </div>
          <button
            className="cm-compose-pop-item"
            onClick={() => {
              onPick(null);
              onClose();
            }}
          >
            No Color
          </button>
        </div>
      )}
    </Popover>
  );
}

/** Insert › Table: hover a grid (10×8) like Office, or "Insert Table…" for bigger ones. */
export function TableGrid({ anchor, onPick, onClose }: { anchor: DOMRect; onPick: (rows: number, cols: number) => void; onClose: () => void }) {
  const [hover, setHover] = useState<[number, number]>([0, 0]);
  const [custom, setCustom] = useState(false);
  const [rows, setRows] = useState(2);
  const [cols, setCols] = useState(5);
  if (custom)
    return (
      <Dialog
        title="Insert Table"
        onClose={onClose}
        width={280}
        footer={
          <>
            <button
              className="cm-button default"
              onClick={() => {
                onPick(Math.max(1, rows), Math.max(1, cols));
                onClose();
              }}
            >
              OK
            </button>
            <button className="cm-button" onClick={onClose}>
              Cancel
            </button>
          </>
        }
      >
        <div className="cm-form">
          <label>Number of columns:</label>
          <input className="cm-input" type="number" min={1} max={63} value={cols} onChange={(e) => setCols(+e.target.value)} style={{ width: 60 }} />
          <label>Number of rows:</label>
          <input className="cm-input" type="number" min={1} max={200} value={rows} onChange={(e) => setRows(+e.target.value)} style={{ width: 60 }} />
        </div>
      </Dialog>
    );
  return (
    <Popover anchor={anchor} onClose={onClose}>
      <div className="cm-compose-pop-head">{hover[0] ? `${hover[1]}x${hover[0]} Table` : "Insert Table"}</div>
      <div className="cm-compose-tgrid" onMouseLeave={() => setHover([0, 0])}>
        {Array.from({ length: 8 }, (_, r) =>
          Array.from({ length: 10 }, (_, c) => (
            <div
              key={`${r}-${c}`}
              className={`cm-compose-tcell${r < hover[0] && c < hover[1] ? " on" : ""}`}
              onMouseEnter={() => setHover([r + 1, c + 1])}
              onClick={() => {
                onPick(r + 1, c + 1);
                onClose();
              }}
            />
          )),
        )}
      </div>
      <button className="cm-compose-pop-item" onClick={() => setCustom(true)}>
        Insert Table…
      </button>
    </Popover>
  );
}

export const SYMBOLS = [
  "€",
  "£",
  "¥",
  "₪",
  "©",
  "®",
  "™",
  "±",
  "≠",
  "≤",
  "≥",
  "÷",
  "×",
  "∞",
  "µ",
  "α",
  "β",
  "π",
  "Ω",
  "∑",
  "°",
  "•",
  "…",
  "–",
  "—",
  "‘",
  "’",
  "“",
  "”",
  "§",
  "¶",
  "†",
  "→",
  "←",
  "↑",
  "↓",
  "✓",
  "☺",
  "♥",
  "½",
  "¼",
  "¾",
];

export function SymbolDialog({ onPick, onClose }: { onPick: (s: string) => void; onClose: () => void }) {
  const [sel, setSel] = useState(SYMBOLS[0]);
  return (
    <Dialog
      title="Symbol"
      onClose={onClose}
      width={420}
      footer={
        <>
          <button
            className="cm-button default"
            onClick={() => {
              onPick(sel);
              onClose();
            }}
          >
            Insert
          </button>
          <button className="cm-button" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="cm-compose-symgrid">
        {SYMBOLS.map((s) => (
          <button
            key={s}
            className={`cm-compose-sym${s === sel ? " sel" : ""}`}
            onClick={() => setSel(s)}
            onDoubleClick={() => {
              onPick(s);
              onClose();
            }}
          >
            {s}
          </button>
        ))}
      </div>
      <div style={{ marginTop: 8, color: "var(--text-muted)" }}>Character code: U+{sel.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}</div>
    </Dialog>
  );
}

/** Include › Web Page or MD: a web address, or a page or Markdown document saved on this computer
 *  (Browse…). */
export function WebPageDialog({ onOk, onClose }: { onOk: (source: string) => Promise<void>; onClose: () => void }) {
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState(false);
  const ok = async () => {
    if (!source.trim() || busy) return;
    setBusy(true);
    try {
      await onOk(source.trim());
      onClose();
    } finally {
      setBusy(false);
    }
  };
  const browse = async () => {
    const f = await openFileDialog({
      multiple: false,
      title: "Insert Web Page or Markdown",
      filters: [
        { name: "Web Pages and Markdown", extensions: ["html", "htm", "md", "markdown"] },
        { name: "Web Pages", extensions: ["html", "htm"] },
        { name: "Markdown", extensions: ["md", "markdown"] },
      ],
    }).catch(() => null);
    if (typeof f === "string") setSource(f);
  };
  return (
    <Dialog
      title="Insert Web Page or Markdown"
      onClose={onClose}
      width={520}
      footer={
        <>
          <button className="cm-button default" disabled={busy || !source.trim()} onClick={() => void ok()}>
            {busy ? "Loading…" : "Insert"}
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div
        className="cm-form"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void ok();
          }
        }}
      >
        <label>Web address or file:</label>
        {/* the box takes the rest of the row (min-width 0: it may shrink), Browse… beside it */}
        <div style={{ display: "flex", gap: 6, minWidth: 0 }}>
          <input
            className="cm-input"
            style={{ flex: 1, minWidth: 0 }}
            value={source}
            placeholder="https://… or a file: a web page or Markdown (.md)"
            onChange={(e) => setSource(e.target.value)}
            disabled={busy}
            autoFocus
          />
          <button className="cm-button" onClick={() => void browse()} disabled={busy}>
            Browse…
          </button>
        </div>
        <span className="cm-muted" style={{ gridColumn: "1 / -1" }}>
          A web page goes into the message as it looks on screen; Markdown as formatted text. Both are ready to edit.
        </span>
      </div>
    </Dialog>
  );
}

export function HyperlinkDialog({
  initialText,
  initialUrl,
  onOk,
  onClose,
  onRemove,
}: {
  initialText: string;
  initialUrl: string;
  onOk: (text: string, url: string) => void;
  onClose: () => void;
  onRemove?: () => void;
}) {
  const [text, setText] = useState(initialText);
  const [url, setUrl] = useState(initialUrl);
  const ok = () => {
    let u = url.trim();
    if (!u) return;
    if (!/^[a-z][a-z0-9+.-]*:/i.test(u)) u = u.includes("@") && !u.includes("/") ? `mailto:${u}` : `http://${u}`;
    onOk(text.trim() || u, u);
    onClose();
  };
  return (
    <Dialog
      title="Insert Hyperlink"
      onClose={onClose}
      width={460}
      footer={
        <>
          {onRemove && (
            <button
              className="cm-button"
              style={{ marginRight: "auto" }}
              onClick={() => {
                onRemove();
                onClose();
              }}
            >
              Remove Link
            </button>
          )}
          <button className="cm-button default" onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div
        className="cm-form"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            ok();
          }
        }}
      >
        <label>Text to display:</label>
        <input className="cm-input" value={text} onChange={(e) => setText(e.target.value)} />
        <label>Address:</label>
        <input className="cm-input" value={url} placeholder="http://" onChange={(e) => setUrl(e.target.value)} autoFocus />
      </div>
    </Dialog>
  );
}

function toLocalInput(ms: number): { date: string; time: string } {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return { date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`, time: `${pad(d.getHours())}:${pad(d.getMinutes())}` };
}

/** Tomorrow 8:00 (the default "Do not deliver before"). */
function defaultDefer(): number {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  d.setHours(8, 0, 0, 0);
  return d.getTime();
}

export interface MessageOptions {
  importance: Importance;
  requestReadReceipt: boolean;
  requestDeliveryReceipt: boolean;
  deferUntil: number | null;
  replyTo: Addr[];
  flagForRecipients: string | null;
}

/** Outlook 2007 "Message Options" (Options › Delay Delivery / Direct Replies To / Properties). */
export function MessageOptionsDialog({ value, onOk, onClose }: { value: MessageOptions; onOk: (v: MessageOptions) => void; onClose: () => void }) {
  const [v, setV] = useState(value);
  const [deferOn, setDeferOn] = useState(value.deferUntil != null);
  const init = toLocalInput(value.deferUntil ?? defaultDefer());
  const [date, setDate] = useState(init.date);
  const [time, setTime] = useState(init.time);
  const [replyOn, setReplyOn] = useState(value.replyTo.length > 0);
  const [replyText, setReplyText] = useState(value.replyTo.map(formatAddr).join("; "));
  const ok = () => {
    const deferUntil = deferOn ? new Date(`${date}T${time || "08:00"}`).getTime() : null;
    onOk({ ...v, deferUntil: deferUntil && !isNaN(deferUntil) ? deferUntil : null, replyTo: replyOn ? parseAddresses(replyText) : [] });
    onClose();
  };
  return (
    <Dialog
      title="Message Options"
      onClose={onClose}
      width={560}
      footer={
        <>
          <button className="cm-button default" onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <fieldset className="cm-fieldset">
        <legend>Message settings</legend>
        <div className="cm-form">
          <label>Importance:</label>
          <select className="cm-input" value={v.importance} onChange={(e) => setV({ ...v, importance: e.target.value as Importance })} style={{ width: 120 }}>
            <option value="low">Low</option>
            <option value="normal">Normal</option>
            <option value="high">High</option>
          </select>
        </div>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Voting and Tracking options</legend>
        <label className="cm-compose-check">
          <input type="checkbox" checked={v.requestDeliveryReceipt} onChange={(e) => setV({ ...v, requestDeliveryReceipt: e.target.checked })} />
          Request a delivery receipt for this message
        </label>
        <label className="cm-compose-check">
          <input type="checkbox" checked={v.requestReadReceipt} onChange={(e) => setV({ ...v, requestReadReceipt: e.target.checked })} />
          Request a read receipt for this message
        </label>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Delivery options</legend>
        <div className="cm-compose-optrow">
          <label className="cm-compose-check">
            <input type="checkbox" checked={replyOn} onChange={(e) => setReplyOn(e.target.checked)} />
            Have replies sent to:
          </label>
          <input className="cm-input cm-fill" disabled={!replyOn} value={replyText} onChange={(e) => setReplyText(e.target.value)} />
        </div>
        <div className="cm-compose-optrow">
          <label className="cm-compose-check">
            <input type="checkbox" checked={deferOn} onChange={(e) => setDeferOn(e.target.checked)} />
            Do not deliver before:
          </label>
          <input className="cm-input" type="date" disabled={!deferOn} value={date} onChange={(e) => setDate(e.target.value)} />
          <input className="cm-input" type="time" disabled={!deferOn} value={time} onChange={(e) => setTime(e.target.value)} />
        </div>
        <div className="cm-compose-optrow">
          <label className="cm-compose-check">
            <input type="checkbox" checked={v.flagForRecipients != null} onChange={(e) => setV({ ...v, flagForRecipients: e.target.checked ? "Follow up" : null })} />
            Flag for recipients:
          </label>
          <select
            className="cm-input"
            disabled={v.flagForRecipients == null}
            value={v.flagForRecipients ?? "Follow up"}
            onChange={(e) => setV({ ...v, flagForRecipients: e.target.value })}
          >
            {["Follow up", "Call", "Do not Forward", "For Your Information", "Forward", "No Response Necessary", "Read", "Reply", "Reply to All", "Review"].map((f) => (
              <option key={f}>{f}</option>
            ))}
          </select>
        </div>
      </fieldset>
    </Dialog>
  );
}

/** Check Names when a name matches several entries ("Microsoft Office Outlook found more than one…"). */
export function CheckNamesDialog({ name, candidates, onPick, onClose }: { name: string; candidates: Suggestion[]; onPick: (s: Suggestion | null) => void; onClose: () => void }) {
  const [sel, setSel] = useState(0);
  return (
    <Dialog
      title="Check Names"
      onClose={() => {
        onPick(null);
        onClose();
      }}
      width={460}
      footer={
        <>
          <button
            className="cm-button default"
            disabled={!candidates.length}
            onClick={() => {
              onPick(candidates[sel] ?? null);
              onClose();
            }}
          >
            OK
          </button>
          <button
            className="cm-button"
            onClick={() => {
              onPick(null);
              onClose();
            }}
          >
            Cancel
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 6 }}>
        ClassicMail found more than one "<b>{name}</b>". Select the address to use:
      </div>
      <div className="cm-compose-list">
        {candidates.map((c, i) => (
          <div
            key={c.email}
            className={`cm-compose-list-row${i === sel ? " sel" : ""}`}
            onClick={() => setSel(i)}
            onDoubleClick={() => {
              onPick(c);
              onClose();
            }}
          >
            <span className="cm-fill">{c.name || c.email}</span>
            <span style={{ color: "var(--text-muted)" }}>{c.email}</span>
          </div>
        ))}
      </div>
    </Dialog>
  );
}

/** Properties (Office button): a read-only summary like Outlook's message properties sheet. */
export function PropertiesDialog({ rows, onClose }: { rows: [string, string][]; onClose: () => void }) {
  return (
    <Dialog
      title="Properties"
      onClose={onClose}
      width={420}
      footer={
        <button className="cm-button default" onClick={onClose}>
          Close
        </button>
      }
    >
      <div className="cm-form">
        {rows.map(([k, v]) => (
          <div key={k} style={{ display: "contents" }}>
            <label>{k}:</label>
            <span style={{ userSelect: "text" }}>{v}</span>
          </div>
        ))}
      </div>
    </Dialog>
  );
}
