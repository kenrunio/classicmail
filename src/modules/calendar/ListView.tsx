// Table views: "Search Calendar" results and "View Recurrences" (Recurring Appointments).
import { useMemo, useState } from "react";
import type { CalEvent } from "../../api/calendar";
import { Icon, showMenu } from "../../ui";
import { fmtDate, fmtTime } from "./dates";
import { describeRecurrence } from "./recurrence";
import { itemMenu } from "./menus";
import { deleteItem, openItem } from "./actions";
import { useCal } from "./state";

export type ListRow = CalEvent & { recurrence?: string[] };

type Col = { key: string; label: string; width: string; get: (r: ListRow) => string | number };

export function ListView({ rows, series, emptyText }: { rows: ListRow[]; series?: boolean; emptyText: string }) {
  const { selectedId, set } = useCal();
  const [sort, setSort] = useState<{ key: string; asc: boolean }>({ key: series ? "summary" : "start", asc: !!series });
  const cols: Col[] = [
    { key: "summary", label: "Subject", width: "34%", get: (r) => r.summary.toLowerCase() },
    { key: "location", label: "Location", width: "18%", get: (r) => r.location.toLowerCase() },
    ...(series
      ? [{ key: "pattern", label: "Recurrence Pattern", width: "48%", get: (r: ListRow) => describeRecurrence(r.recurrence, r.start, r.end, r.allDay) }]
      : [
          { key: "start", label: "Start", width: "18%", get: (r: ListRow) => r.start },
          { key: "end", label: "End", width: "18%", get: (r: ListRow) => r.end },
          { key: "recur", label: "Recurrence", width: "12%", get: (r: ListRow) => (r.isRecurring ? "Recurring" : "") },
        ]),
  ];
  const sorted = useMemo(() => {
    const c = cols.find((x) => x.key === sort.key) ?? cols[0];
    return [...rows].sort((a, b) => {
      const x = c.get(a);
      const y = c.get(b);
      const d = x < y ? -1 : x > y ? 1 : 0;
      return sort.asc ? d : -d;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, sort, series]);
  const fmt = (ms: number, allDay: boolean) => (allDay ? fmtDate(ms) : `${fmtDate(ms)} ${fmtTime(ms)}`);

  return (
    <div
      className="cm-cal-list"
      tabIndex={0}
      onKeyDown={(e) => {
        const idx = sorted.findIndex((r) => r.id === selectedId);
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const n = Math.min(sorted.length - 1, Math.max(0, idx + (e.key === "ArrowDown" ? 1 : -1)));
          if (sorted[n]) set({ selectedId: sorted[n].id });
        } else if (e.key === "Enter" && idx >= 0) {
          e.preventDefault();
          openItem(sorted[idx], series ? "series" : undefined);
        } else if (e.key === "Delete" && idx >= 0) {
          // the view's Delete hotkey (CalendarView) leaves a handled key alone
          e.preventDefault();
          deleteItem(sorted[idx]);
        }
      }}
    >
      <div className="cm-cal-list-head">
        <div className="cm-cal-list-icon" />
        {cols.map((c) => (
          <div key={c.key} style={{ width: c.width }} onClick={() => setSort((s) => ({ key: c.key, asc: s.key === c.key ? !s.asc : true }))}>
            {c.label}
            {sort.key === c.key && <span className="cm-cal-sort">{sort.asc ? " ▲" : " ▼"}</span>}
          </div>
        ))}
      </div>
      <div className="cm-cal-list-body">
        {sorted.length === 0 && <div className="cm-empty">{emptyText}</div>}
        {sorted.map((r) => (
          <div
            key={r.id}
            className={`cm-cal-list-row${r.id === selectedId ? " selected" : ""}`}
            onMouseDown={() => set({ selectedId: r.id })}
            onDoubleClick={() => openItem(r, series ? "series" : undefined)}
            onContextMenu={(e) => {
              e.preventDefault();
              set({ selectedId: r.id });
              showMenu(e.clientX, e.clientY, itemMenu(r));
            }}
          >
            <div className="cm-cal-list-icon">
              <Icon name={r.isRecurring ? "recurring" : r.isMeeting ? "meeting" : "appointment"} size={14} />
            </div>
            <div style={{ width: cols[0].width }} className="cm-ellipsis">
              {r.summary || "(No subject)"}
            </div>
            <div style={{ width: cols[1].width }} className="cm-ellipsis">
              {r.location}
            </div>
            {series ? (
              <div style={{ width: cols[2].width }} className="cm-ellipsis">
                {describeRecurrence(r.recurrence, r.start, r.end, r.allDay)}
              </div>
            ) : (
              <>
                <div style={{ width: cols[2].width }} className="cm-ellipsis">
                  {fmt(r.start, r.allDay)}
                </div>
                <div style={{ width: cols[3].width }} className="cm-ellipsis">
                  {fmt(r.allDay ? r.end - 1 : r.end, r.allDay)}
                </div>
                <div style={{ width: cols[4].width }} className="cm-ellipsis">
                  {r.isRecurring ? "Recurring" : ""}
                </div>
              </>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
