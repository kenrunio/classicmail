// Outlook's Date Navigator: one or more stacked mini-months, bold days that have appointments,
// today boxed in red, the visible range highlighted. Click = select a day; drag = select a range;
// click on a week row's left margin = that week.
import { addDays, addMonths, endOfMonth, format, isSameDay, isSameMonth, startOfDay, startOfMonth, startOfWeek } from "date-fns";
import { useRef } from "react";

/** A local day as "yyyy-MM-dd": the one day key used by the navigator, the calendar and tasks. */
export function dayKey(d: Date | number) {
  return format(d, "yyyy-MM-dd");
}

export function DateNavigator({
  month,
  onMonthChange,
  months = 1,
  selected,
  onSelect,
  boldDays,
  weekStartsOn = 0,
  compact,
}: {
  /** first month shown */
  month: Date;
  onMonthChange: (m: Date) => void;
  months?: number;
  /** highlighted range (inclusive) */
  selected?: { start: Date; end: Date };
  onSelect?: (start: Date, end: Date) => void;
  /** yyyy-MM-dd keys */
  boldDays?: Set<string>;
  weekStartsOn?: 0 | 1 | 6;
  compact?: boolean;
}) {
  const drag = useRef<Date | null>(null);
  const today = new Date();
  const inSel = (d: Date) => selected && d >= startOfDay(selected.start) && d <= startOfDay(selected.end);
  return (
    <div className={`cm-datenav${compact ? " compact" : ""}`} onMouseUp={() => (drag.current = null)} onMouseLeave={() => (drag.current = null)}>
      {Array.from({ length: months }, (_, mi) => {
        const m = addMonths(startOfMonth(month), mi);
        const gridStart = startOfWeek(m, { weekStartsOn });
        const weeks = Math.ceil((endOfMonth(m).getTime() - gridStart.getTime()) / (7 * 86400000) + 0.0001);
        const rows = Math.max(weeks, 6);
        return (
          <div key={mi} className="cm-datenav-month">
            <div className="cm-datenav-head">
              {mi === 0 ? (
                <button className="cm-datenav-arrow" onClick={() => onMonthChange(addMonths(month, -1))} title="Previous month">
                  ◂
                </button>
              ) : (
                <span className="cm-datenav-arrow" />
              )}
              <span className="cm-fill">{format(m, "MMMM yyyy")}</span>
              {mi === months - 1 ? (
                <button className="cm-datenav-arrow" onClick={() => onMonthChange(addMonths(month, 1))} title="Next month">
                  ▸
                </button>
              ) : (
                <span className="cm-datenav-arrow" />
              )}
            </div>
            <div className="cm-datenav-grid">
              <div className="cm-datenav-wk" />
              {Array.from({ length: 7 }, (_, i) => (
                <div key={i} className="cm-datenav-dow">
                  {format(addDays(gridStart, i), "EEEEEE").slice(0, 2)}
                </div>
              ))}
              {Array.from({ length: rows }, (_, w) => {
                const ws = addDays(gridStart, w * 7);
                return [
                  <div key={`w${w}`} className="cm-datenav-wk" title="Select this week" onMouseDown={() => onSelect?.(ws, addDays(ws, 6))} />,
                  ...Array.from({ length: 7 }, (_, i) => {
                    const d = addDays(ws, i);
                    const other = !isSameMonth(d, m);
                    const cls = ["cm-datenav-day", other && "other", isSameDay(d, today) && "today", inSel(d) && "sel", boldDays?.has(dayKey(d)) && "bold"]
                      .filter(Boolean)
                      .join(" ");
                    return (
                      <div
                        key={`${w}-${i}`}
                        className={cls}
                        onMouseDown={(e) => {
                          e.preventDefault();
                          drag.current = d;
                          onSelect?.(d, d);
                        }}
                        onMouseEnter={() => {
                          if (drag.current) {
                            const a = drag.current < d ? drag.current : d;
                            const b = drag.current < d ? d : drag.current;
                            onSelect?.(a, b);
                          }
                        }}
                      >
                        {format(d, "d")}
                      </div>
                    );
                  }),
                ];
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}
