// A meeting's Responses (Outlook's Tracking): each attendee and their answer, and Resend Invitation
// for those who haven't answered — a meeting request e-mail to just them (calendar/invite.rs).
import { useState } from "react";
import { cal, type AttendeeInfo, type EventDetail } from "../../../api/calendar";
import { Dialog, errorBox, Icon } from "../../../ui";
import { describeRecurrence } from "../recurrence";

const ANSWER: Record<AttendeeInfo["response"], { icon: string | null; label: string }> = {
  accepted: { icon: "accept", label: "Accepted" },
  tentative: { icon: "tentative", label: "Tentative" },
  declined: { icon: "decline", label: "Declined" },
  needsAction: { icon: null, label: "None" },
};

/** Waiting for an answer, and not the organizer (you). */
const waiting = (a: AttendeeInfo) => a.response === "needsAction" && !a.organizer && !a.isSelf;

export function ResponsesDialog({ detail, onClose }: { detail: EventDetail; onClose: () => void }) {
  const people = detail.attendees;
  const [sent, setSent] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  // a series' invitation says how it recurs (not just its first date, which may be years ago)
  const series = describeRecurrence(detail.recurrence, detail.seriesStart ?? detail.start, detail.seriesEnd ?? detail.end, detail.allDay);
  const resend = async (emails: string[]) => {
    setBusy(true);
    try {
      // only the ones it went to: someone no longer invited, or a send that failed, stays offered
      const r = await cal.resendInvite(detail.id, emails, series || undefined);
      setSent((s) => new Set([...s, ...r.sent.map((e) => e.toLowerCase())]));
      if (r.error) errorBox(r.error);
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(false);
    }
  };
  const count = (r: AttendeeInfo["response"]) => people.filter((a) => !a.organizer && a.response === r).length;
  const pending = people.filter((a) => waiting(a) && !sent.has(a.email.toLowerCase())).map((a) => a.email);
  return (
    <Dialog
      title="Responses"
      width={620}
      onClose={onClose}
      footer={
        <>
          <button className="cm-button" disabled={busy || !pending.length} onClick={() => resend(pending)}>
            Resend to All Who Haven't Responded{pending.length ? ` (${pending.length})` : ""}
          </button>
          <span className="cm-fill" />
          <button className="cm-button" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="cm-resp-summary">
        {count("accepted")} accepted, {count("tentative")} tentatively accepted, {count("declined")} declined, {count("needsAction")} not yet responded.
      </div>
      <table className="cm-resp-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Attendance</th>
            <th>Response</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {people.map((a) => {
            const ans = ANSWER[a.response] ?? ANSWER.needsAction;
            return (
              <tr key={a.email}>
                <td>
                  <div dir="auto">{a.name || a.email}</div>
                  {/* always a second line, so every row is as tall as one with a name and an address */}
                  <div className="cm-muted">{a.name ? a.email : " "}</div>
                  {a.comment && (
                    <div className="cm-resp-comment" dir="auto">
                      “{a.comment}”
                    </div>
                  )}
                </td>
                <td>{a.organizer ? "Meeting Organizer" : a.optional ? "Optional Attendee" : "Required Attendee"}</td>
                <td>
                  {a.organizer ? (
                    "—"
                  ) : (
                    <span className="cm-resp-answer">
                      {ans.icon && <Icon name={ans.icon} size={14} />}
                      {ans.label}
                    </span>
                  )}
                </td>
                <td>
                  {waiting(a) &&
                    (sent.has(a.email.toLowerCase()) ? (
                      <span className="cm-muted">Invitation sent</span>
                    ) : (
                      <button className="cm-button" disabled={busy} onClick={() => resend([a.email])}>
                        Resend Invitation
                      </button>
                    ))}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="cm-muted cm-resp-note">Responses as of the last Send/Receive.</div>
    </Dialog>
  );
}
