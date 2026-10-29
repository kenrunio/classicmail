use crate::db::Migrations;

/// Outlook-style rules (design §7). `conditions` / `exceptions` / `actions` hold JSON arrays of the
/// tagged enums in `rules::model`. `account_id` NULL = the rule applies to every mailbox.
/// `stop_processing` mirrors a `stopProcessing` action (kept in sync on save).
pub const MIGRATIONS: Migrations = &[
    (
        1,
        r#"
CREATE TABLE rules (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  position INTEGER NOT NULL DEFAULT 0,
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
  conditions TEXT NOT NULL DEFAULT '[]',
  exceptions TEXT NOT NULL DEFAULT '[]',
  actions TEXT NOT NULL DEFAULT '[]',
  stop_processing INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX rules_position ON rules(position, id);
"#,
    ),
    // the Gmail filters ClassicMail made for rules (rules::gmail_filters); `spec` = the filter JSON
    (
        2,
        r#"
CREATE TABLE rule_filters (
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  rule_id INTEGER NOT NULL,
  filter_id TEXT NOT NULL,
  spec TEXT NOT NULL,
  PRIMARY KEY (account_id, filter_id)
);
"#,
    ),
];
