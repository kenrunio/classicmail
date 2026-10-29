# Cross-domain contracts

Methods/components one domain provides for another. Providers MUST implement these exactly;
consumers must cope gracefully (empty state, no crash) if a method returns `no_method` while
domains are being built in parallel.

All times are epoch ms (UTC); all-day dates `YYYY-MM-DD`.

## Calendar provides
* `calendar.listEvents { start: number, end: number, calendarIds?: number[] }` →
  `CalEvent[]` — every visible event overlapping [start, end), selected calendars only unless ids given.
  ```ts
  interface CalEvent {
    id: number; accountId: number; calendarId: number; calendarColor: string;
    summary: string; location: string; start: number; end: number; allDay: boolean;
    startDate?: string; endDate?: string;         // all-day (end exclusive)
    isRecurring: boolean; isMeeting: boolean; myResponse?: "accepted"|"tentative"|"declined"|"needsAction";
    busyStatus: "busy"|"free"|"tentative"|"oof"; isPrivate: boolean; hasReminder: boolean;
  }
  ```
* `calendar.respondToInvite { accountId: number, icalUid: string, response: "accepted"|"tentative"|"declined", comment?: string, sendResponse?: boolean }`
  → `{ eventId: number | null }` — used by the mail reading pane for meeting requests.
* `calendar.findByIcalUid { accountId, icalUid }` → `CalEvent | null` (shows "Accepted"/conflicts in the invite bar).
* Front end: `src/modules/calendar/reminders.ts` exports `startReminderService(): () => void` (main
  window calls it once; it listens for `reminder.due` and opens the `reminders` window).

## Contacts provides
* `contacts.autocomplete { query: string, limit?: number }` → `{ name: string, email: string, source: "contact"|"other"|"recent" }[]`
  (contacts, Google "other contacts", plus recipients from sent mail if the mail tables exist).
* `contacts.findByEmail { email }` → `{ id, displayName, company, jobTitle, phones: {type,value}[], photoUrl? } | null`
* Front end: `src/modules/contacts/SelectNames.tsx` exports
  `SelectNamesDialog(props: { initial: { to: Addr[], cc: Addr[], bcc: Addr[] }, onOk(v), onClose() })`
  where `Addr = { name: string, email: string }` — the Outlook "Select Names" dialog (To/Cc/Bcc buttons).

## Tasks provides
* `src/modules/tasks/TodoBar.tsx` exports `TodoBar({ onClose })` (already referenced by the shell).

## Mail provides
* `mail.listFlagged {}` → `{ id, accountId, subject, from, receivedAt, flagDue?: string, flagStatus: "flagged"|"complete" }[]`
  (for the To-Do Bar task list; double-click opens `message` window with `{ id }`).
* `mail.setFlag { ids: number[], flag: "none"|"today"|"tomorrow"|"thisWeek"|"nextWeek"|"noDate"|"custom"|"complete", due?: string }`
* `mail.compose`-family and `message` window params: `openWindow("message", { id })`,
  `openWindow("compose", { mode: "new"|"reply"|"replyAll"|"forward"|"draft", id?, to?, subject? })`.
* Rust: `crate::mail::...` stays private to mail; other domains use rpc.
* Mail emits `mail.newMail { accountId, messages: [{ id, from, subject, preview }] }` when new mail
  arrives in an Inbox (not on first full sync).

## Rules (built after mail)
* Rust hook called by mail sync after new Inbox messages are stored:
  `crate::rules::on_new_messages(ctx: &Arc<AppCtx>, account_id: i64, message_ids: &[i64]) -> anyhow::Result<()>`.

