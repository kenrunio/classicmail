# Changelog

What changed in each ClassicMail version, newest first. Installers are built with
`node scripts/release.mjs`; each version is a "Version x.y.z" commit on `main`.

## 1.24.1 — 2026-10-29
- First public release on GitHub, with an installer that has no built-in Google client: you import
  your own (see [docs/google-setup.md](docs/google-setup.md)). The welcome screen and Account
  Settings link to that guide.
- The demo mailboxes are a made-up firm, Duvz Software, with made-up people and companies.

## 1.24.0 — 2026-10-28
- **Pop-up boxes answer from the keyboard**, as in Windows: in a Yes/No box press Y or N (the
  underlined letter; C for Cancel, and so on) or Enter for the highlighted button. In every
  dialog, Enter presses the default button (OK), the arrow keys move between the buttons and
  Alt+letter presses the button starting with that letter.
- **Shortcuts work throughout the app**, whatever the keyboard language: Ctrl+R, Alt+F and the
  rest also work with Hebrew (or any other) keyboard on, and Ctrl+1–4 on a French one.
- **Menus from the keyboard**: each menu item has an underlined letter that chooses it; Alt+letter
  or F10 opens a menu with its first item highlighted, and Left / Right move along the menu bar.
- New shortcuts: Ctrl+Shift+I (Inbox), Ctrl+Shift+O (Outbox), Ctrl+Shift+E (New Folder), F3
  (search), Ctrl+M (Send/Receive); Ctrl+N and Ctrl+Shift+M / A / Q / C / K in item windows; Ctrl+P
  in appointments and contacts; Ctrl+C copies the selected contacts (Contacts, Address Book).
- Delete with the folder list focused deletes the folder (after asking), not the selected messages.
  Esc in an open menu or drop-down no longer closes the window too; Delete in the To-Do Bar or the
  calendar list no longer also deletes what is selected in the view; Enter on a focused button
  presses that button; Ctrl+Shift+V in a text box pastes; Delete, Insert and Backspace work with
  the reading pane focused; Tasks shortcuts work without clicking the list first.

## 1.23.0 — 2026-10-28
- **Drag folders around**, as in Outlook: drag a folder onto another folder, onto Inbox, or onto
  the mailbox's name (to the top level) to move it there, with its subfolders and mail. Holding
  the mouse button down first still just changes the order. Undo moves it back.
- **New Folder inside Inbox** makes a real subfolder of Inbox (in Gmail, a label "Inbox/<name>",
  as Outlook makes them), instead of a separate folder at the top level.
- A folder name Gmail keeps for itself (such as "Important" at the top level) now says so, instead
  of "Google API error 400: Invalid label name".

## 1.22.0 — 2026-10-27
- **Usual groups**: type a name in To and press Tab, and the people you usually write to with that
  person appear, highlighted, each where they usually are (To or Cc), learnt from your sent mail and
  the meetings you organised. Tab again keeps them and moves to Cc; Delete, Backspace, Esc or typing
  drops them, and the next name you add asks again (the group those people share). Also in meeting
  invitations (all into the one attendee line).
- After picking a name with Tab, the auto-complete list no longer pops up again over the Cc line.

## 1.21.1 — 2026-10-26
- **Time Zones list shortened** to about 30 regions with general names, as in Outlook
  ("(UTC-05:00) Eastern Time (US & Canada)", "(UTC+01:00) Central Europe (Paris, Berlin, Zurich,
  Rome, Madrid)", "(UTC+02:00) Israel") instead of every city.

## 1.21.0 — 2026-10-26
- **Time Zones** in the appointment / meeting window, as in Outlook: set the start and end in
  another zone (a meeting in New York at 10:00) instead of working out Israel time. The calendar
  shows it in your time, and a note under the times says what that is (17:00–18:00). Your own
  zone is the default; an item set in another zone opens showing its zone.

## 1.20.2 — 2026-10-26
- **Excel previews look like the spreadsheet**: cells stay on one line, numbers are right-aligned,
  and a wide sheet scrolls sideways instead of squeezing every column to one letter. Tables in
  ordinary e-mails no longer collapse that way either.

## 1.20.1 — 2026-10-25
- **Rename someone else's meeting** on your calendar, as in Outlook: the Subject of an invitation
  can be edited. Your subject shows everywhere in ClassicMail (and on your other computers); the
  organizer and the other guests keep theirs, which the meeting window shows under yours.

## 1.20.0 — 2026-10-25
- **Move and copy mail between mailboxes**, like Outlook: drag messages onto another mailbox's
  folder, or Move / Copy to Folder (which now lists every mailbox). Each message is copied across
  whole (date, attachments, unread and flag kept); a move then puts the original in its mailbox's
  Deleted Items. The status bar counts them; Undo removes the copies and brings the originals back.
- **"More than one company" check on Send**: a message to people at two or more outside companies
  (say three at orbit.example and one at random.example) asks you to check before it goes. Your own domains
  don't count, and a reply only asks when it adds a company that wasn't in the conversation. Tick
  "Don't ask me again" to stop it.
- A short English message with only a signature (initials like "J.R.", "US Cell") is no longer
  taken for Italian by the Translate bar.

## 1.19.1 — 2026-10-25
- **Open in New Window**: right-click Mail, Calendar, Contacts or Tasks in the Navigation Pane ›
  Open in New Window opens another ClassicMail window on it — for example Calendar on a second
  screen. It remembers where it was; closing it leaves the main window open.

## 1.19.0 — 2026-10-24
- **AI notice, once**: the first time you use an AI command, ClassicMail explains what goes to
  Gemini or Claude and asks you to turn AI on; until then nothing is sent. Tools › Options › AI
  shows when you agreed and has **Turn AI Off**.
- **Privacy Policy**: Help › Privacy Policy, also linked from Help › About.
- **Removing an account removes everything** kept for it on this computer (search index, cached
  attachments, translations, reminders, settings that named it, undo history) and withdraws its
  access at Google. Old attachment copies from reopened drafts are cleared after a week.
- **The log file holds no mail content**: no subjects, addresses, file names, search words or web
  addresses; ids and error codes only. It is kept small (two files of 2 MB).
- **Connecting your own Google account**: the setup guide (docs/google-setup.md) now covers
  ordinary @gmail.com accounts as well as Google Workspace.

## 1.18.9 — 2026-10-23
- **Open source**: ClassicMail is free and open source under the Apache License 2.0, © 2026 Duvz
  Software, written by Kenny Green. Help › About shows the version, the date it was built and the
  credit.
- **AI models list**: checked once a day by itself; Tools › Options › AI has **Update Models List**
  and shows when it was last checked.
- The welcome screen (no accounts yet) has one "Sign in with Google…" button.

## 1.18.8 — 2026-10-23
- **Web Page or MD looks like the page**: side-by-side columns, headers with a note at the right
  and section bars come through as the page laid them out, and never wider than the window.
- **Wide messages scroll sideways again** while you write.

## 1.18.7 — 2026-10-23
- **View Source**: right-click a message's text › View Source shows tabs above it: Message, HTML
  Source and Original Message (with Copy and Save As). While writing, Format Text › View Source
  lets you edit the message's HTML directly and flip back to see it.
- **Markdown**: a third format beside HTML and Plain Text (with a Preview tab); Markdown you paste
  comes in formatted; Include › Web Page or MD also takes Markdown files and links.
- **Table gridlines**: every table shows faint lines while you write (Format Text › Table › View
  Gridlines); they are never sent.
- The Insert Web Page dialog no longer shows a scroll bar.

## 1.18.6 — 2026-10-23
- **Include › Web Page**: put a web page into a message, from a web address or a page saved on
  your computer. It goes in as it looks on screen, ready to edit, in a form mail programs
  keep. Pages that build themselves with a script work too.

## 1.18.5 — 2026-10-22
- **Pasting a web page**: its floating widgets (an accessibility button, a cookie bar, a sticky
  header) are left out, and nothing in a message can float over the window (the circle under
  Send is gone).
- **Translate sends only text** to the AI: the words with their paragraphs, headings, lists and
  bold, never a pasted page's code. Back to Hebrew still puts the original back exactly.

## 1.18.4 — 2026-10-22
- **Translate turns Hebrew into English**: text you write or paste in Hebrew (or any language
  other than English) is translated to English, instead of "Your text is already in Hebrew".
  English text still translates into your own language.

## 1.18.3 — 2026-10-22
- **Attach File starts with the client's folder**: writing to a client, the menu opens with that
  company's folder and its newest files (any kind: Word, Excel, PowerPoint, PDF...), then
  Recent Items. Attaching a file to them also teaches ClassicMail their folder.
- **Recent Items shows what you just saved** in Word, Excel, PowerPoint or Acrobat (their
  shortcuts are now read the way Windows reads them).
- **Conversations**: the From column lists whoever wrote last first, then the others back in
  time, and leaves you out even when you replied.

## 1.18.2 — 2026-10-22
- **Every program can e-mail through ClassicMail**, 32-bit ones included (FactSet, Adobe and
  others), not just Word and Excel. Tools › Options shows when any part of the setup is missing
  and **Fix It** repairs all of it (one administrator prompt).
- **Attach File › Recent Items**: a document you just saved in Word or Excel heads the list.
- **Conversations**: the From column shows the original sender first, then the others in the
  order they wrote, and no longer adds your own name when you reply.

## 1.18.1 — 2026-10-21
- **Multi-day events by selecting days**: in Month view, select several days (drag across them,
  Shift+click or Shift+arrows) and type to add one all-day event over them, such as a
  conference. In Day / Week, the typing box in the all-day strip now spans the days selected.

## 1.18.0 — 2026-10-21
A code review of the whole program: about 100 fixes, plus these changes you can see.
- **Mail**
  - Pictures you download on one message are no longer allowed on the next one (or its sender).
  - AutoSave now works while you type.
  - The "permanently delete" warning goes by where the messages really are, in every window,
    and No really cancels.
  - "Messages from Sender" works from a message window, and new-mail alerts are never lost.
  - Format Painter keeps links, lines and lists.
  - Pasted tables are sent as they look on screen.
  - Moving the caret no longer redraws the whole compose window.
  - A send that gets a Google server error is not repeated straight away; it stays in the
    Outbox.
- **Calendar**
  - Changes made offline stay queued however long you're offline.
  - Remove Recurrence reaches Google.
  - Sync asks Google only for what changed, with a full read once a day.
  - Clean Up Duplicates keeps your ticks.
  - "5/1/26" means 2026, and all-day items keep their length across clock changes.
  - A resent invitation for a series says how it recurs.
- **Tasks**
  - Tasks use your Color Categories.
  - Subtasks move with their task to another account.
  - The reminder time takes any minute.
- **Contacts and Rules**
  - Closing a contact with "Yes, save" saves your edits.
  - "All mailboxes" can be chosen in Rules and Alerts, and a failed Apply no longer
    duplicates rules.
- **Faster**: address suggestions, Run Rules Now, contact sorting, splitter drags, and the
  calendar's month and week views.

## 1.17.0 — 2026-10-21 (test build)
- **The message body is one editable page**: your reply and the original below it are one
  text, as in Outlook and Word. A drag, Shift+click or Shift+arrows select across both;
  Ctrl+A is a real select-all; one undo history (Ctrl+Z / Ctrl+Y) for the whole message;
  copy, cut, delete and formatting work across both parts.
- **Pasted text keeps its formatting** (Word, Excel, other e-mails), with Office's leftovers
  cleaned out.
- Typing "- ", "* " or "1. " at the start of a line starts a list; Tab / Shift+Tab indent in
  lists and move between table cells.

## 1.16.9 — 2026-10-20
- **Meeting windows open at their normal size**, like the last one you sized, even if the last
  one was maximized (the main window still remembers being maximized).
- **The ribbon no longer squeezes its buttons** in a narrow window: "Propose New Time" keeps
  its whole label.
- **Responses**: every row is the same height.

## 1.16.8 — 2026-10-20
- **Meeting Responses**: in a meeting you organise, Responses (Attendees group, or the link on
  the "N attendees accepted" bar) lists each attendee, Required / Optional, their answer and
  any comment. **Resend Invitation** e-mails just the people who haven't answered (one, or all
  of them) a meeting request for the same meeting; they answer from their mail program and
  the meeting updates as usual.
- **Selecting text in a message**: a drag that starts in the margin or below the text now
  selects, as in Word.

## 1.16.7 — 2026-10-20
- **Attachment Reminder**: a picture in your message counts as attached, so "see the attached
  chart" over a pasted picture no longer asks whether you forgot to attach a file.

## 1.16.6 — 2026-10-20
- **Archive / Delete: the whole list settles at once.** The rows and group headers below the
  archived message (Yesterday, Last Week…) now move up together with it, instead of a second
  later.

## 1.16.5 — 2026-10-20
- **Archive (Backspace), Delete, Move and Junk are instant**: the message leaves the list at
  once and the next one shows in the reading pane, instead of after a short pause (about
  0.3 s before, under 0.1 s now).

## 1.16.4 — 2026-10-20
- **No more "This message is in Dutch"** on a short forward ("FYI") above a signature logo:
  pictures embedded in the text no longer count as words.
- **To / Cc / Bcc: Tab picks a name at once.** Names come up immediately, and typing "e" then
  Tab before the list appears takes the first name it would have shown, never a lone "e".
- **Ctrl+A selects all attachments** (also click, Ctrl+click, Shift+click); Delete removes them.
- **Esc closes a message like the X**, also with the cursor in the message text: a changed
  message asks "Do you want to save changes?".

## 1.16.3 — 2026-10-19
- **Someone else's meeting with no notes**: its Zoom / Meet details fill the window with one
  scroll bar, instead of a small box above an empty notes area.
- **Zoom's own notes read cleanly**: no "[name][1]" footnotes, and Google's redirect links open
  the real Zoom address. "More ways to join" is now "Joining instructions", as in Outlook.
  (Refreshed at the next Send/Receive.)

## 1.16.2 — 2026-10-19
- **Zoom / Meet details now reach meetings already in your calendar** (1.16.1 only filled
  them in for meetings that changed). They appear at the next Send/Receive.
- **Links open from the notes and the Zoom / Meet details**; a click elsewhere in the notes
  edits them. A Location with a web address gets a **Join** button.

## 1.16.1 — 2026-10-19
- **Meetings from Google Calendar users show their Zoom / Meet details**: the link, dial-in
  numbers and SIP appear above the notes (with clickable links), as in Outlook. AI Review's
  meeting lookups see them too. (Fills in at the next Send/Receive.)
- **Notes written in Outlook or Google Calendar read as text**, not as raw HTML.
- **Replies to your meetings** ("Accepted: …") show who accepted, tentatively accepted or
  declined, without Accept / Tentative / Decline buttons; a cancellation says so.
- The recurrence line shows the series' first date and usual times, also on a moved occurrence.
- **Send as Attachment works for every Windows user on the PC**: the shared mail DLL lives in
  ProgramData and each user's ClassicMail gets its own messages. After installing, click
  Tools › Options › Other › Set Up for Office… once (one administrator prompt).

## 1.16.0 — 2026-10-19
- **Send as Attachment from Word, Excel and other programs opens a ClassicMail message**
  with the document attached (and any recipients, subject and text the program gives).
  Turn it on in Tools › Options › Other. Windows asks once for administrator permission,
  because Office follows the computer's mail program setting. ClassicMail starts if it isn't
  running. Turning it off puts back the previous mail program (Outlook).
- **Pictures don't go to the AI**: Translate and AI Review on a message with a pasted
  screenshot no longer stall; the pictures stay where they were.

## 1.15.0 — 2026-10-19
- **AI Review can look things up** on this computer: free time in your calendar
  (`{a free slot Tuesday afternoon next week}`, also in the recipient's time zone), your
  meetings and their details (Zoom links, addresses), tasks and due dates, and exact time-zone
  conversions. Only your primary calendar counts as busy unless you name another one.
  Tools › Options › Calendar: usual meeting length (45 min) and gap between meetings (15 min).
- **Drop files from Explorer onto a message** to attach them.
- **The Translate bar** judges a message's language by its opening, so a short Hebrew e-mail
  over an English signature / disclaimer is offered a translation.
- Dragging names between To / Cc / Bcc now works in the real compose window too.

## 1.14.0 — 2026-10-19
- **Drag names between To, Cc and Bcc** to move them (Ctrl+drag copies), or from a message you
  read into one you write.
- **Drag an attachment** onto an Explorer folder, the desktop or another program.
- **Translate** leaves your signature as it is and translates the subject too (keeping RE: / FW:);
  Back to <language> puts both back.

## 1.13.0 — 2026-10-19
- **Resize table columns and rows with the mouse**: drag a column's or row's line, in your reply
  and in tables of a forwarded / quoted e-mail.
- **Even Out** (Format Text › Table): columns after the first as wide as the widest cell, rows
  after the header as tall as the tallest — a tidy table in one click.
- **The Font group follows the cursor**, like Word: font, size and Bold / Italic / Underline /
  Strikethrough of the text at the cursor or across the selection, blank when it is mixed. After
  Ctrl+A, a font or size changes the whole message, the original included.

## 1.12.0 — 2026-10-18
- **Tables in a forwarded or quoted e-mail can be edited as tables** (e.g. a table Claude wrote):
  with the cursor in one, Format Text › Table works on it — Shading, Borders, Pen Color, insert /
  delete rows and columns, Merge / Split Cells, Header Row, Cell Alignment.

## 1.11.0 — 2026-10-18
- **Attachment folders are learnt per company** (the other party's e-mail domain): Save As, Save
  All Attachments and Attach File › Browse open in the folder used before for that company — its
  usual folder when that has over 75% of the saves (Tools › Options › Preferences), otherwise the
  folder they all share (Clients\Acme for …\Acme and …\Acme\Results\Q3).
- **Tables like Word** (Format Text › Table): insert, Shading, Borders (all / outside / inside /
  one side / none, in a pen colour and width), insert / delete rows and columns (new ones take
  their neighbour's formatting), Merge / Split Cells, Header Row, Cell Alignment.
- **Font, Font Size and Format Painter work in the original below your reply** (an old e-mail
  reused as a template); Format Painter copies and paints between reply and original, and is on
  Format Text too.

## 1.10.0 — 2026-10-18
- **Names in From / To / Cc / Bcc can be cut, copied and pasted**, as in Outlook — in e-mails you
  read and in ones you write. Click a name (Ctrl/Shift+click, drag or Shift+arrows for more,
  Ctrl+A for all of them), then Ctrl+X / C / V or right-click. They copy as `Name <email>; …`.
- **AI Review uses `{curly braces}` only** — square brackets are ordinary text again. Instructions
  typed into the original below your reply (an old e-mail reused as a template) now work too.
- **Ctrl+A selects the whole message**, your reply and the original below it, wherever the
  cursor is; Copy, Cut, Delete, typing and formatting then act on all of it.
- **Paste files to attach them:** copy files in Explorer, then Ctrl+V (or Paste) in a message.
- **Typing fixes, like Word:** Caps Lock left on ("eLI" → "Eli", Caps Lock goes off); the Hebrew
  keyboard left on for an English word ("Eךן" → "Eli", the keyboard switches to English); an empty
  line runs right-to-left while a Hebrew keyboard is on.
- The ribbon's Paste pastes into the To / Cc / Subject box that has the cursor, not the body.

## 1.9.0 — 2026-10-14
- **Rules can be kept in Gmail** (Rules and Alerts › *Also keep these rules in Gmail*): rules Gmail
  can do become Gmail filters, so they work while ClassicMail is closed and on the phone; the rest
  show why they stay local. Needs Gmail's "manage settings" permission — sign in to each mailbox
  once more (Tools › Account Settings).
- **Attachment preview:** click an attachment to see it in the Reading Pane (PDF, pictures,
  Excel / CSV, Word, text, HTML, attached e-mails); "Message" goes back, double-click opens it.
- **Opening an e-mail is instant**, like New / Reply.
- Behind the scenes: code checks and formatting tools (they found an Edit-menu Undo / Redo bug, now
  fixed), tests on a fixed date.

## 1.8.0 — 2026-10-14
- **CRM link removed.** The CRM tab in Options, the CRM line above e-mails and Log to CRM are
  gone; the CRM now matches e-mail against Gmail itself. (The CRM token stays in Windows
  Credential Manager, "ClassicMail" / `crm.token`, until removed there.)
- **"Don't ask me again" on warnings.** Delete / empty / discard / no subject / forgotten
  attachment / plain text and similar warnings can be switched off from the box itself.
  Tools › Options › Other › **Show All Warnings Again** turns them all back on. Choices such as
  "Save changes?" and "this occurrence or the series?" still ask every time.
- **Search: results only Gmail found are tagged "Gmail"**, with **Show only these** / Show all
  results; in a folder search, the count covers only what is in that folder.

## 1.7.0 — 2026-10-14
- **Windows open finished, not blank.** A compose window is kept ready in the background, so
  New / Reply / Forward appear at once; every other window opens ~0.3 s sooner.
- Open conversations: the subject is indented like the name.
- Options › Appearance: narrower; the Modern / Classic choice is gone (always the new Outlook's
  font — Font and Size cover the rest).

## 1.6.1 — 2026-10-13
- Messages can be dragged onto **Archive** in the folder pane (archives them).

## 1.6.0 — 2026-10-13
- **Calendar right-click menu like Outlook's:** Quick Print; Accept / Tentative / Decline with
  Edit the Response / Send Now / Don't Send; Propose New Time (sent to the organizer as the
  response note); Reply (to everyone, with Reply to One in the message window) and Forward
  (Ctrl+R / Ctrl+F); Private; Show As; Categorize; Delete. The meeting window gets the same
  Respond group, and its Print prints the item.

## 1.5.1 — 2026-10-13
- Options › Mail: one choice for what to keep on this computer — the last N days or all mail —
  with attachments following it, and a size estimate for each.

## 1.5.0 — 2026-10-13
- **Download all mail** (optional): every message body in the background, optionally attachments
  too; progress in the status bar.
- **Gmail is searched alongside the local search**; its extra matches join the list.
- In a reply, Delete at the end of your text pulls the original up.
- Faster folder list; AI Review no longer treats links as instructions; mail merge builds each
  message when it is sent and keeps its pace after a restart; notes tables keep a blank "#".

## 1.4.3 — 2026-10-13
- Meeting notes: a table can be switched between list and grid at any time (the "Table header:"
  line keeps what is needed).

## 1.4.2 — 2026-10-13
- **Zoom:** Ctrl + mouse wheel in messages; View › Increase / Decrease Font Size.
- Meeting notes: tidy a pasted table as a list (phone) or a grid (desktop); grids show as real
  tables in ClassicMail.

## 1.4.1 — 2026-10-13
- Translate also translates names and the signature (Kenny → קני).

## 1.4.0 — 2026-10-13
- Meeting notes: phone-friendly tables.
- Compose ribbon: AI Review in its own AI group; Translate next to Spelling.
- Translate works in any message: into the original's language, else the default language set in
  Options › AI.

## 1.3.0 — 2026-10-13
- **Reply is Reply All**, with a Reply to One toggle in the message window (Ctrl+R).
- **Reply with AI** (drafts a reply), **Translate my reply** (and back), **AI Review**
  (fills in [instructions] in your text).

## 1.2.1 — 2026-10-13
- **Mail merge:** one personalised message per row of an Excel / CSV list ({Column} fields),
  sent paced through the Outbox, saved as drafts, or as a test to yourself.
- Attachments: large ones open; Open With…
- Hebrew mail labelled with the wrong character set shows correctly; no sideways scroll bar on
  Hebrew lists; language detection ignores links.
- Fixed a Reminders window that could be left orphaned.

## 1.1.0 — 2026-10-11
- **Show as Conversations**, with replies from Sent Items inside the thread.
- **Fast first download over Gmail IMAP** (read-only, resumable).
- Single-line list layout with a column chooser; drag mail to folders; reorder folders and
  Favorite Folders.
- **Archive** folder per mailbox (Gmail-archived mail; Backspace archives).
- **Outlook archives (.pst):** open read-only in the folder pane, search, reply / forward.
- **Calendar:** My / Shared / Other calendars, side-by-side view, colour categories (synced to
  Google), Clean Up Calendar for duplicates.
- **Translate bar** for foreign-language mail (Gemini / Claude), Options › AI.
- **Undo / Redo** (Ctrl+Z / Ctrl+Y) across mail, calendar, contacts and tasks.
- Contacts sort options; custom UI font, size and weight.
- Fixed a crash on Hebrew text.

## 1.0.0 — 2026-10-11
- First version: Mail, Calendar, Contacts, Tasks and the To-Do Bar for Google Workspace /
  Gmail, in the Outlook 2007 style — compose with signatures and send-as, Outlook-style Rules,
  reminders, desktop alerts, tray icon, close to tray and start with Windows.
