# ClassicMail — design

An Outlook 2007–style desktop client for Google Workspace (Gmail, Calendar, Contacts, Tasks),
for one or more Google accounts side by side, with Outlook-style Rules.

Decisions (agreed 2026-10-10):

| Topic | Decision |
|---|---|
| Shell | Tauri v2 (Rust back end, WebView2) — React 19 + TypeScript + Vite front end |
| Accounts | Several Google accounts (e.g. a personal and a shared mailbox) in one profile, Outlook-style |
| Google access | Gmail / Calendar / People / Tasks REST APIs, OAuth "Internal" app in the user's own Workspace |
| Message list | One row per message (Outlook 2007), not Gmail conversations |
| Labels | Shown as an Outlook folder tree; nested labels (`a/b`) become subfolders |
| Extras | Tasks + To-Do Bar, Outlook-style Rules (run locally) |
| Shipping | NSIS setup.exe, per-user install, unsigned for now |
| Dev data | Built-in **fake Google server** + demo accounts, so every code path runs without a real account |

---

## 1. Repository layout

```
ClassicMail/                      (Rust build output: CARGO_TARGET_DIR, default C:\dev\outlook_build\target)
  docs/                           design.md (this), google-setup.md, user-guide.md
  scripts/                        dev.mjs (multi-slot dev runner), shot.mjs (CDP screenshots/driving)
  src/                            React front end
    main.tsx                      picks the window by `?window=` query (main, compose, message, ...)
    api/                          rpc.ts (typed invoke wrapper), events.ts, types.ts (shared DTOs)
    ui/                           Office-2007 widget kit (Button, SplitButton, Menu, ContextMenu, Toolbar,
                                  Ribbon, Dialog, Splitter, TreeView, Grid, DateNavigator, Icons, ...)
    theme/office2007.css          colour tokens + base styles (the ONLY place colours are defined)
    shell/                        main window: MenuBar, StandardToolbar, NavigationPane, StatusBar, TodoBar
    modules/mail|calendar|contacts|tasks/   module views shown in the main window
    windows/                      separate windows: compose, message, appointment, contact, task,
                                  reminders, desktop-alert, options, rules, accounts, address-book
  src-tauri/
    src/lib.rs                    builds AppCtx, registers plugins, the `rpc` command, protocols
    src/ctx.rs                    AppCtx (db, events, accounts, sync handle, http)
    src/rpc.rs                    method registry; every module adds handlers in its own register()
    src/events.rs                 typed events pushed to all windows
    src/db/                       connection pool + per-module migrations
    src/google/                   auth (OAuth PKCE + keyring), client (retry/refresh), gmail, calendar,
                                  people, tasks REST wrappers
    src/fake_google/              axum server emulating the 4 Google APIs + OAuth, seeded demo data
    src/sync/                     scheduler + per-domain sync (mail, calendar, contacts, tasks), pending ops
    src/mail/ calendar/ contacts/ tasks/ rules/ settings/   domain logic + `register()` of rpc methods
```

### Module ownership (for parallel work)
Each domain owns: `src-tauri/src/<domain>/**`, its fake API in `src-tauri/src/fake_google/<api>.rs`,
`src/modules/<domain>/**`, and its windows in `src/windows/<domain>*`. Shared files
(`lib.rs`, `ctx.rs`, `rpc.rs`, `db/mod.rs`, `ui/**`, `theme/**`, `shell/**`, `api/rpc.ts`, `api/events.ts`)
are owned by the integrator; domains only *call* them. Each domain's schema lives in
`src-tauri/src/<domain>/schema.rs` as a list of `(version, sql)` migrations keyed by module name, so
domains never edit a shared migration list. Each domain's DTO types live in `src/api/<domain>.ts`.

## 2. Front-end ⇄ back-end contract

* One Tauri command: `rpc(method: string, params: object) -> result` (JSON). Errors come back as
  `{ code, message }`. Front end: `rpc<T>("mail.list", {...})` from `src/api/rpc.ts`.
* Method names are `<domain>.<verb>` e.g. `mail.listMessages`, `calendar.listEvents`.
* Params/results are camelCase JSON (`#[serde(rename_all = "camelCase")]` on Rust structs).
* Back end → front end: a single Tauri event `cm-event` with payload `{ kind, ...data }`:
  - `mail.changed { accountId, folderIds?: number[] }` — message list / counts need refresh
  - `mail.newMail { accountId, messages: [{id, from, subject, preview}] }` — triggers desktop alert
  - `calendar.changed`, `contacts.changed`, `tasks.changed`, `rules.changed`, `settings.changed`
  - `sync.status { accountId, phase, text, progress? }` — status bar text ("Receiving 120 of 4,000")
  - `reminder.due { items }`
  - `accounts.changed`
* Front end uses TanStack Query; `api/events.ts` maps event kinds to query-key invalidations.
* Timestamps cross the boundary as **epoch milliseconds (UTC)**; all-day dates as `"YYYY-MM-DD"`.
  The UI formats in the system time zone.

## 3. Data (SQLite, one file in the app data dir)

WAL mode, FTS5. `PRAGMA foreign_keys=ON`. Migration bookkeeping: `schema_migrations(module, version)`.

Core (owner: integrator):
* `accounts(id, email, display_name, provider 'google'|'demo', color, position, created_at, enabled)`
* `settings(key PRIMARY KEY, value JSON)`
* `sync_state(account_id, kind, value)` — history ids, sync tokens, etc.
* `pending_ops(id, account_id, domain, kind, payload JSON, attempts, last_error, created_at)` —
  local-first changes waiting to be pushed to Google.

Mail: `folders`, `messages`, `message_folders` (many-to-many = Gmail labels), `message_bodies`,
`attachments`, `messages_fts`, `outbox`, `signatures`. Calendar: `calendars`, `events`.
Contacts: `contacts`, `contact_groups`, `contacts_fts`, `other_contacts`. Tasks: `task_lists`, `tasks`.
Rules: `rules`.

### Local-first writes
Every user action (mark read, move, delete, flag, edit event, ...) updates SQLite **immediately**,
emits the change event, and appends a `pending_ops` row. The account's sync worker pushes pending
ops first, then pulls remote changes. A failed op retries with back-off; after 5 failures it is
dropped, the local change is re-synced from the server, and the status bar shows the error.

## 4. Google integration

* OAuth 2.0 installed-app flow: PKCE + loopback redirect (`http://127.0.0.1:<random>/`), system
  browser via the opener plugin. The OAuth client JSON (Desktop app type) is imported once in the
  Account Setup wizard (stored in settings) or embedded at build time from `google-client.json`.
* Scopes: `https://mail.google.com/`, `calendar`, `contacts`, `contacts.other.readonly`, `tasks`,
  `openid email profile`. The app is **Internal** to the user's Workspace domain, so no verification and no 7-day expiry.
* Refresh tokens are stored in Windows Credential Manager (keyring crate), service `ClassicMail`.
* `google::client::Api` wraps reqwest: bearer token, refresh on 401, exponential back-off on 429/5xx,
  base URLs overridable (fake server). One `Api` per account.
* Demo accounts point every base URL at the in-process fake server, token `demo-<email>`.

### Mail sync
1. Labels → `folders` (system labels mapped: INBOX→Inbox, SENT→Sent Items, DRAFT→Drafts,
   TRASH→Deleted Items, SPAM→Junk E-mail; plus a local Outbox; STARRED = flag, UNREAD = state;
   CATEGORY_*, IMPORTANT, CHAT hidden). User labels nest on `/`.
2. Full sync (no history id): record `profile.historyId`, then `messages.list(includeSpamTrash)`
   newest-first, fetch `format=metadata` concurrently (8 in flight, back-off on 429), insert as they
   arrive so the UI fills progressively. Resumable via a stored page token.
3. Incremental: `history.list(startHistoryId)` → added / deleted / label changes. 404 → full resync.
4. Bodies: `format=full` on demand when a message is opened, plus a background prefetch of the most
   recent 30 days (Inbox first) so reading and full-text search are instant. Text parts are decoded
   with their charset (encoding_rs — Hebrew windows-1255 / iso-8859-8 matter).
5. Mail poll every 60 s (configurable), F9 = Send/Receive now.
* Move = label change; Delete = `trash`; Delete in Deleted Items = permanent delete (confirm dialog);
  Flag = STARRED (local flag due-date kept in SQLite); Read = remove UNREAD.
* Send: `mail-builder` builds the RFC 822 message (multipart/alternative + attachments + inline
  images, In-Reply-To/References), queued in `outbox`, sent with `messages.send` (threadId kept).
  Drafts are saved to Gmail drafts.

### Calendar sync
Per selected calendar: `events.list(singleEvents=true, timeMin=-12 months, timeMax=+24 months)`
every 5 min, replacing the window locally. Recurring series edited through the master event
(`recurringEventId`). Responses (Accept/Tentative/Decline) patch our attendee entry.

### Contacts / Tasks
People API `connections.list` with sync tokens; `otherContacts` for address autocomplete.
Google Tasks lists → Tasks folders. Flagged mail also appears in the To-Do Bar task list.

## 5. Fake Google server (`fake_google`)

axum on `127.0.0.1:0`, started in-process when a demo account exists (and in tests). Implements the
subset of endpoints we call with Google's JSON shapes, error codes and paging. State is seeded
deterministically (≈400 messages incl. Hebrew/HTML/attachments/meeting invites, ≈60 events incl.
recurring ones, ≈80 contacts, tasks) and persisted to `fake_google.json` in the data dir. A timer
"delivers" a new message every few minutes in demo mode; `demo.deliverMail` does it on demand.

## 6. UI — Outlook 2007 look (Blue theme)

Main window (top → bottom): native title bar · menu bar (File Edit View Go Tools Actions Help) ·
Standard toolbar (New ▾ | Print | Move to Folder | Delete | Reply, Reply to All, Forward |
Send/Receive ▾ | Find, address-book search box) · body · status bar.
Body columns: Navigation Pane (Favorite Folders, Mail Folders tree, big module buttons Mail /
Calendar / Contacts / Tasks at the bottom, small icon row) | module view | To-Do Bar
(Date Navigator, upcoming appointments, task list with "Type a new task").

Mail view: folder title bar ("Inbox") + Instant Search box; message list "Arranged By: Date /
Newest on top", grouped Today / Yesterday / Last Week / Older, two-line rows when the Reading Pane
is on the right; Reading Pane right/bottom/off. Info bar for blocked pictures. Double-click opens a
message window; compose windows use the 2007 Ribbon (Message / Insert / Options / Format Text).

Palette tokens (all in `theme/office2007.css`): pane header gradient `#E3EFFF→#C4DDFF`, borders
`#6593CF`, dark title text `#15428B`, selected row `#C2DCFD` (focused) / `#E1E1E1` (unfocused),
hover/selected nav buttons orange `#FFE6A0→#FFD167` / `#FFBD69→#FFAC42`, window background
`#BFDBFF`, list background white, Tahoma/Segoe UI 8–9pt (11–12px). No Microsoft logos or icon
artwork — icons are our own SVGs drawn in the same spirit.

Keyboard: Ctrl+1/2/3/4 modules, Ctrl+N new item, Ctrl+R / Ctrl+Shift+R / Ctrl+F reply/all/forward,
Ctrl+Enter send, Del delete, Ctrl+Q / Ctrl+U read/unread, Insert flag, Ctrl+Shift+V move, Ctrl+E
search, F9 Send/Receive, Ctrl+P print.

## 7. Rules (local)

`rules(id, name, enabled, position, account_id?, conditions JSON, exceptions JSON, actions JSON,
stop_processing)`. Applied to messages newly arriving in an Inbox (after sync), and on demand via
"Run Rules Now". Conditions: from, sent to, subject/body/header contains, has attachment,
importance, sent only to me, my name in To/Cc, through account. Actions: move/copy to folder, delete,
mark read, flag, set importance, forward to, desktop alert, stop processing. Rules Wizard dialog with
the classic two-step "Select condition(s) / Edit the rule description (click an underlined value)".

## 8. CRM link (removed)

There was a link to an external CRM (a strip and "Log to CRM") here; it was removed after 1.7.0. Migration crm/2 drops the old `crm_cache` table.

## 9. Shipping

`npm run release` → `tauri build` → `ClassicMail_<ver>_x64-setup.exe` (NSIS, current user,
WebView2 download-bootstrapper) copied to `release/`. App data in `%APPDATA%\com.ekgir.classicmail`.
Tray icon, single instance, optional start with Windows, desktop alerts, reminders window.

## 10. Testing

* Rust: unit tests (MIME, sanitising, rules engine, search parser, recurrence) and integration tests
  that run sync against the fake server.
* Front end: vitest for pure logic.
* End to end: the dev app runs WebView2 with `--remote-debugging-port`, and `scripts/shot.mjs`
  (playwright-core over CDP) drives and screenshots the real app.
* Nothing in tests or dev ever talks to a real Google account.
