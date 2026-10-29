use crate::db::Migrations;

/// Translation cache: one row per (target language, source text). `src_hash` = first 16 bytes of
/// SHA-256 of the trimmed source text (hex); `message_id` = the message it was first seen in.
pub const MIGRATIONS: Migrations = &[(
    1,
    r#"
CREATE TABLE ai_translations (
  target TEXT NOT NULL,
  src_hash TEXT NOT NULL,
  translation TEXT NOT NULL,
  message_id INTEGER,
  model TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (target, src_hash)
);
CREATE INDEX ai_translations_msg ON ai_translations(message_id);
"#,
)];
