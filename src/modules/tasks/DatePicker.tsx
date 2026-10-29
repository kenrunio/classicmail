// Outlook-style date box: "None" / "Wed 10/14/2026" text you can type into, plus a ▾ that drops
// down a month calendar with Today and None buttons.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { DateNavigator } from "../../ui";
import { fmtDate, parseDayKey, parseDateInput, dayKey } from "./dates";

export function DatePicker({
  value,
  onChange,
  disabled,
  width = 130,
  allowNone = true,
  autoFocus,
  onKeyDown,
  className,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
  disabled?: boolean;
  width?: number;
  allowNone?: boolean;
  autoFocus?: boolean;
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
  className?: string;
}) {
  const [text, setText] = useState(fmtDate(value));
  const [open, setOpen] = useState(false);
  const [bad, setBad] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setText(fmtDate(value));
    setBad(false);
  }, [value]);

  const commit = () => {
    const v = parseDateInput(text);
    if (v === undefined || (v === null && !allowNone)) {
      setBad(true);
      return false;
    }
    setBad(false);
    if (v !== value) onChange(v);
    else setText(fmtDate(value));
    return true;
  };

  return (
    <div ref={wrap} className={`cm-tasks-datepick${disabled ? " disabled" : ""}${className ? ` ${className}` : ""}`} style={{ width }}>
      <input
        className={`cm-input${bad ? " cm-tasks-bad" : ""}`}
        value={text}
        disabled={disabled}
        autoFocus={autoFocus}
        title={bad ? "Outlook cannot understand this date. Type e.g. 'tomorrow', 'fri' or a date." : undefined}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
          if (e.key === "ArrowDown" && e.altKey) {
            e.preventDefault();
            setOpen(true);
          }
          onKeyDown?.(e);
        }}
      />
      <button className="cm-tasks-datepick-btn" disabled={disabled} tabIndex={-1} onMouseDown={(e) => e.preventDefault()} onClick={() => setOpen((o) => !o)}>
        ▾
      </button>
      {open && wrap.current && (
        <DropCalendar
          anchor={wrap.current}
          value={value}
          allowNone={allowNone}
          onPick={(v) => {
            setOpen(false);
            onChange(v);
          }}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}

function DropCalendar({
  anchor,
  value,
  allowNone,
  onPick,
  onClose,
}: {
  anchor: HTMLElement;
  value: string | null;
  allowNone: boolean;
  onPick: (v: string | null) => void;
  onClose: () => void;
}) {
  const [month, setMonth] = useState(() => (value ? parseDayKey(value) : new Date()));
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(() => {
    const r = anchor.getBoundingClientRect();
    return { left: r.left, top: r.bottom + 1 };
  });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const a = anchor.getBoundingClientRect();
    let { left, top } = { left: a.left, top: a.bottom + 1 };
    if (left + r.width > window.innerWidth - 2) left = Math.max(2, window.innerWidth - r.width - 2);
    if (top + r.height > window.innerHeight - 2) top = Math.max(2, a.top - r.height - 1);
    setPos({ left, top });
  }, [anchor]);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.contains(t)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [anchor, onClose]);
  const sel = value ? parseDayKey(value) : undefined;
  return createPortal(
    <div ref={ref} className="cm-tasks-dropcal" style={pos} onMouseDown={(e) => e.preventDefault()}>
      <DateNavigator month={month} onMonthChange={setMonth} selected={sel ? { start: sel, end: sel } : undefined} onSelect={(d) => onPick(dayKey(d))} />
      <div className="cm-tasks-dropcal-foot">
        <button className="cm-button" onClick={() => onPick(dayKey(new Date()))}>
          Today
        </button>
        {allowNone && (
          <button className="cm-button" onClick={() => onPick(null)}>
            None
          </button>
        )}
      </div>
    </div>,
    document.body,
  );
}
