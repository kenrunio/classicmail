// Outlook 2007 Day / Work Week / Week view: day headers, all-day band, 24-hour time grid with
// work hours shaded, overlapping appointments side by side, click+drag selection, type-to-create,
// drag to move (Ctrl = copy), drag the top/bottom edge to resize, now line.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { addDays, format } from "date-fns";
import { cal, type CalEvent, type CalendarInfo } from "../../api/calendar";
import { errorBox, Icon, showMenu } from "../../ui";
import { atMinutes, DAY, dayStart, fmtLongDate, wallMinutes, MIN, minutesOf, plusDays, sameDay, uses12h, dayKey } from "./dates";
import { gridColumns, groupTarget, isBandItem, layoutDay, layoutLanes, spanInGroup } from "./layout";
import { apptColors, stripeBackground, tint } from "./color";
import { itemColors, useCategories } from "./categories";
import { calLabel } from "./NavPane";
import { itemMenu, slotMenu } from "./menus";
import { askSendUpdate, openItem } from "./actions";
import { newFromSelection } from "./menus";
import { useCal, type CalSettings, type Selection } from "./state";

const SLOT_H = 21;
const LANE_H = 20;
const RULER_W = 52;

type Override = { start: number; end: number; allDay: boolean; startDate?: string; endDate?: string };

/** One side-by-side column group: its calendars and their (de-duplicated) items. */
export interface GridGroup {
  cals: CalendarInfo[];
  events: CalEvent[];
}

type Drag =
  | { kind: "select"; anchor: number; allDay: false; group: number }
  | { kind: "selectDays"; anchorDay: number }
  | { kind: "move"; ev: CalEvent; offset: number; x0: number; y0: number; moved: boolean; band: boolean; dayIdx0: number }
  | { kind: "resize"; ev: CalEvent; edge: "start" | "end"; moved: boolean };

export function DayWeekGrid({
  days,
  groups,
  settings,
  showTabs,
  allCalendars,
  onOverlay,
}: {
  days: Date[];
  groups: GridGroup[];
  settings: CalSettings;
  /** side by side: coloured calendar tabs above the day headers */
  showTabs: boolean;
  allCalendars: CalendarInfo[];
  /** calendar id → view in overlay mode (true) / side by side again (false) */
  onOverlay: (calendarId: number, overlay: boolean) => void;
}) {
  const { selection, selectedId, set } = useCal();
  const cats = useCategories();
  const scale = settings.timeScale;
  const pxPerMin = SLOT_H / scale;
  const nd = days.length;
  const G = Math.max(1, groups.length);
  const cols = useMemo(() => gridColumns(nd, G), [nd, G]);
  const n = cols.length;
  const colDay = (i: number) => days[cols[i].day];
  /** the calendar new items in group g go to (also marks which group holds the selection) */
  const groupCal = (g: number) => (G > 1 ? (groupTarget(groups[g]?.cals ?? []) ?? groups[g]?.cals[0])?.id : undefined);
  const selInGroup = (g: number) => G === 1 || selection?.calendarId === groupCal(g);
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const colsRef = useRef<HTMLDivElement>(null);
  const bandRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<Drag | null>(null);
  const [ghost, setGhost] = useState<{ ev: CalEvent; start: number; end: number; copy: boolean; group: number } | null>(null);
  const [overrides, setOverrides] = useState<Record<number, Override>>({});
  const [inline, setInline0] = useState<(Selection & { text: string }) | null>(null);
  // the latest inline-editor state; cleared on commit so Enter + the following blur save only once
  const inlineRef = useRef<(Selection & { text: string }) | null>(null);
  const setInline = (v: (Selection & { text: string }) | null) => {
    inlineRef.current = v;
    setInline0(v);
  };
  const [now, setNow] = useState(Date.now());
  const [width, setWidth] = useState(800);

  useEffect(() => setOverrides({}), [groups]);
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(t);
  }, []);
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // scroll to the start of the working day when the range or scale changes
  const rangeKey = `${days[0]?.getTime()}-${n}-${scale}`;
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const top = Math.max(0, settings.workDayStart * pxPerMin - 2);
    if (el) el.scrollTop = top;
    // the pane may still be settling its height on first show: apply again once laid out
    const raf = requestAnimationFrame(() => el && (el.scrollTop = top));
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rangeKey]);

  const evsByGroup = useMemo(() => groups.map((g) => g.events.map((e) => (overrides[e.id] ? { ...e, ...overrides[e.id] } : e))), [groups, overrides]);
  const evs = useMemo(() => evsByGroup.flat(), [evsByGroup]);
  // category colours (subject matching runs regexes): once per data change, not on every mouse move
  const colorsById = useMemo(() => new Map(evs.map((e) => [e.id, itemColors(e, cats)])), [evs, cats]);
  const band = useMemo(() => {
    const items = evsByGroup.flatMap((list, g) =>
      list
        .filter(isBandItem)
        .map((e) => {
          let first = -1;
          let last = -1;
          days.forEach((d, i) => {
            const ds = d.getTime();
            const de = plusDays(d, 1).getTime();
            if (e.start < de && e.end > ds) {
              if (first < 0) first = i;
              last = i;
            }
          });
          return { id: e.id, ...spanInGroup(first, last, g, nd), ok: first >= 0, ev: e, dayFirst: first, dayLast: last };
        })
        .filter((x) => x.ok),
    );
    const laid = layoutLanes(items);
    const lanes = laid.reduce((m, x) => Math.max(m, x.lane + 1), 0);
    return { laid, lanes };
  }, [evsByGroup, days, nd]);

  const columns = useMemo(
    () =>
      cols.map((c) => {
        const d = days[c.day];
        const ds = d.getTime();
        const de = plusDays(d, 1).getTime();
        const items = (evsByGroup[c.group] ?? [])
          .filter((e) => !isBandItem(e) && e.start < de && (e.end > ds || (e.end === e.start && e.start >= ds)))
          .map((e) => ({ id: e.id, ev: e, startMin: wallMinutes(e.start, d), endMin: wallMinutes(e.end, d) }));
        return layoutDay(items, scale);
      }),
    [evsByGroup, days, cols, scale],
  );

  // ---- geometry -----------------------------------------------------------------------------
  const pointAt = (cx: number, cy: number, exact = false) => {
    const r = colsRef.current!.getBoundingClientRect();
    const i = Math.min(n - 1, Math.max(0, Math.floor(((cx - r.left) / r.width) * n)));
    let m = Math.min(1440 - scale, Math.max(0, (cy - r.top) / pxPerMin));
    if (!exact) m = Math.floor(m / scale) * scale;
    // wall-clock time, so DST days (23 / 25 hours) still line up with the ruler
    return { i, g: cols[i].group, m, ms: atMinutes(colDay(i), Math.floor(m)) + (m - Math.floor(m)) * MIN };
  };
  const bandDayAt = (cx: number) => {
    const el = bandRef.current!;
    const r = el.getBoundingClientRect();
    // clientWidth: the day cells' width, without the reserved scrollbar slot
    return Math.min(n - 1, Math.max(0, Math.floor(((cx - r.left) / Math.max(1, el.clientWidth)) * n)));
  };
  const snap = (ms: number) => {
    const d = dayStart(ms);
    const m = Math.round(wallMinutes(ms, d) / scale) * scale;
    return atMinutes(d, m);
  };

  // ---- drag handling ------------------------------------------------------------------------
  const startDrag = (d: Drag, e: React.MouseEvent) => {
    dragRef.current = d;
    const move = (ev: MouseEvent) => onDragMove(ev);
    const up = (ev: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      onDragEnd(ev);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    e.preventDefault();
    rootRef.current?.focus({ preventScroll: true });
  };

  const onDragMove = (e: MouseEvent) => {
    const d = dragRef.current;
    if (!d) return;
    if (d.kind === "select") {
      const p = pointAt(e.clientX, e.clientY);
      // a time selection stays inside its calendar's columns
      const ms = p.g === d.group ? p.ms : atMinutes(days[cols[p.i].day], Math.floor(p.m));
      const a = Math.min(d.anchor, ms);
      const b = Math.max(d.anchor, ms) + scale * MIN;
      set({ selection: { start: a, end: b, allDay: false, calendarId: groupCal(d.group) } });
    } else if (d.kind === "selectDays") {
      const i0 = d.anchorDay;
      const g = cols[i0].group;
      const i = bandDayAt(e.clientX);
      const di = cols[i].group === g ? cols[i].day : i < i0 ? 0 : nd - 1;
      const a = Math.min(cols[i0].day, di);
      const b = Math.max(cols[i0].day, di);
      set({ selection: { start: days[a].getTime(), end: plusDays(days[b], 1).getTime(), allDay: true, calendarId: groupCal(g) } });
    } else if (d.kind === "move") {
      if (!d.moved && Math.abs(e.clientX - d.x0) + Math.abs(e.clientY - d.y0) < 5) return;
      d.moved = true;
      const dur = d.ev.end - d.ev.start;
      let start: number;
      if (d.band) {
        const bi = bandDayAt(e.clientX);
        const delta = cols[bi].day - cols[d.dayIdx0].day;
        start = addDays(d.ev.start, delta).getTime();
        const days0 = Math.round(dur / DAY);
        setGhost({ ev: d.ev, start, end: d.ev.allDay ? plusDays(start, Math.max(1, days0)).getTime() : start + dur, copy: e.ctrlKey, group: cols[bi].group });
        return;
      }
      const p = pointAt(e.clientX, e.clientY, true);
      start = snap(p.ms - d.offset);
      setGhost({ ev: d.ev, start, end: start + dur, copy: e.ctrlKey, group: p.g });
    } else if (d.kind === "resize") {
      d.moved = true;
      const p = pointAt(e.clientX, e.clientY);
      const g = groupOf(d.ev);
      if (d.edge === "end") {
        const end = Math.max(d.ev.start + scale * MIN, p.ms + scale * MIN);
        setGhost({ ev: d.ev, start: d.ev.start, end, copy: false, group: g });
      } else {
        const start = Math.min(d.ev.end - scale * MIN, p.ms);
        setGhost({ ev: d.ev, start, end: d.ev.end, copy: false, group: g });
      }
    }
  };

  const groupOf = (ev: CalEvent) =>
    Math.max(
      0,
      groups.findIndex((g) => g.events.some((x) => x.id === ev.id)),
    );

  const onDragEnd = async (e: MouseEvent) => {
    const d = dragRef.current;
    dragRef.current = null;
    setGhost(null);
    if (!d || (d.kind !== "move" && d.kind !== "resize") || !d.moved) return;
    // recompute the final position from the event (state may lag a frame behind)
    let start: number | undefined;
    let end: number | undefined;
    let toGroup = groupOf(d.ev);
    if (d.kind === "move" && !d.band) {
      const p = pointAt(e.clientX, e.clientY, true);
      start = snap(p.ms - d.offset);
      end = start + (d.ev.end - d.ev.start);
      toGroup = p.g;
    } else if (d.kind === "move" && d.band) {
      const bi = bandDayAt(e.clientX);
      const delta = cols[bi].day - cols[d.dayIdx0].day;
      start = addDays(d.ev.start, delta).getTime();
      end = addDays(d.ev.end, delta).getTime();
      toGroup = cols[bi].group;
    } else if (d.kind === "resize") {
      const p = pointAt(e.clientX, e.clientY);
      if (d.edge === "end") end = Math.max(d.ev.start + scale * MIN, p.ms + scale * MIN);
      else start = Math.min(d.ev.end - scale * MIN, p.ms);
      start ??= d.ev.start;
      end ??= d.ev.end;
    }
    // dropped on another calendar's columns: move / copy into that calendar
    const destCals = groups[toGroup]?.cals ?? [];
    const inDest = destCals.some((c) => c.id === d.ev.calendarId);
    const dest = inDest ? undefined : groupTarget(destCals);
    if (G > 1 && !inDest && !dest) {
      errorBox("You do not have permission to create items in this calendar.");
      return;
    }
    if (start === undefined || end === undefined || (start === d.ev.start && end === d.ev.end && !dest)) return;
    const ev = d.ev;
    const copy = d.kind === "move" && e.ctrlKey;
    const what = `"${ev.summary || "Untitled"}"`;
    const verb = copy ? "Copy" : d.kind === "resize" ? "Resize" : "Move";
    if (dest) {
      try {
        if (copy) {
          const c = await cal.copy(ev.id, start, end, dest.id, `Copy ${what} to ${dest.name}`);
          set({ selectedId: c.id });
        } else {
          const r = await cal.moveTo(
            ev.id,
            dest.id,
            ev.allDay ? { allDay: true, startDate: dayKey(start), endDate: dayKey(end) } : { start, end, sendUpdates: false },
            `Move ${what} to ${dest.name}`,
          );
          set({ selectedId: r.id });
        }
      } catch (err) {
        errorBox(err);
      }
      return;
    }
    let send: boolean | null = false;
    if (!copy && ev.isMeeting && ev.isOrganizer) {
      // show the item where it was dropped while asking
      setOverrides((o) => ({
        ...o,
        [ev.id]: ev.allDay ? { start: start!, end: end!, allDay: true, startDate: dayKey(start!), endDate: dayKey(end!) } : { start: start!, end: end!, allDay: false },
      }));
      send = await askSendUpdate(ev);
      if (send === null) {
        setOverrides((o) => {
          const c = { ...o };
          delete c[ev.id];
          return c;
        });
        return;
      }
    }
    try {
      if (copy) {
        const c = await cal.copy(ev.id, start, end, undefined, `${verb} ${what}`);
        set({ selectedId: c.id });
      } else if (ev.allDay) {
        const sd = dayKey(start);
        const ed = dayKey(end);
        setOverrides((o) => ({ ...o, [ev.id]: { start, end, allDay: true, startDate: sd, endDate: ed } }));
        await cal.update(ev.id, { allDay: true, startDate: sd, endDate: ed, sendUpdates: send }, "occurrence", `${verb} ${what}`);
      } else {
        setOverrides((o) => ({ ...o, [ev.id]: { start, end, allDay: false } }));
        await cal.update(ev.id, { start, end, sendUpdates: send }, "occurrence", `${verb} ${what}`);
      }
    } catch (err) {
      setOverrides((o) => {
        const c = { ...o };
        delete c[ev.id];
        return c;
      });
      errorBox(err);
    }
  };

  const canDrag = (ev: CalEvent) => !ev.readOnly && !(ev.isMeeting && !ev.isOrganizer);

  // ---- inline create ------------------------------------------------------------------------
  const commitInline = async () => {
    const it = inlineRef.current;
    inlineRef.current = null;
    setInline(null);
    rootRef.current?.focus({ preventScroll: true });
    if (!it || !it.text.trim()) return;
    try {
      const d = await cal.create(
        it.allDay
          ? { summary: it.text.trim(), allDay: true, startDate: dayKey(it.start), endDate: dayKey(it.end), reminderMinutes: -1, busyStatus: "free", calendarId: it.calendarId }
          : { summary: it.text.trim(), start: it.start, end: it.end, reminderMinutes: settings.defaultReminderMinutes, calendarId: it.calendarId },
        `New Appointment "${it.text.trim()}"`,
      );
      set({ selectedId: d.id, selection: null });
    } catch (e) {
      errorBox(e);
    }
  };

  // ---- keyboard -----------------------------------------------------------------------------
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (inline) return;
    const k = e.key;
    if (k.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey && selection && selectedId === null) {
      e.preventDefault();
      setInline({ ...selection, text: k });
      return;
    }
    const order = [...evs].sort((a, b) => a.start - b.start || a.id - b.id);
    if (k === "Tab") {
      e.preventDefault();
      if (!order.length) return;
      const idx = order.findIndex((x) => x.id === selectedId);
      const next = e.shiftKey ? (idx <= 0 ? order.length - 1 : idx - 1) : (idx + 1) % order.length;
      set({ selectedId: order[next].id, selection: null });
      return;
    }
    if (k === "Enter") {
      e.preventDefault();
      const ev = evs.find((x) => x.id === selectedId);
      if (ev) openItem(ev);
      else if (selection) newFromSelection(selection);
      return;
    }
    if (k === "Escape") {
      set({ selection: null, selectedId: null });
      return;
    }
    if (k.startsWith("Arrow")) {
      e.preventDefault();
      const base = selection ?? { start: atMinutes(days[0], settings.workDayStart), end: atMinutes(days[0], settings.workDayStart) + scale * MIN, allDay: false };
      let { start, end } = base;
      const step = scale * MIN;
      if (k === "ArrowDown") {
        if (e.shiftKey) end += step;
        else {
          start = end;
          end = start + step;
        }
      } else if (k === "ArrowUp") {
        if (e.shiftKey) end = Math.max(start + step, end - step);
        else {
          start -= step;
          end = start + step;
        }
      } else {
        const dir = k === "ArrowRight" ? 1 : -1;
        // same clock time / whole day on the next day (not ± 24 h: DST days have 23 or 25 hours)
        start = atMinutes(plusDays(start, dir), minutesOf(start));
        end = base.allDay ? plusDays(start, 1).getTime() : start + step;
        const idx = days.findIndex((d) => sameDay(d, start));
        if (idx < 0) useCal.getState().set({ anchor: dayStart(start).getTime() });
      }
      set({ selection: { start, end, allDay: base.allDay, calendarId: base.calendarId }, selectedId: null });
      // keep the selection visible
      const sc = scrollRef.current;
      if (sc && !base.allDay) {
        const top = minutesOf(start) * pxPerMin;
        if (top < sc.scrollTop) sc.scrollTop = top;
        else if (top + SLOT_H > sc.scrollTop + sc.clientHeight) sc.scrollTop = top + SLOT_H - sc.clientHeight;
      }
    }
  };

  // ---- rendering ----------------------------------------------------------------------------
  const colW = (width - RULER_W - 16) / Math.max(1, n);
  const groupStart = (i: number) => cols[i].day === 0 && cols[i].group > 0;
  const today = dayStart(now).getTime();
  const bandLanes = Math.max(1, band.lanes + (inline?.allDay ? 1 : 0));
  const hourCycle = uses12h;

  const renderAppt = (ev: CalEvent, style: React.CSSProperties, opts: { band?: boolean; dayIdx: number; clipL?: boolean; clipR?: boolean }) => {
    const colors = colorsById.get(ev.id) ?? itemColors(ev, cats);
    const color = colors[0];
    const c = apptColors(color);
    // several categories: the colour bar is split into one band per category (busy items)
    const stripe =
      colors.length > 1 && (ev.busyStatus === "busy" || !ev.busyStatus)
        ? `linear-gradient(${colors.map((h, i) => `${apptColors(h).stripe} ${(i * 100) / colors.length}% ${((i + 1) * 100) / colors.length}%`).join(", ")})`
        : stripeBackground(ev.busyStatus, color);
    const selected = ev.id === selectedId;
    const short = !opts.band && typeof style.height === "number" && style.height < SLOT_H * 1.5;
    const dragging = ghost?.ev.id === ev.id && !ghost.copy;
    return (
      <div
        key={`${ev.id}-${opts.dayIdx}`}
        className={`cm-cal-appt${selected ? " selected" : ""}${opts.band ? " band" : ""}${dragging ? " dragging" : ""}${ev.myResponse === "needsAction" ? " unanswered" : ""}`}
        style={{ ...style, background: `linear-gradient(${c.fillTop}, ${c.fillBottom})`, borderColor: selected ? undefined : c.border }}
        title={`${ev.summary}${ev.location ? `\n${ev.location}` : ""}\n${ev.allDay ? "All day" : `${format(ev.start, hourCycle ? "h:mm a" : "HH:mm")} - ${format(ev.end, hourCycle ? "h:mm a" : "HH:mm")}`}${ev.categories?.length ? `\nCategories: ${ev.categories.join(", ")}` : ""}${ev.alsoIn?.length ? `\nAlso in: ${ev.alsoIn.join("; ")}` : ""}`}
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          e.stopPropagation();
          set({ selectedId: ev.id, selection: null });
          if (!canDrag(ev)) {
            rootRef.current?.focus({ preventScroll: true });
            return;
          }
          const target = e.target as HTMLElement;
          const edge = target.dataset.edge as "start" | "end" | undefined;
          if (edge && !opts.band) {
            startDrag({ kind: "resize", ev, edge, moved: false }, e);
            return;
          }
          if (opts.band) {
            startDrag({ kind: "move", ev, offset: 0, x0: e.clientX, y0: e.clientY, moved: false, band: true, dayIdx0: bandDayAt(e.clientX) }, e);
          } else {
            const p = pointAt(e.clientX, e.clientY, true);
            startDrag({ kind: "move", ev, offset: p.ms - ev.start, x0: e.clientX, y0: e.clientY, moved: false, band: false, dayIdx0: p.i }, e);
          }
        }}
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
        <div className="cm-cal-appt-stripe" style={{ background: stripe }} />
        {colors.length > 2 && (
          <div className="cm-cal-appt-cats" aria-hidden>
            {colors.map((h) => (
              <span key={h} style={{ background: h }} />
            ))}
          </div>
        )}
        <div className={`cm-cal-appt-text${short ? " short" : ""}`}>
          {opts.clipL && <span className="cm-cal-clip">◂ </span>}
          {ev.isRecurring && <Icon name="recurring" size={12} />}
          {ev.isMeeting && <Icon name="meeting" size={12} />}
          {ev.isPrivate && <Icon name="private" size={12} />}
          {ev.importance === "high" && <Icon name="importance-high" size={12} />}
          {opts.band && !ev.allDay && <span className="cm-cal-appt-time">{format(ev.start, hourCycle ? "h:mma" : "HH:mm")} </span>}
          <span className="cm-cal-appt-subject">{ev.summary || "(No subject)"}</span>
          {ev.location && <span className="cm-cal-appt-loc"> ({ev.location})</span>}
          {opts.clipR && <span className="cm-cal-clip"> ▸</span>}
        </div>
        {!opts.band && canDrag(ev) && (
          <>
            <div className="cm-cal-appt-edge top" data-edge="start" />
            <div className="cm-cal-appt-edge bottom" data-edge="end" />
          </>
        )}
      </div>
    );
  };

  return (
    <div className="cm-cal-dw" ref={rootRef} tabIndex={0} onKeyDown={onKeyDown}>
      {/* side by side: one coloured tab per calendar (overlaid calendars share their group) */}
      {showTabs && (
        <div className="cm-cal-dw-tabs" style={{ gridTemplateColumns: `${RULER_W}px repeat(${G}, 1fr) 16px` }}>
          <div className="cm-cal-corner" />
          {groups.map((g, gi) => (
            <div key={gi} className={`cm-cal-grouptabs${gi > 0 ? " group-start" : ""}`}>
              {g.cals.map((c, ci) => {
                // the first calendar of a later group can be overlaid on the first group;
                // an overlaid calendar (not the base one) can go back to side by side
                const canOverlay = gi > 0 && ci === 0;
                const canSplit = gi === 0 && ci > 0;
                return (
                  <div
                    key={c.id}
                    className={`cm-cal-caltab${ci > 0 ? " stacked" : ""}`}
                    style={{ background: `linear-gradient(${tint(c.color, 0.35)}, ${tint(c.color, 0.05)})`, borderColor: c.color }}
                    title={`${c.name} (${c.accountEmail})`}
                  >
                    {canSplit && (
                      <button className="cm-cal-tabarrow" title="View in Side-By-Side Mode" onClick={() => onOverlay(c.id, false)}>
                        ▸
                      </button>
                    )}
                    {canOverlay && (
                      <button className="cm-cal-tabarrow" title="View in Overlay Mode" onClick={() => onOverlay(c.id, true)}>
                        ◂
                      </button>
                    )}
                    <span className="cm-ellipsis">{calLabel(c, allCalendars)}</span>
                  </div>
                );
              })}
            </div>
          ))}
          <div className="cm-cal-corner" />
        </div>
      )}
      {/* day headers */}
      <div className="cm-cal-dw-head" style={{ gridTemplateColumns: `${RULER_W}px repeat(${n}, 1fr) 16px` }}>
        <div className="cm-cal-corner" />
        {cols.map((c, ci) => {
          const d = days[c.day];
          const isToday = d.getTime() === today;
          const label = colW > 120 ? format(d, "d EEEE") : colW > 60 ? format(d, "d EEE") : colW > 34 ? format(d, "EEEEE d") : format(d, "d");
          return (
            <div
              key={ci}
              className={`cm-cal-dayhead${isToday ? " today" : ""}${groupStart(ci) ? " group-start" : ""}`}
              title={fmtLongDate(d)}
              onClick={() => useCal.getState().goTo(d.getTime(), "day")}
            >
              {label}
            </div>
          );
        })}
        <div className="cm-cal-corner" />
      </div>
      {/* all-day band */}
      <div className="cm-cal-dw-band" style={{ gridTemplateColumns: `${RULER_W}px 1fr 16px` }}>
        <div className="cm-cal-corner" />
        <div
          className="cm-cal-band"
          ref={bandRef}
          // spans the scrollbar slot too, which is always reserved (scrollbar-gutter), so the day
          // cells are exactly as wide as the header and grid columns whether or not it scrolls
          style={{ height: Math.min(5, bandLanes) * LANE_H + 6, overflowY: bandLanes > 5 ? "auto" : "hidden", gridColumn: "2 / 4", scrollbarGutter: "stable" }}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            const i = bandDayAt(e.clientX);
            const d = colDay(i);
            set({ selection: { start: d.getTime(), end: plusDays(d, 1).getTime(), allDay: true, calendarId: groupCal(cols[i].group) }, selectedId: null });
            startDrag({ kind: "selectDays", anchorDay: i }, e);
          }}
          onDoubleClick={() => newFromSelection(useCal.getState().selection, { allDay: true })}
          onContextMenu={(e) => {
            e.preventDefault();
            const i = bandDayAt(e.clientX);
            const d = colDay(i);
            const sel =
              selection?.allDay && selInGroup(cols[i].group) && selection.start <= d.getTime() && selection.end > d.getTime()
                ? selection
                : { start: d.getTime(), end: plusDays(d, 1).getTime(), allDay: true, calendarId: groupCal(cols[i].group) };
            set({ selection: sel, selectedId: null });
            showMenu(e.clientX, e.clientY, slotMenu(sel, scale));
          }}
        >
          {cols.map((c, i) => {
            const d = days[c.day];
            const sel = selection?.allDay && selInGroup(c.group) && selection.start < plusDays(d, 1).getTime() && selection.end > d.getTime();
            return (
              <div
                key={i}
                className={`cm-cal-band-cell${sel ? " sel" : ""}${d.getTime() === today ? " today" : ""}${groupStart(i) ? " group-start" : ""}`}
                style={{ left: `${(i / n) * 100}%`, width: `${100 / n}%` }}
              />
            );
          })}
          {band.laid.map(({ item, lane }) =>
            renderAppt(
              item.ev,
              {
                position: "absolute",
                top: 3 + lane * LANE_H,
                height: LANE_H - 2,
                left: `calc(${(item.first / n) * 100}% + 2px)`,
                width: `calc(${((item.last - item.first + 1) / n) * 100}% - 5px)`,
              },
              { band: true, dayIdx: item.first, clipL: item.ev.start < days[item.dayFirst].getTime(), clipR: item.ev.end > plusDays(days[item.dayLast], 1).getTime() },
            ),
          )}
          {ghost &&
            isBandItem(ghost.ev) &&
            (() => {
              let fi = days.findIndex((d) => plusDays(d, 1).getTime() > ghost.start);
              let li = -1;
              days.forEach((d, i) => d.getTime() < ghost.end && (li = i));
              if (fi < 0 || li < fi) return null;
              fi += ghost.group * nd;
              li += ghost.group * nd;
              return (
                <div className="cm-cal-ghost" style={{ position: "absolute", top: 3, height: LANE_H - 2, left: `${(fi / n) * 100}%`, width: `${((li - fi + 1) / n) * 100}%` }} />
              );
            })()}
          {inline?.allDay &&
            (() => {
              const g = Math.max(
                0,
                groups.findIndex((_, gi) => G === 1 || groupCal(gi) === inline.calendarId),
              );
              const first = Math.max(
                0,
                days.findIndex((d) => sameDay(d, inline.start)),
              );
              // as wide as the days selected (a conference's three days), within this view
              let last = first;
              while (last < nd - 1 && plusDays(days[last], 1).getTime() < inline.end) last++;
              const i = g * nd + first;
              return (
                <textarea
                  className="cm-cal-inline"
                  style={{ top: 3 + band.lanes * LANE_H, height: LANE_H - 2, left: `${(i / n) * 100}%`, width: `${((last - first + 1) / n) * 100}%` }}
                  autoFocus
                  value={inline.text}
                  onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
                  onChange={(e) => setInline({ ...inline, text: e.target.value })}
                  onBlur={commitInline}
                  onMouseDown={(e) => e.stopPropagation()}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitInline();
                    } else if (e.key === "Escape") setInline(null);
                  }}
                />
              );
            })()}
        </div>
      </div>
      {/* time grid */}
      <div className="cm-cal-dw-scroll" ref={scrollRef}>
        <div className="cm-cal-dw-body" style={{ height: 1440 * pxPerMin }}>
          <div className="cm-cal-ruler" style={{ width: RULER_W }}>
            {Array.from({ length: 24 }, (_, h) => (
              <div key={h} className="cm-cal-hour" style={{ top: h * 60 * pxPerMin, height: 60 * pxPerMin }}>
                <span className="cm-cal-hour-n">{hourCycle ? (h % 12 === 0 ? 12 : h % 12) : String(h).padStart(2, "0")}</span>
                <span className="cm-cal-hour-s">{hourCycle ? (h === 0 || h === 12 ? (h === 0 ? "am" : "pm") : h < 12 ? "am" : "pm") : "00"}</span>
              </div>
            ))}
          </div>
          <div
            className="cm-cal-cols"
            ref={colsRef}
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              const p = pointAt(e.clientX, e.clientY);
              const calendarId = groupCal(p.g);
              if (e.shiftKey && selection && !selection.allDay && selInGroup(p.g)) {
                set({ selection: { start: Math.min(selection.start, p.ms), end: Math.max(selection.end, p.ms + scale * MIN), allDay: false, calendarId }, selectedId: null });
                e.preventDefault();
                return;
              }
              set({ selection: { start: p.ms, end: p.ms + scale * MIN, allDay: false, calendarId }, selectedId: null });
              startDrag({ kind: "select", anchor: p.ms, allDay: false, group: p.g }, e);
            }}
            onDoubleClick={(e) => {
              if ((e.target as HTMLElement).closest(".cm-cal-appt")) return;
              newFromSelection(useCal.getState().selection);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              const p = pointAt(e.clientX, e.clientY);
              const inside = selection && !selection.allDay && selInGroup(p.g) && selection.start <= p.ms && selection.end > p.ms;
              const sel = inside ? selection : { start: p.ms, end: p.ms + scale * MIN, allDay: false, calendarId: groupCal(p.g) };
              set({ selection: sel, selectedId: null });
              showMenu(e.clientX, e.clientY, slotMenu(sel, scale));
            }}
          >
            {cols.map((c, i) => {
              const d = days[c.day];
              const inG = selInGroup(c.group);
              const ds = d.getTime();
              const de = plusDays(d, 1).getTime();
              const work = settings.workDays.includes(d.getDay());
              const sel = selection && inG && !selection.allDay && selection.start < de && selection.end > ds ? selection : null;
              const isToday = ds === today;
              return (
                <div key={i} className={`cm-cal-col${isToday ? " today" : ""}${groupStart(i) ? " group-start" : ""}`} style={{ left: `${(i / n) * 100}%`, width: `${100 / n}%` }}>
                  {work && (
                    <div className="cm-cal-work" style={{ top: settings.workDayStart * pxPerMin, height: Math.max(0, settings.workDayEnd - settings.workDayStart) * pxPerMin }} />
                  )}
                  <div className="cm-cal-lines" style={{ backgroundSize: `100% ${SLOT_H}px, 100% ${60 * pxPerMin}px` }} />
                  {sel && (
                    <div className="cm-cal-sel" style={{ top: wallMinutes(sel.start, d) * pxPerMin, height: (wallMinutes(sel.end, d) - wallMinutes(sel.start, d)) * pxPerMin }} />
                  )}
                  {columns[i].map((p) => {
                    const left = (p.col / p.cols) * 100;
                    const w = (p.span / p.cols) * 100;
                    const top = p.startMin * pxPerMin;
                    const height = Math.max(SLOT_H, (p.endMin - p.startMin) * pxPerMin) - 1;
                    return renderAppt(p.item.ev, { position: "absolute", top, height, left: `calc(${left}% * 0.92 + 1px)`, width: `calc(${w}% * 0.92 - 2px)` }, { dayIdx: i });
                  })}
                  {ghost && !isBandItem(ghost.ev) && ghost.group === c.group && ghost.start < de && ghost.end > ds && (
                    <div
                      className={`cm-cal-ghost${ghost.copy ? " copy" : ""}`}
                      style={{
                        top: wallMinutes(ghost.start, d) * pxPerMin,
                        height: Math.max(SLOT_H, (wallMinutes(ghost.end, d) - wallMinutes(ghost.start, d)) * pxPerMin) - 1,
                      }}
                    >
                      {ghost.copy ? "+" : ""}
                      {format(ghost.start, hourCycle ? "h:mm a" : "HH:mm")} - {format(ghost.end, hourCycle ? "h:mm a" : "HH:mm")}
                    </div>
                  )}
                  {inline && !inline.allDay && (G === 1 || inline.calendarId === groupCal(c.group)) && inline.start >= ds && inline.start < de && (
                    <textarea
                      className="cm-cal-inline"
                      style={{
                        top: wallMinutes(inline.start, d) * pxPerMin,
                        height: Math.max(SLOT_H, (wallMinutes(inline.end, d) - wallMinutes(inline.start, d)) * pxPerMin) - 1,
                        left: 1,
                        width: "92%",
                      }}
                      autoFocus
                      value={inline.text}
                      onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
                      onChange={(e) => setInline({ ...inline, text: e.target.value })}
                      onBlur={commitInline}
                      onMouseDown={(e) => e.stopPropagation()}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") {
                          e.preventDefault();
                          commitInline();
                        } else if (e.key === "Escape") setInline(null);
                      }}
                    />
                  )}
                  {isToday && <div className="cm-cal-now" style={{ top: wallMinutes(now, d) * pxPerMin }} />}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
