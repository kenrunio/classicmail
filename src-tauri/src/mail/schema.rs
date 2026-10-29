use crate::db::Migrations;

/// Mail tables. Folder membership mirrors Gmail labels (`message_folders`); `messages.label_ids`
/// keeps the raw label ids as last seen from Gmail (space separated) as the source of truth.
pub const MIGRATIONS: Migrations = &[
    (
        1,
        r#"
CREATE TABLE folders (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  gmail_label_id TEXT,                 -- NULL for local-only folders (Outbox)
  name TEXT NOT NULL,                  -- display name of this level ("Orion")
  path TEXT NOT NULL,                  -- full Gmail label name ("Clients/Orion")
  parent_id INTEGER REFERENCES folders(id) ON DELETE SET NULL,
  role TEXT NOT NULL DEFAULT 'user',   -- inbox|sent|drafts|trash|spam|outbox|all|starred|important|user
  hidden INTEGER NOT NULL DEFAULT 0,
  unread_count INTEGER NOT NULL DEFAULT 0,
  total_count INTEGER NOT NULL DEFAULT 0,
  color TEXT,
  position INTEGER NOT NULL DEFAULT 0,
  UNIQUE(account_id, gmail_label_id)
);
CREATE INDEX folders_account ON folders(account_id);

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  gmail_id TEXT NOT NULL,
  thread_id TEXT NOT NULL DEFAULT '',
  history_id INTEGER,
  internal_date INTEGER NOT NULL,       -- received (ms)
  sent_date INTEGER,                    -- Date: header (ms)
  subject TEXT NOT NULL DEFAULT '',
  from_name TEXT NOT NULL DEFAULT '',
  from_email TEXT NOT NULL DEFAULT '',
  to_json TEXT NOT NULL DEFAULT '[]',   -- [{"name","email"}]
  cc_json TEXT NOT NULL DEFAULT '[]',
  bcc_json TEXT NOT NULL DEFAULT '[]',
  reply_to_json TEXT NOT NULL DEFAULT '[]',
  snippet TEXT NOT NULL DEFAULT '',
  label_ids TEXT NOT NULL DEFAULT '',   -- Gmail label ids, space separated
  is_read INTEGER NOT NULL DEFAULT 0,
  is_flagged INTEGER NOT NULL DEFAULT 0,          -- Gmail STARRED
  flag_due TEXT,                                  -- local Outlook follow-up date YYYY-MM-DD
  flag_status TEXT,                               -- NULL | 'flagged' | 'complete' (local)
  is_draft INTEGER NOT NULL DEFAULT 0,
  gmail_draft_id TEXT,
  has_attachments INTEGER NOT NULL DEFAULT 0,
  importance INTEGER NOT NULL DEFAULT 1,          -- 0 low, 1 normal, 2 high
  size INTEGER NOT NULL DEFAULT 0,
  message_id_header TEXT,
  in_reply_to TEXT,
  references_header TEXT,
  last_verb TEXT,                                 -- 'replied' | 'repliedAll' | 'forwarded' (local)
  last_verb_at INTEGER,
  is_invite INTEGER NOT NULL DEFAULT 0,           -- has a text/calendar METHOD:REQUEST part
  ical_uid TEXT,
  body_state INTEGER NOT NULL DEFAULT 0,          -- 0 headers only, 1 body fetched
  UNIQUE(account_id, gmail_id)
);
CREATE INDEX messages_date ON messages(account_id, internal_date DESC);
CREATE INDEX messages_thread ON messages(account_id, thread_id);
CREATE INDEX messages_flagged ON messages(is_flagged) WHERE is_flagged = 1;

CREATE TABLE message_folders (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  PRIMARY KEY (folder_id, message_id)
);
CREATE INDEX message_folders_msg ON message_folders(message_id);

CREATE TABLE message_bodies (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  html TEXT,                 -- original HTML (sanitised when served)
  text TEXT,                 -- plain text (original, or derived from HTML)
  headers_json TEXT,         -- all headers [{name,value}] for "Message Options / Internet headers"
  calendar_ics TEXT,         -- text/calendar part, if any
  fetched_at INTEGER NOT NULL
);

CREATE TABLE attachments (
  id INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  part_id TEXT,
  gmail_attachment_id TEXT,
  filename TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  content_id TEXT,           -- for cid: inline images (without <>)
  is_inline INTEGER NOT NULL DEFAULT 0,
  cached_path TEXT           -- downloaded copy in the data dir
);
CREATE INDEX attachments_msg ON attachments(message_id);

-- full-text index, rowid = messages.id
CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_text, to_text, body,
  content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2'
);

-- messages waiting to be sent (Outlook's Outbox)
CREATE TABLE outbox (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  compose_json TEXT NOT NULL,      -- the composed message as the UI sent it
  raw BLOB,                        -- built RFC 822 bytes
  thread_id TEXT,
  reply_to_message_id INTEGER,     -- local messages.id being replied to / forwarded
  verb TEXT,                       -- 'replied' | 'repliedAll' | 'forwarded'
  gmail_draft_id TEXT,             -- draft to delete after sending
  status TEXT NOT NULL DEFAULT 'queued',   -- queued | sending | failed
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  send_at INTEGER NOT NULL,        -- deferred delivery ("Do not deliver before")
  created_at INTEGER NOT NULL
);

CREATE TABLE signatures (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  html TEXT NOT NULL DEFAULT ''
);
-- per account: which signature for new messages / replies (settings-like)
CREATE TABLE signature_defaults (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  new_signature_id INTEGER REFERENCES signatures(id) ON DELETE SET NULL,
  reply_signature_id INTEGER REFERENCES signatures(id) ON DELETE SET NULL
);

-- Gmail "send as" identities per account (From: picker)
CREATE TABLE send_as (
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL DEFAULT '',
  is_default INTEGER NOT NULL DEFAULT 0,
  signature TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (account_id, email)
);

-- senders whose pictures load automatically ("Add Sender to Safe Senders List")
CREATE TABLE safe_senders (
  entry TEXT PRIMARY KEY            -- address or @domain
);

-- Outlook's auto-complete nickname cache
CREATE TABLE recipient_cache (
  email TEXT PRIMARY KEY,
  name TEXT NOT NULL DEFAULT '',
  use_count INTEGER NOT NULL DEFAULT 0,
  last_used INTEGER NOT NULL DEFAULT 0
);
"#,
    ),
    // v2 (mail-core): list/sort helpers, resync generations, trash/spam location, flag completion time
    (
        2,
        r#"
ALTER TABLE message_folders ADD COLUMN msg_date INTEGER NOT NULL DEFAULT 0;
CREATE INDEX message_folders_date ON message_folders(folder_id, msg_date DESC, message_id DESC);
ALTER TABLE messages ADD COLUMN subject_norm TEXT NOT NULL DEFAULT '';   -- subject without RE:/FW:
ALTER TABLE messages ADD COLUMN to_display TEXT NOT NULL DEFAULT '';     -- "Sarah Cohen; Bob" (To column)
ALTER TABLE messages ADD COLUMN sync_gen INTEGER NOT NULL DEFAULT 0;     -- full-resync generation
ALTER TABLE messages ADD COLUMN location INTEGER NOT NULL DEFAULT 0;     -- 0 normal, 1 trash, 2 spam
ALTER TABLE messages ADD COLUMN flag_completed_at INTEGER;
CREATE INDEX messages_unread ON messages(account_id, is_read, internal_date DESC);
CREATE INDEX messages_body ON messages(account_id, body_state, internal_date DESC);
CREATE INDEX messages_msgid ON messages(message_id_header);
"#,
    ),
    (
        3,
        r#"
-- "Download Pictures" clicked for this message: remembered, pictures load from then on
ALTER TABLE messages ADD COLUMN allow_remote INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        4,
        r#"
-- Show as Conversations: a thread's messages newest first (replaces the (account_id, thread_id) index)
CREATE INDEX messages_thread_date ON messages(account_id, thread_id, internal_date DESC);
DROP INDEX IF EXISTS messages_thread;
"#,
    ),
    (
        5,
        r#"
-- senders whose pictures were downloaded before (pictures then load automatically)
CREATE INDEX messages_allow_remote_from ON messages(lower(from_email)) WHERE allow_remote = 1;
"#,
    ),
    (
        6,
        r#"
-- UTF-8 Hebrew bodies that were decoded as windows-1255 (Outlook's label) before the charset fix:
-- "׳™" / "׳•" (geresh + ™ / •) is what UTF-8 י / ו turns into, and never occurs in real text.
-- Drop the cached body so it is fetched again (and re-indexed) with the fixed decoding.
UPDATE messages SET body_state = 0 WHERE id IN (
  SELECT message_id FROM message_bodies
  WHERE coalesce(text, '') || coalesce(html, '') LIKE '%׳™%' OR coalesce(text, '') || coalesce(html, '') LIKE '%׳•%'
);
DELETE FROM message_bodies
WHERE coalesce(text, '') || coalesce(html, '') LIKE '%׳™%' OR coalesce(text, '') || coalesce(html, '') LIKE '%׳•%';
"#,
    ),
    (
        7,
        r#"
-- mail.listFolders' counts (All Mail, Unread Mail, For Follow Up, Large Mail) in one pass over this
-- index alone instead of four scans of the messages table (~100 ms → ~6 ms on 50k messages)
CREATE INDEX messages_counts ON messages(account_id, location, is_draft, is_read, is_flagged, flag_status, size);
"#,
    ),
    (
        8,
        r#"
-- paced batches (mail merge): rows of one batch go out at least pace_ms apart, even after the app
-- was closed; their raw is NULL (built when sent, so a 300-row merge doesn't hold 300 copies of
-- its attachments) and size is the estimate shown in the Outbox
ALTER TABLE outbox ADD COLUMN batch TEXT;
ALTER TABLE outbox ADD COLUMN pace_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE outbox ADD COLUMN size INTEGER;
"#,
    ),
    (
        9,
        r#"
-- where attachments were saved, per correspondent's e-mail domain (src/mail/save_folders.rs):
-- Save As / Save All open in the folder used before for that domain
CREATE TABLE attachment_folders (
  domain TEXT NOT NULL,
  folder TEXT NOT NULL COLLATE NOCASE,
  count INTEGER NOT NULL DEFAULT 0,
  last_used INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (domain, folder)
);
"#,
    ),
];
