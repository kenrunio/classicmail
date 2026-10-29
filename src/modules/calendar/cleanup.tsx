// "Clean Up Calendar…": finds true duplicates inside each selected calendar (same subject, start,
// end, location — e.g. a Gmail reservation created three times) and deletes only the copies the
// user leaves ticked. Keeps one per group (with attendees / notes / oldest), never pre-ticks
// meetings organised by others and warns before deleting them.
import { useEffect, useMemo, useState } from "react";
import { create } from "zustand";
import { format } from "date-fns";
import { cal, type DupGroup } from "../../api/calendar";
import { Dialog, errorBox, confirmBox, Icon } from "../../ui";
import { fmtLongDate, fmtTime, plusDays } from "./dates";

const useDlg = create<{ open: boolean; set: (o: boolean) => void }>((set) => ({ open: false, set: (open) => set({ open }) }));

export function openCleanUp() {
  useDlg.getState().set(true);
}

// one function for the life of the app: the dialog loads once per opening, not on every re-render
const closeCleanUp = () => useDlg.getState().set(false);

export function CleanUpHost() {
  const open = useDlg((s) => s.open);
  return open ? <CleanUpDialog onClose={closeCleanUp} /> : null;
}

function when(g: { start: number; end: number; allDay: boolean }) {
  if (g.allDay) {
    // the last day by the calendar, not end - 24 h (DST days have 23 or 25 hours)
    const last = plusDays(g.end, -1).getTime();
    return last > g.start ? `${format(g.start, "EEE d/M/yyyy")} - ${format(last, "EEE d/M/yyyy")} (all day)` : `${fmtLongDate(g.start)} (all day)`;
  }
  return `${fmtLongDate(g.start)} ${fmtTime(g.start)} - ${fmtTime(g.end)}`;
}

function CleanUpDialog({ onClose }: { onClose: () => void }) {
  const [groups, setGroups] = useState<DupGroup[] | null>(null);
  const [cals, setCals] = useState<Record<string, string>>({});
  const [scanned, setScanned] = useState(0);
  const [ticked, setTicked] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    cal
      .findDuplicates()
      .then((r) => {
        if (!live) return;
        setGroups(r.groups);
        setCals(r.calendars);
        setScanned(r.scanned);
        setTicked(new Set(r.groups.flatMap((g) => g.items.filter((i) => i.suggestDelete).map((i) => i.id))));
      })
      .catch((e) => {
        if (!live) return;
        errorBox(e);
        onClose();
      });
    return () => {
      live = false;
    };
  }, [onClose]);

  const items = useMemo(() => (groups ?? []).flatMap((g) => g.items), [groups]);
  const toggle = (id: number) =>
    setTicked((t) => {
      const n = new Set(t);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const run = async () => {
    const chosen = items.filter((i) => ticked.has(i.id));
    if (!chosen.length) return onClose();
    // a whole group ticked would leave nothing: confirm
    const emptied = (groups ?? []).filter((g) => g.items.every((i) => ticked.has(i.id)));
    if (
      emptied.length &&
      !(await confirmBox(
        `All copies of "${emptied[0].summary}" are ticked, so no copy will be kept. Delete them all?`,
        "Clean Up Calendar",
        "warning",
        "calendar.cleanupAllCopies",
      ))
    )
      return;
    const others = chosen.filter((i) => i.warn);
    if (
      others.length &&
      !(await confirmBox(
        `${others.length} of the ticked items ${others.length === 1 ? "is a meeting" : "are meetings"} organized by someone else (${[...new Set(others.map((o) => o.organizer).filter(Boolean))].join(", ")}). They will be removed from your calendar without sending a response. Continue?`,
        "Clean Up Calendar",
        "warning",
        "calendar.cleanupOthersMeetings",
      ))
    )
      return;
    if (
      !(await confirmBox(
        `Delete ${chosen.length} duplicate item${chosen.length === 1 ? "" : "s"}? The other copies are kept.`,
        "Clean Up Calendar",
        "help",
        "calendar.cleanupDelete",
      ))
    )
      return;
    setBusy(true);
    try {
      const r = await cal.deleteEvents(chosen.map((i) => i.id));
      onClose();
      if (r.failed.length) errorBox(`${r.deleted} deleted; ${r.failed.length} could not be deleted:\n${r.failed.join("\n")}`);
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title="Clean Up Calendar"
      width={640}
      onClose={onClose}
      footer={
        <>
          <span className="cm-fill cm-muted">{groups ? `${ticked.size} of ${items.length} items ticked for deletion` : ""}</span>
          <button className="cm-button default" onClick={run} disabled={busy || !groups || ticked.size === 0}>
            Delete Ticked
          </button>
          <button className="cm-button" onClick={onClose}>
            Cancel
          </button>
        </>
      }
    >
      <div className="cm-cal-cleanup">
        {!groups ? (
          <div className="cm-muted">Looking for duplicate items…</div>
        ) : groups.length === 0 ? (
          <div className="cm-cal-dlg">
            <Icon name="info" size={32} />
            <div>No duplicates were found in the selected calendars ({scanned.toLocaleString()} items checked).</div>
          </div>
        ) : (
          <>
            <div className="cm-cal-cleanup-hint">
              These items appear more than once in the same calendar (same subject, time and location). The ticked copies will be deleted; one copy of each is kept — the one with
              attendees or notes, else the oldest. Untick anything you want to keep.
            </div>
            <div className="cm-cal-cleanup-list">
              {groups.map((g, gi) => (
                <div key={gi} className="cm-cal-dupgroup">
                  <div className="cm-cal-dupgroup-head">
                    <b className="cm-ellipsis">{g.summary}</b>
                    <span className="cm-muted">
                      {when({ ...g, allDay: g.items[0]?.allDay ?? false })} · {cals[String(g.calendarId)] ?? ""} · {g.items.length} copies
                    </span>
                  </div>
                  {g.items.map((it, k) => (
                    <label key={it.id} className={`cm-cal-dupitem${ticked.has(it.id) ? " del" : ""}`}>
                      <input type="checkbox" checked={ticked.has(it.id)} disabled={it.readOnly} onChange={() => toggle(it.id)} />
                      <span className="cm-ellipsis cm-fill">
                        Copy {k + 1}
                        {it.isSeries ? " (recurring series)" : ""}
                        {it.location ? ` — ${it.location}` : ""}
                      </span>
                      <span className="cm-cal-dupflags">
                        {it.hasAttendees && <span title="Has attendees">attendees</span>}
                        {it.hasNotes && <span title="Has notes">notes</span>}
                        {it.created ? <span title="Created">{format(it.created, "d/M/yyyy HH:mm")}</span> : null}
                        {it.warn && (
                          <span className="warn" title={`Organized by ${it.organizer ?? "someone else"}`}>
                            organized by {it.organizer ?? "someone else"}
                          </span>
                        )}
                        {it.readOnly && <span title="Read-only">read-only</span>}
                        {!ticked.has(it.id) && <span className="keep">keep</span>}
                      </span>
                    </label>
                  ))}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </Dialog>
  );
}
