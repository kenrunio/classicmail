# Google API notes — what ClassicMail relies on

ClassicMail was built against the in-process fake (`src-tauri/src/fake_google`). This file lists the
real-Google behaviour our code assumes (the fake mirrors it), what was changed after the
conformance review (Oct 2026), and what to watch on the first real sign-in.

## Quotas and pacing (`google/limits.rs`)

Google meters per user per minute. Projects created **after 1 May 2026** get the new quotas
(older projects keep their old ones):

| API | Per-user quota | Notes |
|---|---|---|
| Gmail | 6,000 units / min | `messages.get` 20, `attachments.get` 20, `messages.trash` 20, `messages.list` 5, `messages.modify` 5, `history.list` 2, `labels.*` get/list 1, `batchModify` 50, `messages.send` / `drafts.send` 100, `drafts.update` 15 |
| Calendar | 600 requests / min | sliding window, 403 or 429 `usageLimits` when exceeded |
| People | not published | we assume 90 / min; mutations must be sequential (Google: "Mutate requests for the same user should be sent sequentially") |
| Tasks | 50,000 / day courtesy | we assume 300 / min |

* Each account's `Api` has a token bucket per API, sized so that a minute never exceeds 95 % of the
  quota. Sync and body prefetch run as **background** work and wait; user actions (open a
  message, save a contact) may borrow one bucket of debt so they are never queued behind sync.
* Gmail budget: settings key `google.gmailUnitsPerMinute` (default 6000). If the Cloud project is
  older and still has 15,000 units/min, set it to `15000` (restart the app).
* Errors retried by `client.rs`: 429, 403 with reason `rateLimitExceeded` / `userRateLimitExceeded`,
  5xx — back-off 1 s, 2 s, 4 s, 8 s, 16 s plus jitter, or `Retry-After` (seconds or HTTP date).
  A `Retry-After` above 90 s (Gmail's daily sending limit) is reported at once instead of waited
  out. Rate-limit errors never count as permanent: pending local changes are retried later, not
  thrown away (a plain 403 / 400 / 404 / 410 is permanent).
* Network errors are retried only for GET / PUT / PATCH / DELETE or when the connection was never
  made — a POST that may have reached Google (send, insert event, create contact) is not repeated.
* Responses are gzip'ed: Google only does that when the User-Agent contains `gzip`.

### First sync estimate

Gmail full sync costs ~20 units per message (`format=metadata`) plus 5 per 500 ids listed. At our
pace (≈ 4,500 units/min sustained):

| Mailbox | Headers | ≈ time |
|---|---|---|
| 10,000 messages | 200,000 units | 45 min |
| 50,000 messages | 1,000,000 units | 3 h 40 min |
| 100,000 messages | 2,000,000 units | 7 h 20 min |

Newest mail comes first; the Inbox is usable after the first minutes. The full sync runs in
2-minute slices so calendar, contacts and tasks sync in between, and mail that arrives meanwhile is
fetched (with new-mail alerts) between slices. Progress survives restarts (page token in
`sync_state`; a stale token restarts the listing without refetching what is already here). Body
prefetch (last 30 days, ≤400 per pass) starts only after all headers are in.
With a 15,000 units/min project and the setting raised, divide the times by ~2.5.

### Fast initial download over IMAP (`mail/imap_sync.rs`, `google/imap.rs`)

The table above is the fallback. By default (`mail.fastInitialSync`, Tools › Options ›
Send/Receive) the first full sync downloads all headers over Gmail IMAP instead, which the per-user
API quota doesn't meter (Gmail IMAP only caps download at ~2.5 GB/day; headers are ~2 KB each):

1. The full sync records `profile.historyId` as before (`mail.full.historyId`).
2. `imaps://imap.gmail.com:993`, `AUTHENTICATE XOAUTH2` with the account's access token (scope
   `https://mail.google.com/`, already granted). Folders are found by special-use flag (`\All`,
   `\Trash`, `\Junk`; XLIST `\AllMail` / `\Spam` as a fallback) — names are localised.
3. Each folder is opened with **EXAMINE** (read-only) and fetched newest first, 2,000 UIDs per
   `UID FETCH (UID X-GM-MSGID X-GM-THRID X-GM-LABELS FLAGS INTERNALDATE RFC822.SIZE
   BODY.PEEK[HEADER.FIELDS (...)])`. Gmail documents X-GM-MSGID / X-GM-THRID as the decimal form
   of the API's hex ids; we store `{:016x}`. Labels: `\Inbox`→INBOX, `\Sent`, `\Draft`,
   `\Starred`, `\Important`, user labels by name (modified UTF-7) → label id; no `\Seen` →
   UNREAD; the folder gives TRASH / SPAM; categories via `UID SEARCH X-GM-RAW "category:…"`.
   Rows go through the same `meta_from` / `upsert_meta` as the API path. No snippet (it is filled
   when the body is downloaded).
4. Then the normal API listing runs from the top but only **checks** ids (5 units per 500) and
   fetches what IMAP didn't have; `resync_labels` (one `messages.list?labelIds=X` per label, 4 at a
   time) makes labels exactly the API's; history from step 1 covers changes made meanwhile.
5. Resumable (per-folder UIDVALIDITY + lowest UID done in `sync_state` `mail.imap.state`), sliced
   like the API sync. Network errors: the API path runs for that cycle, IMAP is retried (3 tries).
   Sign-in refused (IMAP disabled by the Workspace admin), no All Mail folder: status bar shows
   "Fast download unavailable — using the standard method" and the API path does it all.

Fake server: ~50,000 messages in ~10 s for the IMAP part, ~45 s for the whole first sync (most of
it the fake's slow listing). Real Gmail: expect a few minutes for 100k messages.

## OAuth (`google/auth.rs`)

* Installed-app loopback flow: redirect `http://127.0.0.1:<random port>/` (Desktop clients accept
  any port), PKCE S256, `access_type=offline` + `prompt=consent` (always yields a refresh token),
  `login_hint`. Each loopback connection is served on its own task — browsers open silent
  speculative connections that used to be able to block the real redirect.
* Scopes: `openid email profile https://mail.google.com/ …/auth/calendar …/auth/contacts
  …/auth/contacts.other.readonly …/auth/tasks`. Google's consent screen lets users untick scopes;
  sign-in now fails with a clear message if Gmail, Calendar, Contacts or Tasks were not granted
  (Other contacts is optional — autocomplete only).
* Refresh: `invalid_grant` → "needs to sign in again"; `invalid_client` → re-import the client
  file; other token-endpoint failures are transient. 401 from an API → refresh once, retry.
* User info: `https://openidconnect.googleapis.com/v1/userinfo` → `email`, `name`.

## Gmail

* `messages.list`: `maxResults` ≤ 500, `includeSpamTrash=true` for the full sync, newest first,
  unknown/stale `pageToken` → 400. `labelIds=X` is used to rebuild label membership cheaply after
  a resync (history expired) instead of re-reading every message.
* `messages.get format=metadata` with repeated `metadataHeaders`. Header values may come back with
  RFC 2047 words **already decoded** by Gmail (uncertain — we handle both): address headers are
  split before decoding, and a comma inside an unquoted decoded display name
  (`Cohen, Dana <d@x>`) is glued back to its address.
* `format=full`: text parts carry `body.data` (transfer-decoded, **original charset** — decoded
  with the part's `charset`, windows-1255 / iso-8859-8 included); parts with a filename and big
  parts carry `attachmentId` (ids are not stable across fetches; a 400/404 re-reads the part).
* `history.list`: `startHistoryId` too old → 404 → full resync (cheap label resync + fetch of
  what's new); history ids are strings; drafts autosave shows up as messagesDeleted + messagesAdded.
* Send: JSON `{raw, threadId}` up to 2 MB, above that
  `POST /upload/gmail/v1/users/me/messages/send?uploadType=multipart` (JSON metadata
  `{threadId}` + `message/rfc822`). `uploadType=media` cannot carry `threadId` (a query parameter
  `threadId` is a 400) — the old code lost threading for large replies. Drafts create/update use
  the same split. Max 35 MB per upload, 25 MB of attachments (checked in compose).
* Threading: Gmail joins `threadId` only when In-Reply-To/References are set **and** the Subject
  matches (Re:/Fwd: ignored); without `threadId` an API send always starts a new conversation.
  The fake now behaves the same. A rejected thread id (400 or 404) → sent as a new conversation.
* `batchModify` ≤ 1000 ids (chunked); trash / untrash / delete are per message.

## Calendar v3

* `calendarList` (≤ 250 per page): `summaryOverride`, `backgroundColor`, `accessRole`,
  `primary`, `selected`, `hidden`, `timeZone`, `defaultReminders`. Hidden and **freeBusyReader**
  calendars are skipped; ids with `#` / `@` are percent-encoded in paths. A calendar Google refuses
  (403/404) is logged and skipped instead of failing the whole calendar sync.
* `events.list` with `singleEvents=true&orderBy=startTime` (orderBy without singleEvents is a
  400), `timeMin/timeMax` RFC 3339 with offset, `maxResults` 2500, all pages; a second listing
  without singleEvents brings the series masters. All-day: `date`, end exclusive. Instances:
  `<masterId>_<YYYYMMDDTHHMMSSZ>` (`_<YYYYMMDD>` all-day), `recurringEventId`,
  `originalStartTime`; cancelled instances are absent unless `showDeleted`.
* Recurring timed events need `start.timeZone` (we always send it). UNTIL is UTC (`…Z`) for timed
  series, a DATE for all-day ones (built by the UI).
* `eventType` `birthday` (contacts' birthdays on the primary calendar), `fromGmail` and
  `workingLocation` are read-only in ClassicMail; Google rejects time/summary changes to them
  and deleting birthdays.
* Writes: `sendUpdates=all|none`, delete of an already-deleted event → 404/410 = success.

## People API

* `connections.list`: `personFields` required, `pageSize` ≤ 1000, `requestSyncToken=true`.
  **No `sortOrder`** any more: it is ignored when a sync token is requested, and Google requires
  every call that uses a sync token to repeat the first call's parameters (the fake enforces it).
* Expired sync token (tokens live 7 days): **400 FAILED_PRECONDITION** with
  `ErrorInfo.reason = EXPIRED_SYNC_TOKEN` (not 410) → full resync. People errors are
  `google.rpc.Status` (`status` + `details`), parsed alongside the Gmail-style `errors[].reason`.
* Deleted contacts come back in incremental syncs with `metadata.deleted = true`.
* `updateContact` needs the current `etag`; it now reads the contact first and sends **only the
  fields the user changed** in `updatePersonFields`, carrying over entries ClassicMail doesn't
  model (other events / relations, other apps' clientData, extra organizations). The old code
  replaced 15 fields wholesale and wiped those. Stale etag → 400 FAILED_PRECONDITION → read & retry
  once.
* `otherContacts.list`: `readMask=names,emailAddresses,metadata`, own sync token.

## Tasks API

* `tasks.list`: `maxResults` ≤ 100; `showCompleted=true&showHidden=true` (tasks completed in
  Google's apps are hidden at once), `showDeleted` + `updatedMin` for incremental pulls,
  `showAssigned=true` (tasks assigned from Docs / Chat).
* `due` is a date: we send `YYYY-MM-DDT00:00:00.000Z`, Google ignores the time.
* `move`: `parent` / `previous` must not be hidden tasks (we skip hidden neighbours);
  `destinationTasklist` moves between lists (not for recurring tasks).

## Watch on the first real sync

0. **Fast IMAP download** (log lines `mail: fast download…` / `fast IMAP download…`): status bar
   "Downloading message headers (fast): n of N", then "Checking the message list". If it says
   "Fast download unavailable", check the log for the IMAP error (Admin console › Gmail › End user
   access › IMAP). After it, few or no "messages of this page were not in the fast download" lines should
   appear; many would mean ids don't match (expected: X-GM-MSGID hex = API id).

1. **Status bar / log for 429 or 403 rateLimitExceeded** — if they appear often, lower
   `google.gmailUnitsPerMinute`; if never and the sync is slow, check the project's real quota in
   Cloud Console › IAM › Quotas and raise the setting.
2. **Hebrew subjects and sender names** look right in the list (header decoding is the main
   uncertainty; both encoded and pre-decoded forms are handled).
3. **Reply threading** in Gmail web: a reply sent from ClassicMail should land in the same
   conversation; a reply with a big attachment (multipart upload — not exercised against real
   Google yet) too.
4. **Consent screen**: tick every box; a missing scope now stops sign-in with a message.
5. **Contacts edit** of a contact that has data from Google Contacts (birthdays, custom fields):
   the extra data must survive. Edits still win field-by-field over concurrent edits made
   elsewhere since the last pull (no 3-way merge).
6. **Calendar volume**: every 5 minutes each visible calendar is listed for −12/+24 months (no
   sync tokens). Fine for a handful of calendars; with many busy calendars watch the time a
   calendar cycle takes.
7. **People quota** numbers are assumed (90 req/min). A bulk contact operation is paced to that.
8. **History gap**: if ClassicMail isn't run for over a week, Gmail's history expires and mail
   resyncs: new messages only cost quota; labels are rebuilt with a few hundred units.
