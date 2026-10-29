# Reply All by default, Translate my reply, Reply with AI, AI Review — design

Date: 2026-10-13 · Status: approved design, spec awaiting review

## 1. Reply = Reply All, with a toggle

**Main window**
- Mail toolbar: the **Reply to All** button is removed; **Reply** opens a Reply All
  (`compose("replyAll", id)`). With one person on the message that is an ordinary reply.
- **Ctrl+R** and the right-click / Actions-menu **Reply** also do Reply All. **Ctrl+Shift+R**
  and the menus' **Reply to All** item stay (same result).
- The reading pane / message window toolbars follow the same rule (their Reply does Reply All).

**Compose window** (reply / reply-all modes only)
- Back end: `Prepared` gains `replyTargets: { one: { to, cc }, all: { to, cc } }`, computed with the
  existing `reply_recipients(src, false|true, own)`; absent for new / forward / draft.
- A **Reply to One** button in the Message tab's Names group, next to Check Names.
  - Pressing it sets To/Cc to `one`, plus any recipients the user added that are in neither
    `one` nor `all`. The label becomes **Reply All**.
  - Pressing **Reply All** sets To/Cc to `all`, plus the user-added names.
  - Initial label: **Reply to One** when the current To+Cc has more than one address, otherwise
    **Reply All**. The button is disabled when `one` and `all` are the same (one person).
- Marks the message dirty like any recipient change. Undo: toggle again.

## 2. Translate my reply

- Shown next to **Spelling** (Proofing group) when the language of the quoted original
  (`detectLanguage(text of quotedHtml)`, `src/modules/ai/lang.ts`) is not English and not null.
  Label **Translate to <Language>** (e.g. Hebrew); icon: the translate icon used by the Translate bar.
  Hidden for new messages, forwards without a non-English original, and plain-text mode.
- Pressing it:
  1. Splits the editor HTML into *reply* and *signature* (see "Signature split").
  2. `ai.compose.translate { html, language }` → HTML in that language: informal, everyday
     e-mail register (for Hebrew: the way Israelis write work e-mail, not formal/literary), same
     paragraphs/formatting/links, names and numbers unchanged.
  3. Replaces the reply part in the editor (one transaction: Ctrl+Z works), keeps the signature,
     and remembers the English HTML. The button becomes **Back to English**.
- **Back to English** restores the remembered English reply part. If the translated text was
  edited since, ask "Discard your changes to the translation?" first.
- After a restore the button is **Translate to …** again. Editing the English and translating
  again translates the current text.
- RTL: the translated reply part gets `dir="rtl"` on its blocks when the language is RTL.
- Busy state: the button shows "Translating…" and is disabled; errors → message box.

## 3. Reply with AI

- Main mail toolbar: **Reply with AI** next to Reply (also in the Actions menu and the message
  right-click menu). Disabled without a selected single message.
- Opens the compose window as Reply All with `aiDraft=1`. When the window is ready it shows an
  info line "Drafting a reply with AI…" and calls `ai.compose.draftReply { messageId, accountId }`.
- Back end builds the context from the source message (subject, from, to/cc, date, and the full
  body text incl. the quoted history, capped at ~30,000 characters, newest first) and asks for a
  **short English reply** (2–6 sentences, matching the thread's tone, first-name greeting when
  appropriate, no signature, no subject line, no invented facts — unknowns become `[…]`
  placeholders the user can fill or hand to AI Review). Output: plain paragraphs → `<p>` HTML.
- The draft is inserted at the top of the editor, above the signature, as one transaction (Ctrl+Z
  removes it). If the user already typed before the draft arrives, the draft is inserted above
  their text. Failure → info line "AI draft failed: …"; the reply window stays usable.

## 4. AI Review

- Compose window, Proofing group: **AI Review** button (Ctrl+Shift+A not assigned — button only).
- Instructions are text inside `{ … }` (changed 2026-10-18: square brackets are ordinary text —
  quoted mail is full of `[mailto:…]` and `[EXTERNAL]`). In a mail merge, `{Field}` with a list
  column's name is a merge field, not an instruction.
- Instructions typed into the quoted original count too (people edit an old e-mail reused as a
  template, or answer inline): only braces that weren't in it when the window opened (for a
  reopened draft: braces that don't look machine-made). Each block holding one is reviewed on its
  own, with the original's other braces masked (`compose/quotedInstructions.ts`).
- Pressing it:
  1. Splits reply / signature as in §2. If neither the reply part nor the original has an
     instruction: info "No {instructions} found. Write what you want changed in curly braces, e.g.
     {thank John for the great work}." and stop.
  2. `ai.compose.review { html, context, fields }` where `context` is the text of the quoted
     original(s) (names, facts) plus the current To/Cc names and the subject.
  3. The AI returns the reply HTML with every instruction carried out *in place* and removed;
     all other text, formatting and links unchanged; same language as the surrounding text.
  4. Validation: non-empty, and no instruction brackets left (else one retry, then show the
     result anyway with an info line). Replace the reply part in one transaction (Ctrl+Z
     undoes). Info line: "AI Review applied N instruction(s). Ctrl+Z to undo."
- Busy: "Reviewing…", disabled. Errors → message box.

## Shared pieces

**AI availability.** The three AI buttons are enabled when `ai.getSettings` reports a
configured provider and key; otherwise disabled with the tooltip "Set up AI in Tools › Options ›
AI". Reply to One / Reply All needs no AI.

**Signature split** (`src/modules/mail/compose/split.ts`, unit-tested): find the signature in
the editor HTML by the account's current signature text (same normalised-text matching
`swapSignature` uses, extracted into the helper). Returns `{ reply, signature }`; when no
signature is found, everything is `reply`.

**Back end** `src-tauri/src/ai/compose.rs`, rpc:
- `ai.compose.translate { html, language }` → `{ html }`
- `ai.compose.draftReply { messageId }` → `{ html }`
- `ai.compose.review { html, context, fields }` → `{ html, applied }`
All use `ai::complete` (current provider/model). HTML in/out is sanitised with the existing
compose sanitiser (no scripts/styles beyond inline formatting) before returning. Prompts live in
the module as constants. Bodies sent to the AI are the user's own mail going to their own
configured provider — same as the Translate bar today.

## Testing (no real AI, Google or e-mail)

- Rust: prompts + response handling against the fake AI provider (`ai` tests' fake): translate
  keeps the HTML structure, review strips braces / counts instructions / merge `fields` leave
  `{x}`, draft context includes the quoted history and is capped; sanitising of AI HTML.
- `prepare` test: `replyTargets` for a three-person thread (one vs all, own address excluded).
- Vitest: `split.ts` (signature found / not found / edited), the Reply-to-One toggle logic
  (pure function: current recipients + targets → next recipients and label), bracket detection
  incl. the merge rule.
- Manual check in a dev slot with the demo account and the fake AI provider.
