// Outlook-style date and time combo boxes used by the appointment and recurrence dialogs.
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { format, startOfMonth } from "date-fns";
import { DateNavigator, Icon } from "../../../ui";
import { dayKey, fmtDuration, parseDateText, parseDayKey, parseTime, SHORT_DATE, uses12h } from "../dates";

function usePopup() {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest(".cm-cal-popup") && !anchor.current?.contains(t)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault(); // the Esc closes the drop-down, not the window
      setOpen(false);
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [open]);
  const pos = () => {
    const r = anchor.current?.getBoundingClientRect();
    return r ? { left: r.left, top: r.bottom + 1, minWidth: r.width } : { left: 0, top: 0 };
  };
  return { open, setOpen, anchor, pos };
}

/** "Sun 10/11/2026 ▾" — value is yyyy-MM-dd. */
export function DateField({ value, onChange, disabled, width = 130 }: { value: string; onChange: (v: string) => void; disabled?: boolean; width?: number }) {
  const { open, setOpen, anchor, pos } = usePopup();
  const [text, setText] = useState("");
  const [month, setMonth] = useState(() => startOfMonth(parseDayKey(value)));
  useEffect(() => setText(format(parseDayKey(value), `EEE ${SHORT_DATE}`)), [value]);
  const commit = () => {
    const d = parseDateText(text);
    if (d) onChange(dayKey(d));
    else setText(format(parseDayKey(value), `EEE ${SHORT_DATE}`));
  };
  return (
    <div className={`cm-cal-combo${disabled ? " disabled" : ""}`} ref={anchor} style={{ width }}>
      <input
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
      <button
        tabIndex={-1}
        disabled={disabled}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => {
          setMonth(startOfMonth(parseDayKey(value)));
          setOpen(!open);
        }}
      >
        <Icon name="chevron-down" size={10} />
      </button>
      {open &&
        createPortal(
          <div className="cm-cal-popup" style={{ ...pos(), width: 200 }}>
            <DateNavigator
              month={month}
              onMonthChange={setMonth}
              selected={{ start: parseDayKey(value), end: parseDayKey(value) }}
              onSelect={(d) => {
                onChange(dayKey(d));
                setOpen(false);
              }}
            />
            <div className="cm-cal-popup-foot">
              <button className="cm-button" onClick={() => (onChange(dayKey(new Date())), setOpen(false))}>
                Today
              </button>
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}

export function fmtMinutes(m: number): string {
  const d = new Date(2000, 0, 1, Math.floor(m / 60), m % 60);
  return format(d, uses12h ? "h:mm a" : "HH:mm");
}

/** Time combo. `durationFrom` (minutes of the start on the same day) adds "(30 minutes)" hints. */
export function TimeField({
  value,
  onChange,
  disabled,
  durationFrom,
  width = 110,
}: {
  value: number;
  onChange: (m: number) => void;
  disabled?: boolean;
  durationFrom?: number;
  width?: number;
}) {
  const { open, setOpen, anchor, pos } = usePopup();
  const [text, setText] = useState(fmtMinutes(value));
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => setText(fmtMinutes(value)), [value]);
  useEffect(() => {
    if (open) listRef.current?.querySelector(".sel")?.scrollIntoView({ block: "center" });
  }, [open]);
  const commit = () => {
    const m = parseTime(text);
    if (m !== null) onChange(m);
    else setText(fmtMinutes(value));
  };
  const start = durationFrom ?? 0;
  const options = Array.from({ length: 48 }, (_, i) => i * 30).filter((m) => durationFrom === undefined || m >= start);
  if (durationFrom !== undefined) options.unshift(...[0, 15].map((x) => start + x).filter((m) => m % 30 !== 0 && m < 1440));
  options.sort((a, b) => a - b);
  return (
    <div className={`cm-cal-combo${disabled ? " disabled" : ""}`} ref={anchor} style={{ width }}>
      <input
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
      <button tabIndex={-1} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={() => setOpen(!open)}>
        <Icon name="chevron-down" size={10} />
      </button>
      {open &&
        createPortal(
          <div className="cm-cal-popup cm-cal-timelist" ref={listRef} style={{ ...pos(), minWidth: durationFrom !== undefined ? 190 : width }}>
            {options.map((m) => (
              <div
                key={m}
                className={m === value ? "sel" : ""}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => {
                  onChange(m);
                  setOpen(false);
                }}
              >
                {fmtMinutes(m)}
                {durationFrom !== undefined && <span className="cm-muted"> ({fmtDuration(m - start)})</span>}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
