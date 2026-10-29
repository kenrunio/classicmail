use crate::db::Migrations;

pub const MIGRATIONS: Migrations = &[(
    1,
    r#"
CREATE TABLE task_lists (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  google_id TEXT,                      -- NULL until the create has been pushed
  title TEXT NOT NULL,
  updated INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  is_default INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX task_lists_gid ON task_lists(account_id, google_id);

CREATE TABLE tasks (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  list_id INTEGER NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
  google_id TEXT,
  parent_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  position TEXT NOT NULL DEFAULT '',   -- Google's 20-digit position among siblings
  title TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  due TEXT,                            -- YYYY-MM-DD (Google keeps the date only)
  status TEXT NOT NULL DEFAULT 'needsAction',  -- needsAction | completed
  completed_at INTEGER,
  updated INTEGER NOT NULL DEFAULT 0,
  etag TEXT,
  hidden INTEGER NOT NULL DEFAULT 0,
  -- Outlook-only fields, kept locally and never overwritten by sync
  start_date TEXT,
  outlook_status TEXT NOT NULL DEFAULT 'notStarted', -- notStarted | inProgress | waiting | deferred
  importance INTEGER NOT NULL DEFAULT 1,             -- 0 low, 1 normal, 2 high
  percent INTEGER NOT NULL DEFAULT 0,
  reminder_at INTEGER,
  categories TEXT NOT NULL DEFAULT '[]',             -- JSON array of category names
  private INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX tasks_gid ON tasks(account_id, google_id);
CREATE INDEX tasks_list ON tasks(list_id, position);
CREATE INDEX tasks_due ON tasks(status, due);
"#,
)];
