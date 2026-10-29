// Calendar items as text for others: Reply / Forward of a meeting (an e-mail with the
// meeting's details quoted, like Outlook's "-----Original Appointment-----") and Quick Print.
import type { EventDetail } from "../../api/calendar";
import type { Addr } from "../../api/mailCompose";
import { htmlToText } from "../mail/compose/editor";
import { escapeHtml } from "../mail/format";
import { fmtLongDate, fmtTime, plusDays } from "./dates";
import { notesParts } from "./notesTable";
import { printPage } from "./print";
import { describeRecurrence } from "./recurrence";
import { SHOW_AS } from "./actions";

/** Reply is a Reply All; the message window can narrow it to the organizer (Reply to One). */
export type ItemMailMode = "reply" | "forward";

const NOT_PEOPLE = /(@resource\.calendar\.google\.com|@group\.calendar\.google\.com|@group\.v\.calendar\.google\.com)$/i;

const person = (email: string | null | undefined, name?: string | null): Addr | null => (email && !NOT_PEOPLE.test(email) ? { email, name: name ?? "" } : null);

/** Google descriptions may be HTML (made in Google Calendar, or an Outlook series) or plain text (ours). */
export function isHtmlNotes(description: string): boolean {
  return /<\/?(br|p|div|a|b|i|u|ul|ol|li|span|strong|em|font|table)\b/i.test(description);
}

/** The notes as text; a link whose words aren't its address keeps the address: "Join <https://…>". */
export function notesText(description: string): string {
  if (!isHtmlNotes(description)) return description;
  const doc = new DOMParser().parseFromString(description, "text/html");
  doc.querySelectorAll("a[href]").forEach((a) => {
    const href = (a.getAttribute("href") ?? "").trim();
    const text = (a.textContent ?? "").trim();
    if (/^(https?:|mailto:)/i.test(href) && text && !href.replace(/^mailto:/i, "").includes(text) && !text.includes(href)) a.append(` <${href}>`);
  });
  return htmlToText(doc.body.innerHTML).trim();
}

/** "Tuesday, October 14, 2026 3:00 PM-4:00 PM" / all day: "Tuesday, October 14, 2026" (-"Thursday, …") */
export function whenText(d: Pick<EventDetail, "start" | "end" | "allDay">): string {
  if (d.allDay) {
    // the last day by the calendar, not end - 24 h (DST days have 23 or 25 hours)
    const last = plusDays(d.end, -1).getTime();
    return last > d.start ? `${fmtLongDate(d.start)} - ${fmtLongDate(last)}` : fmtLongDate(d.start);
  }
  const sameDay = new Date(d.start).toDateString() === new Date(d.end).toDateString();
  return sameDay ? `${fmtLongDate(d.start)} ${fmtTime(d.start)}-${fmtTime(d.end)}` : `${fmtLongDate(d.start)} ${fmtTime(d.start)} - ${fmtLongDate(d.end)} ${fmtTime(d.end)}`;
}

const names = (v: { name: string; email: string }[]) => v.map((a) => a.name || a.email).join("; ");

/** The notes as HTML: text kept as typed, tables (any of the notes formats) as real tables. */
function notesHtml(description: string): string {
  return notesParts(notesText(description))
    .map((p) =>
      p.kind === "text"
        ? p.text.trim()
          ? `<div style="white-space:pre-wrap" dir="auto">${escapeHtml(p.text.replace(/^\n+|\n+$/g, ""))}</div>`
          : ""
        : `<table border="1" cellpadding="3" cellspacing="0" style="border-collapse:collapse;margin:6px 0">${p.grid
            .map(
              (r, i) =>
                `<tr>${r.map((c, j) => (i === 0 ? `<th align="left">${escapeHtml(c)}</th>` : `<td${p.numeric[j] ? ' align="right"' : ""}>${escapeHtml(c)}</td>`)).join("")}</tr>`,
            )
            .join("")}</table>`,
    )
    .join("");
}

/** Label / value rows describing the item (empty values left out). */
function detailRows(d: EventDetail): [string, string][] {
  const others = d.attendees.filter((a) => !a.organizer && !NOT_PEOPLE.test(a.email));
  const rooms = d.attendees.filter((a) => NOT_PEOPLE.test(a.email) && a.email.includes("@resource."));
  const where = [d.location, ...rooms.map((r) => r.name)].filter(Boolean).join("; ");
  const rec = d.recurrence?.length ? describeRecurrence(d.recurrence, d.seriesStart ?? d.start, d.seriesEnd ?? d.end, d.allDay) : "";
  const rows: [string, string][] = [
    ["Subject:", d.summary || "Untitled"],
    ["When:", whenText(d)],
    ["Recurrence:", rec],
    ["Where:", where],
    ["Organizer:", d.isMeeting ? d.organizerName || d.organizerEmail || "" : ""],
    ["Required Attendees:", names(others.filter((a) => !a.optional))],
    ["Optional Attendees:", names(others.filter((a) => a.optional))],
  ];
  return rows.filter(([, v]) => v);
}

type Recipients = { to: Addr[]; cc: Addr[] };

/**
 * To / Cc / Subject and the quoted meeting for a Reply / Forward of a calendar item. A reply goes
 * to everyone (organizer and attendees, not me or the rooms); `targets` lets the message window
 * switch it to the organizer alone.
 */
export function itemMail(d: EventDetail, mode: ItemMailMode): Recipients & { subject: string; quotedHtml: string; targets?: { one: Recipients; all: Recipients } } {
  const me = d.accountEmail.toLowerCase();
  const seen = new Set([me]);
  const add = (out: Addr[], a: Addr | null) => {
    if (!a || seen.has(a.email.toLowerCase())) return;
    seen.add(a.email.toLowerCase());
    out.push(a);
  };
  const to: Addr[] = [];
  if (mode === "reply") {
    add(to, person(d.organizerEmail, d.organizerName));
    for (const a of d.attendees) if (!a.isSelf) add(to, person(a.email, a.name));
  }
  // Reply to One: the organizer (my own meeting has no one person to narrow it to)
  const organizer = d.isOrganizer ? [] : to.filter((a) => a.email.toLowerCase() === d.organizerEmail?.toLowerCase());
  const targets = mode === "reply" ? { one: { to: organizer.length ? organizer : to, cc: [] }, all: { to, cc: [] } } : undefined;
  const subject = `${mode === "forward" ? "FW" : "RE"}: ${d.summary || "Untitled"}`;
  const rows = detailRows(d)
    .map(([k, v]) => `<b>${escapeHtml(k)}</b> ${escapeHtml(v)}<br>`)
    .join("");
  const notes = notesHtml(d.description);
  const quotedHtml =
    `<div id="cm-original"><p><br></p><div style="border:none;border-top:solid #B5C4DF 1pt;padding:3pt 0 0 0">` +
    `<p><b>-----Original Appointment-----</b><br>${rows}</p></div>${notes ? `<p><br></p>${notes}` : ""}</div>`;
  return { to, cc: [], subject, quotedHtml, targets };
}

/** Quick Print: the item in Outlook's memo style, printed from a hidden frame. */
export function printItem(d: EventDetail) {
  const rows = [
    ...detailRows(d).filter(([k]) => k !== "Subject:"),
    ["Show Time As:", SHOW_AS.find((s) => s.value === d.busyStatus)?.label ?? ""],
    ["Categories:", (d.categories ?? []).join(", ")],
    ["Calendar:", `${d.calendarName} (${d.accountEmail})`],
  ].filter(([, v]) => v);
  printPage(
    d.summary || "Untitled",
    `body{font-family:Calibri,'Segoe UI',Arial,sans-serif;font-size:11pt;margin:24px}
    .name{font-size:14pt;font-weight:bold;border-bottom:3px solid #000;padding-bottom:4px;margin-bottom:8px}
    table.h{border-collapse:collapse;margin-bottom:14px} table.h td{padding:1px 8px 1px 0;vertical-align:top} td.k{font-weight:bold;width:140px}`,
    `<div class="name" dir="auto">${escapeHtml(d.summary || "Untitled")}</div><table class="h">${rows
      .map(([k, v]) => `<tr><td class="k">${escapeHtml(k)}</td><td dir="auto">${escapeHtml(v)}</td></tr>`)
      .join("")}</table>${notesHtml(d.description)}`,
  );
}
