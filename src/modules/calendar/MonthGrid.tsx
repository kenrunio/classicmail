// Outlook 2007 Month view: week rows of day cells, all-day / multi-day bars across a week,
// timed items as "9:00am Subject" lines (Low / Medium / High detail), "more items" arrow,
// drag an item to another day (Ctrl = copy). Select days (click, drag across them, Shift+click or
// Shift+arrows) and type to add an all-day event over them: one day, or a conference's three.
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { addDays, format } from "date-fns";
import { cal, type CalEvent } from "../../api/calendar";
import { errorBox, Icon, showMenu } from "../../ui";
import { allDayRange, DAY, DAY_FIRST, dayStart, fmtShortTime, plusDays, sameDay, dayKey } from "./dates";
import { isBandItem, layoutLanes } from "./layout";
import { apptColors, stripeBackground, tint } from "./color";
import { itemColor, useCategories } from "./categories";
import { itemMenu, newFromSelection, slotMenu } from "./menus";
import { askSendUpdate, openItem } from "./actions";
import { useCal, type CalSettings } from "./state";

const LINE = 17;
const HEAD = 18;

export function MonthGrid({ days, events, settings, anchor }: { days: Date[]; events: CalEvent[]; settings: CalSettings; anchor: Date }) {
  const { selection, selectedId, set, monthDetail } = useCal();
  const cats = useCategories();
  const rootRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [cellH, setCellH] = useState(100);
  // the new event being typed: the selected days [start, end) and its subject
  type Inline = { start: number; end: number; text: string };
  const [inline, setInline0] = useState<Inline | null>(null);
  const inlineRef = useRef<Inline | null>(null);
  const setInline = (v: Inline | null) => {
    inlineRef.current = v;
    setInline0(v);
  };
  const drag = useRef<{ ev: CalEvent; day0: number; moved: boolean; x0: number; y0: number } | null>(null);
  const [dropDay, setDropDay] = useState<number | null>(null);
  const weeks = days.length / 7;
  const today = dayStart(Date.now()).getTime();

  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setCellH(el.clientHeight / weeks));
    ro.observe(el);
    return () => ro.disconnect();
  }, [weeks]);

  const rows = useMemo(
    () =>
      Array.from({ length: weeks }, (_, w) => {
        const wd = days.slice(w * 7, w * 7 + 7);
        const ws = wd[0].getTime();
        const we = plusDays(wd[6], 1).getTime();
        const bandItems = events
          .filter((e) => isBandItem(e) && e.start < we && e.end > ws)
          .map((e) => {
            let first = -1;
            let last = -1;
            wd.forEach((d, i) => {
              if (e.start < plusDays(d, 1).getTime() && e.end > d.getTime()) {
                if (first < 0) first = i;
                last = i;
              }
            });
            return { id: e.id, first, last, ev: e };
          })
          .filter((x) => x.first >= 0);
        const laid = layoutLanes(bandItems);
        const lanesPerDay = wd.map((_, i) => laid.filter((x) => x.item.first <= i && x.item.last >= i).reduce((m, x) => Math.max(m, x.lane + 1), 0));
        const timed = wd.map((d) => {
          const ds = d.getTime();
          const de = plusDays(d, 1).getTime();
          return events.filter((e) => !isBandItem(e) && e.start < de && (e.end > ds || (e.start === e.end && e.start >= ds))).sort((a, b) => a.start - b.start);
        });
        return { wd, laid, lanesPerDay, timed };
      }),
    [events, days, weeks],
  );

  // category colours (subject matching runs regexes): once per data change, not on every render
  const colorById = useMemo(() => new Map(events.map((e) => [e.id, itemColor(e, cats)])), [events, cats]);
  const colorOf = (ev: CalEvent) => colorById.get(ev.id) ?? itemColor(ev, cats);

  const capacity = Math.max(1, Math.floor((cellH - HEAD - 4) / LINE));
  const dayAt = (cx: number, cy: number) => {
    const r = bodyRef.current!.getBoundingClientRect();
    const c = Math.min(6, Math.max(0, Math.floor(((cx - r.left) / r.width) * 7)));
    const w = Math.min(weeks - 1, Math.max(0, Math.floor(((cy - r.top) / r.height) * weeks)));
    return w * 7 + c;
  };
  // the day a Shift+click / Shift+arrow / drag selection grows from, and the day it reaches
  const anchorDay = useRef<number | null>(null);
  const focusDay = useRef<number | null>(null);
  /** Select the days from `a` to `b` (indexes into `days`, either order). */
  const selectDays = (a: number, b: number) => {
    focusDay.current = b;
    const [lo, hi] = a <= b ? [a, b] : [b, a];
    set({ selection: { start: days[lo].getTime(), end: plusDays(days[hi], 1).getTime(), allDay: true }, selectedId: null });
  };
  const selectDay = (i: number) => {
    anchorDay.current = i;
    selectDays(i, i);
  };
  /** Mouse down on a day: select it (Shift: from the anchor to it), and a drag selects the days passed over. */
  const onDayDown = (e: React.MouseEvent, idx: number) => {
    if (e.shiftKey && anchorDay.current !== null && selection) selectDays(anchorDay.current, idx);
    else selectDay(idx);
    rootRef.current?.focus({ preventScroll: true });
    const from = anchorDay.current ?? idx;
    const move = (m: MouseEvent) => selectDays(from, dayAt(m.clientX, m.clientY));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const onItemDown = (e: React.MouseEvent, ev: CalEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    set({ selectedId: ev.id, selection: null });
    rootRef.current?.focus({ preventScroll: true });
    if (ev.readOnly || (ev.isMeeting && !ev.isOrganizer)) return;
    drag.current = { ev, day0: dayAt(e.clientX, e.clientY), moved: false, x0: e.clientX, y0: e.clientY };
    const move = (m: MouseEvent) => {
      const d = drag.current;
      if (!d) return;
      if (!d.moved && Math.abs(m.clientX - d.x0) + Math.abs(m.clientY - d.y0) < 5) return;
      d.moved = true;
      setDropDay(dayAt(m.clientX, m.clientY));
    };
    const up = async (m: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      const d = drag.current;
      drag.current = null;
      setDropDay(null);
      if (!d?.moved) return;
      const delta = Math.round((days[dayAt(m.clientX, m.clientY)].getTime() - days[d.day0].getTime()) / DAY);
      if (!delta) return;
      const s = addDays(d.ev.start, delta).getTime();
      const en = addDays(d.ev.end, delta).getTime();
      const send = m.ctrlKey ? false : await askSendUpdate(d.ev);
      if (send === null) return;
      try {
        const what = `"${d.ev.summary || "Untitled"}"`;
        if (m.ctrlKey) await cal.copy(d.ev.id, s, en, undefined, `Copy ${what}`);
        else if (d.ev.allDay) await cal.update(d.ev.id, { allDay: true, startDate: dayKey(s), endDate: dayKey(en), sendUpdates: send }, "occurrence", `Move ${what}`);
        else await cal.update(d.ev.id, { start: s, end: en, sendUpdates: send }, "occurrence", `Move ${what}`);
      } catch (err) {
        errorBox(err);
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const commitInline = async () => {
    const it = inlineRef.current;
    inlineRef.current = null;
    setInline(null);
    rootRef.current?.focus({ preventScroll: true });
    if (!it || !it.text.trim()) return;
    try {
      const d = await cal.create(
        { summary: it.text.trim(), allDay: true, startDate: dayKey(it.start), endDate: dayKey(it.end), reminderMinutes: -1, busyStatus: "free" },
        `New Event "${it.text.trim()}"`,
      );
      set({ selectedId: d.id, selection: null });
    } catch (e) {
      errorBox(e);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (inline) return;
    const k = e.key;
    if (k.length === 1 && !e.ctrlKey && !e.altKey && selection && selectedId === null) {
      e.preventDefault();
      setInline({ ...allDayRange(selection.start, selection.end), text: k });
      return;
    }
    if (k === "Enter") {
      e.preventDefault();
      const ev = events.find((x) => x.id === selectedId);
      if (ev) openItem(ev);
      else if (selection) newFromSelection(selection, { allDay: true });
      return;
    }
    if (k.startsWith("Arrow")) {
      e.preventDefault();
      const step = k === "ArrowLeft" ? -1 : k === "ArrowRight" ? 1 : k === "ArrowUp" ? -7 : 7;
      // Shift+arrows grow or shrink the selection from its anchor, within the month shown
      if (e.shiftKey && selection && anchorDay.current !== null && focusDay.current !== null) {
        const to = focusDay.current + step;
        if (to >= 0 && to < days.length) selectDays(anchorDay.current, to);
        return;
      }
      const cur = selection ? days.findIndex((d) => sameDay(d, selection.start)) : days.findIndex((d) => d.getTime() === today);
      const next = (cur < 0 ? 0 : cur) + step;
      if (next < 0 || next >= days.length) {
        const target = plusDays(cur < 0 ? days[0] : days[cur], step);
        useCal.getState().set({ anchor: target.getTime(), selection: { start: target.getTime(), end: plusDays(target, 1).getTime(), allDay: true }, selectedId: null });
      } else selectDay(next);
    }
  };

  const itemTitle = (ev: CalEvent) => `${ev.summary}${ev.location ? ` (${ev.location})` : ""}\n${ev.allDay ? "All day" : `${fmtShortTime(ev.start)} - ${fmtShortTime(ev.end)}`}`;

  return (
    <div className="cm-cal-month" ref={rootRef} tabIndex={0} onKeyDown={onKeyDown}>
      <div className="cm-cal-month-head">
        {days.slice(0, 7).map((d) => (
          <div key={d.getDay()} className="cm-cal-month-dow">
            {format(d, "EEEE")}
          </div>
        ))}
      </div>
      <div className="cm-cal-month-body" ref={bodyRef}>
        {rows.map((row, w) => (
          <div key={w} className="cm-cal-week" style={{ height: `${100 / weeks}%` }}>
            {row.wd.map((d, i) => {
              const idx = w * 7 + i;
              const ds = d.getTime();
              const other = d.getMonth() !== anchor.getMonth();
              const isSel = selection && selection.start < plusDays(d, 1).getTime() && selection.end > ds;
              const lanes = row.lanesPerDay[i];
              const timed = monthDetail === "low" ? [] : row.timed[i];
              const room = Math.max(0, capacity - lanes);
              const shown = timed.slice(0, monthDetail === "medium" ? Math.max(room, 0) : room);
              const hidden = timed.length - shown.length + row.laid.filter((x) => x.item.first <= i && x.item.last >= i && x.lane >= capacity).length;
              return (
                <div
                  key={ds}
                  className={`cm-cal-mcell${other ? " other" : ""}${ds === today ? " today" : ""}${isSel ? " sel" : ""}${dropDay === idx ? " drop" : ""}${settings.workDays.includes(d.getDay()) ? "" : " offday"}`}
                  onMouseDown={(e) => {
                    if (e.button !== 0) return;
                    onDayDown(e, idx);
                  }}
                  onDoubleClick={() => newFromSelection({ start: ds, end: plusDays(d, 1).getTime(), allDay: true }, { allDay: true })}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    // inside a selection of several days: New All Day Event… covers them all
                    const inSel = !!selection && selection.start <= ds && selection.end > ds;
                    if (!inSel) selectDay(idx);
                    const sel = inSel && selection ? selection : { start: ds, end: plusDays(d, 1).getTime(), allDay: true };
                    showMenu(e.clientX, e.clientY, slotMenu(sel, settings.timeScale));
                  }}
                >
                  <div className="cm-cal-mcell-head">
                    <span className="cm-cal-mcell-date" onMouseDown={(e) => e.stopPropagation()} onClick={() => useCal.getState().goTo(ds, "day")} title="Go to this day">
                      {d.getDate() === 1 || (w === 0 && i === 0) ? format(d, DAY_FIRST ? "d MMM" : "MMM d") : format(d, "d")}
                    </span>
                  </div>
                  <div className="cm-cal-mcell-items" style={{ paddingTop: Math.min(lanes, capacity) * LINE }}>
                    {shown.map((ev) =>
                      monthDetail === "medium" ? (
                        <div
                          key={ev.id}
                          className={`cm-cal-mbar${ev.id === selectedId ? " selected" : ""}`}
                          style={{ background: stripeBackground(ev.busyStatus, colorOf(ev)) }}
                          title={itemTitle(ev)}
                          onMouseDown={(e) => onItemDown(e, ev)}
                          onDoubleClick={(e) => {
                            e.stopPropagation();
                            openItem(ev);
                          }}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            set({ selectedId: ev.id, selection: null });
                            showMenu(e.clientX, e.clientY, itemMenu(ev));
                          }}
                        />
                      ) : (
                        <div
                          key={ev.id}
                          className={`cm-cal-mitem${ev.id === selectedId ? " selected" : ""}`}
                          style={colorOf(ev) !== ev.calendarColor ? { background: tint(colorOf(ev), 0.45) } : undefined}
                          title={itemTitle(ev)}
                          onMouseDown={(e) => onItemDown(e, ev)}
                          onDoubleClick={(e) => {
                            e.stopPropagation();
                            openItem(ev);
                          }}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            set({ selectedId: ev.id, selection: null });
                            showMenu(e.clientX, e.clientY, itemMenu(ev));
                          }}
                        >
                          <span className="cm-cal-mitem-dot" style={{ background: stripeBackground(ev.busyStatus, colorOf(ev)) }} />
                          <span className="cm-cal-mitem-time">{ev.start < ds ? "" : fmtShortTime(ev.start)}</span>
                          {ev.isRecurring && <Icon name="recurring" size={11} />}
                          {ev.isPrivate && <Icon name="private" size={11} />}
                          <span className="cm-cal-mitem-subj">{ev.summary || "(No subject)"}</span>
                        </div>
                      ),
                    )}
                  </div>
                  {hidden > 0 && (
                    <button
                      className="cm-cal-more"
                      title={`${hidden} more item${hidden === 1 ? "" : "s"} — click to see this day`}
                      onMouseDown={(e) => e.stopPropagation()}
                      onClick={() => useCal.getState().goTo(ds, "day")}
                    >
                      <Icon name="chevron-down" size={10} />
                    </button>
                  )}
                </div>
              );
            })}
            {/* the new event being typed, across the selected days of the week it starts in */}
            {inline &&
              (() => {
                const first = row.wd.findIndex((d) => sameDay(d, inline.start));
                if (first < 0) return null;
                let last = first;
                while (last < 6 && plusDays(row.wd[last], 1).getTime() < inline.end) last++;
                const lanes = Math.max(...row.lanesPerDay.slice(first, last + 1));
                return (
                  <input
                    className="cm-cal-inline month"
                    style={{ top: HEAD + lanes * LINE, left: `calc(${(first / 7) * 100}% + 2px)`, width: `calc(${((last - first + 1) / 7) * 100}% - 5px)` }}
                    autoFocus
                    value={inline.text}
                    onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
                    onChange={(e) => setInline({ ...inline, text: e.target.value })}
                    onBlur={commitInline}
                    onMouseDown={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Enter") commitInline();
                      else if (e.key === "Escape") setInline(null);
                    }}
                  />
                );
              })()}
            {/* multi-day / all-day bars across the week */}
            {row.laid
              .filter((x) => x.lane < capacity)
              .map(({ item, lane }) => {
                const ev = item.ev;
                const c = apptColors(colorOf(ev));
                return (
                  <div
                    key={ev.id}
                    className={`cm-cal-mband${ev.id === selectedId ? " selected" : ""}`}
                    style={{
                      top: HEAD + lane * LINE,
                      left: `calc(${(item.first / 7) * 100}% + 2px)`,
                      width: `calc(${((item.last - item.first + 1) / 7) * 100}% - 5px)`,
                      background: `linear-gradient(${c.fillTop}, ${c.fillBottom})`,
                      borderColor: ev.id === selectedId ? undefined : c.border,
                    }}
                    title={itemTitle(ev)}
                    onMouseDown={(e) => onItemDown(e, ev)}
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      openItem(ev);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      set({ selectedId: ev.id, selection: null });
                      showMenu(e.clientX, e.clientY, itemMenu(ev));
                    }}
                  >
                    {ev.start < row.wd[item.first].getTime() && "◂ "}
                    {ev.isRecurring && <Icon name="recurring" size={11} />}
                    {!ev.allDay && <span className="cm-cal-mitem-time">{fmtShortTime(ev.start)} </span>}
                    <span className="cm-cal-mitem-subj">{ev.summary || "(No subject)"}</span>
                    {ev.location && <span className="cm-muted"> ({ev.location})</span>}
                    {ev.end > plusDays(row.wd[item.last], 1).getTime() && " ▸"}
                  </div>
                );
              })}
          </div>
        ))}
      </div>
    </div>
  );
}
