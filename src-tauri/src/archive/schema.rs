//! Outlook archive (.pst) index tables. Bodies are read from the PST on demand; only headers (and,
//! optionally, message text) are indexed here.

pub const MIGRATIONS: crate::db::Migrations = &[(
    1,
    r#"
CREATE TABLE archives (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  store_name TEXT NOT NULL DEFAULT '',
  format TEXT NOT NULL DEFAULT '',
  file_size INTEGER NOT NULL DEFAULT 0,
  file_mtime INTEGER NOT NULL DEFAULT 0,
  -- 'pending' | 'indexing' | 'ready' | 'error' | 'missing' | 'locked'
  status TEXT NOT NULL DEFAULT 'pending',
  error TEXT,
  item_count INTEGER NOT NULL DEFAULT 0,
  indexed_at INTEGER,
  index_ms INTEGER,
  -- "Index message text for search": 0 off, 1 wanted / running, 2 done
  index_bodies INTEGER NOT NULL DEFAULT 0,
  bodies_indexed INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  added_at INTEGER NOT NULL
);
CREATE TABLE archive_folders (
  id INTEGER PRIMARY KEY,
  archive_id INTEGER NOT NULL REFERENCES archives(id) ON DELETE CASCADE,
  node_id INTEGER NOT NULL,
  parent_id INTEGER REFERENCES archive_folders(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  container_class TEXT NOT NULL DEFAULT '',
  -- 'mail' (shown) | 'calendar' | 'contacts' | 'tasks' | 'notes' | 'journal' (not shown in v1)
  kind TEXT NOT NULL DEFAULT 'mail',
  content_count INTEGER NOT NULL DEFAULT 0,
  -- rows indexed so far (resume point) and whether the folder is complete
  indexed INTEGER NOT NULL DEFAULT 0,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  UNIQUE (archive_id, node_id)
);
CREATE TABLE archive_messages (
  id INTEGER PRIMARY KEY,
  archive_id INTEGER NOT NULL REFERENCES archives(id) ON DELETE CASCADE,
  folder_id INTEGER NOT NULL REFERENCES archive_folders(id) ON DELETE CASCADE,
  node_id INTEGER NOT NULL,
  message_class TEXT NOT NULL DEFAULT '',
  subject TEXT NOT NULL DEFAULT '',
  subject_norm TEXT NOT NULL DEFAULT '',
  from_name TEXT NOT NULL DEFAULT '',
  from_email TEXT NOT NULL DEFAULT '',
  to_display TEXT NOT NULL DEFAULT '',
  cc_display TEXT NOT NULL DEFAULT '',
  date INTEGER NOT NULL DEFAULT 0,
  sent_date INTEGER,
  size INTEGER NOT NULL DEFAULT 0,
  is_read INTEGER NOT NULL DEFAULT 1,
  flag_status TEXT,
  has_attachments INTEGER NOT NULL DEFAULT 0,
  importance INTEGER NOT NULL DEFAULT 1,
  conversation_topic TEXT NOT NULL DEFAULT '',
  last_verb TEXT,
  body_indexed INTEGER NOT NULL DEFAULT 0,
  UNIQUE (archive_id, node_id)
);
CREATE INDEX archive_messages_folder_date ON archive_messages(folder_id, date DESC, id DESC);
CREATE INDEX archive_messages_archive ON archive_messages(archive_id, date DESC);
CREATE INDEX archive_messages_body ON archive_messages(archive_id, body_indexed);
-- headers (and optional message text) for search; rowid = archive_messages.id
CREATE VIRTUAL TABLE archive_fts USING fts5(
  subject, from_text, to_text, body,
  content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2'
);
"#,
)];
