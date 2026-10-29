# Mail Merge — design

Date: 2026-10-12 · Status: approved design, spec awaiting review

## Goal

Send one personalised e-mail per row of an Excel/CSV list, written in the normal New Message
window, with `{keyword}` placeholders filled from the row. Typical mailing: 100–500 investors.
Keep it simple: no CRM logging, no per-recipient attachments, no Cc/Bcc.

## User experience

**Entry points.** Tools › **Mail Merge…** (main menu) and a **Mail Merge** button next to **New**
on the mail toolbar. Both show a file picker (`*.xlsx; *.xls; *.xlsm; *.ods; *.csv`), load the
list, then open the compose window in merge mode (`openWindow("compose", { mode: "new", merge: <path> })`).

**Merge mode in the compose window**
- To / Cc / Bcc rows are hidden. In their place a merge bar shows
  `Mail merge: <file name> — <N> recipients`, a **Change List…** button and an
  **Insert Field ▾** dropdown listing the columns (inserts `{Column Name}` at the cursor; into the
  Subject when the Subject box has focus).
- From account / alias, subject, body (incl. signature), attachments, importance etc. work as
  usual and are the same for every message.
- The **Send** button becomes a menu:
  - **Send Test to Me** — one message built from a random valid row, sent to the From address,
    subject prefixed `[TEST] `. Status bar says which row (e.g. "Test sent using row 37 — dan@x.com").
    The window stays open.
  - **Save All as Drafts** — one Gmail draft per row (appears in Drafts). Window closes when done.
  - **Send All…** — queues one Outbox message per row, paced (below). Window closes.
- Autosave/Ctrl+S in merge mode saves nothing to Gmail (a template with `{fields}` is not a
  draft to anyone); closing asks "Discard this mail merge?".

**Pre-flight check** (before Save All as Drafts / Send All) — one dialog listing:
- number of messages that will be created;
- rows skipped: blank or invalid e-mail (with row numbers, first 10 shown);
- duplicate addresses (case-insensitive) — only the first row is used;
- **unknown fields**: `{x}` in subject/body that matches no column → blocking error, must fix;
- rows where a field used in the text is blank → listed, user may continue;
- more than 2,000 messages → warning (Google Workspace daily limit).
Buttons: **Send** / **Save Drafts** (as applicable) and **Cancel**.

## List rules

- First sheet of a workbook; first non-empty row = headers; fully empty rows ignored.
- **E-mail column**: the first header that, trimmed and case-insensitive, equals `email` or
  starts with `email ` (e.g. `Email`, `Email Address`). None → error "The list needs a column
  titled Email (or starting with 'Email ')."
- A cell may hold one address; surrounding spaces and a `Name <addr>` form are accepted. Valid =
  contains exactly one `@` with text on both sides and a dot in the domain.
- Every other non-empty header is a field. **Matching**: `{key}` matches a header when both,
  lower-cased with spaces/underscores/hyphens removed, are equal (`{firstname}` = `{First Name}`
  = `{first_name}`). The e-mail column is also usable as a field.
- Cell values as shown in Excel: text as is; whole numbers without `.0`; other numbers as Rust
  `f64` display; dates `dd/mm/yyyy` (date-times `dd/mm/yyyy HH:MM`); booleans `TRUE`/`FALSE`;
  errors and empty → empty string.
- CSV: delimiter auto-detected among `,` `;` tab (most frequent in the header line); encoding:
  UTF-8 (BOM stripped) if valid UTF-8, else Windows-1255.

## Filling

- Placeholders are `{...}` with 1–60 chars of anything but `{}` and newlines. Unknown ones are
  reported by the pre-flight check, never sent.
- Subject: plain substitution. Body HTML: values HTML-escaped (`& < > "`), substituted in the
  final HTML (after `finalizeHtml`, so signature and font wrapper are included). The editor can
  split a placeholder with formatting tags (`{<b>first</b>name}`); tags inside the braces are
  ignored when matching and removed together with the placeholder.
- Plain-text mode bodies are filled the same way without escaping.

## Back end — `src-tauri/src/mail/merge/`

- `list.rs` — `load(path) -> MergeList { file_name, headers, email_col, rows: Vec<Vec<String>> }`
  using `calamine` (xlsx/xls/xlsm/ods) and `csv` + `encoding_rs`.
- `fill.rs` — `placeholders(text)`, `key_norm`, `fill_text`, `fill_html`, `check(list, subject,
  html) -> Preflight { valid_rows, skipped, duplicates, unknown_fields, blank_fields, over_limit }`.
- `mod.rs` — rpc:
  - `mail.merge.load { path }` → `{ fileName, headers, emailColumn, rowCount }`.
  - `mail.merge.check { path, compose }` → `Preflight`.
  - `mail.merge.run { path, compose, mode: "test" | "drafts" | "send" }` →
    `{ created, skipped, testRow? }`. The list is re-read from `path` at run time (the file is
    the source of truth; it is small). For each valid row it clones the `Compose`, sets `to` to
    the row's address, fills subject/html, then:
    - `send`: `outbox::queue` with `defer_until = now + i × gap`, `gap = 6 s` (≈10/min; 300 rows
      ≈ 30 min). Merge outbox rows don't carry the template's draft id.
    - `drafts`: `drafts::save` sequentially (Gmail quota limiter paces it); progress via
      `ctx.events.status`.
    - `test`: one row chosen at random, `to` = From address, subject `[TEST] `-prefixed,
      `outbox::queue` with no delay.
- **Outbox wake-up**: today deferred Outbox items are only sent on the next mail poll. The sync
  worker's sleep also wakes when the earliest queued Outbox `send_at` for the account is due,
  and such a wake runs only the Outbox step (not a full mail sync). This also makes Delay
  Delivery exact.

## Front end

- `src/api/mailMerge.ts` — typed wrappers.
- `src/modules/mail/merge.ts` — `startMailMerge()` (file picker → load → open compose).
- `ComposeWindow.tsx` — `merge` param: merge bar, Insert Field, hidden recipients, Send menu,
  pre-flight dialog, no Gmail autosave.
- Tools menu item (`src/shell/mainMenus.ts`) and mail toolbar button (`src/modules/mail/index.tsx`).

## Errors

Unreadable/locked file ("Close the file in Excel and try again" for sharing violations), no
e-mail column, no valid rows, unknown fields → message boxes; nothing is queued unless the whole
batch was built successfully (build all messages first, then insert the Outbox rows in one
transaction).

## Testing (no real Google, no real e-mail)

- `list.rs`: xlsx built in-test (via `rust_xlsxwriter` dev-dependency) and CSV fixtures — header
  detection, `Email Address`, Hebrew UTF-8 with BOM, Windows-1255, `;` delimiter, numbers/dates,
  blank rows.
- `fill.rs`: key normalisation, escaping, split-by-tags placeholders, unknown/blank detection,
  duplicates, invalid addresses.
- `mod.rs` against the fake Google server: `send` queues N rows with 6 s spacing and correct
  To/subject/body; `drafts` creates N drafts; `test` sends one to the From address.
- Vitest: merge-mode compose helpers (Send menu choice → rpc), if logic is extracted.
- Manual check in a dev slot with a demo account.
