use crate::db::Migrations;

pub const MIGRATIONS: Migrations = &[
    (
        1,
        r#"
CREATE TABLE calendars (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  gcal_id TEXT NOT NULL,
  summary TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  color TEXT NOT NULL DEFAULT '#9fc6e7',
  fg_color TEXT NOT NULL DEFAULT '#000000',
  selected INTEGER NOT NULL DEFAULT 1,
  access_role TEXT NOT NULL DEFAULT 'owner',
  is_primary INTEGER NOT NULL DEFAULT 0,
  time_zone TEXT NOT NULL DEFAULT 'UTC',
  default_reminders TEXT NOT NULL DEFAULT '[]',
  position INTEGER NOT NULL DEFAULT 0,
  UNIQUE(account_id, gcal_id)
);

CREATE TABLE events (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
  gcal_id TEXT,                         -- NULL until the event exists on the server
  recurring_event_id TEXT,              -- gcal id of the series master (instances)
  is_master INTEGER NOT NULL DEFAULT 0, -- recurring series master (never listed, holds recurrence)
  parent_local_id INTEGER,              -- local placeholder instance of a not-yet-pushed series
  original_start_ms INTEGER,
  ical_uid TEXT,
  summary TEXT NOT NULL DEFAULT '',
  location TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  start_ms INTEGER NOT NULL,
  end_ms INTEGER NOT NULL,
  all_day INTEGER NOT NULL DEFAULT 0,
  start_date TEXT,
  end_date TEXT,
  time_zone TEXT,
  status TEXT NOT NULL DEFAULT 'confirmed',
  organizer_email TEXT,
  organizer_name TEXT,
  organizer_self INTEGER NOT NULL DEFAULT 1,
  attendees TEXT NOT NULL DEFAULT '[]',
  reminders TEXT,                       -- Google {useDefault, overrides}
  reminder_min INTEGER,                 -- effective reminder (minutes before start), NULL = none
  recurrence TEXT,                      -- JSON array of RRULE/EXDATE lines (masters)
  transparency TEXT NOT NULL DEFAULT 'opaque',
  visibility TEXT NOT NULL DEFAULT 'default',
  show_as TEXT,                         -- our extension: busy/free/tentative/oof
  importance TEXT,                      -- our extension: low/normal/high
  event_type TEXT NOT NULL DEFAULT 'default',
  html_link TEXT,
  etag TEXT,
  updated_ms INTEGER,
  my_response TEXT,
  color_id TEXT,
  local_dirty INTEGER NOT NULL DEFAULT 0,
  UNIQUE(calendar_id, gcal_id)
);
CREATE INDEX events_range ON events(calendar_id, start_ms, end_ms);
CREATE INDEX events_ical ON events(account_id, ical_uid);
CREATE INDEX events_series ON events(calendar_id, recurring_event_id);

-- Reminder dismiss / snooze, per occurrence (event id + its start, so moving an item re-arms it).
CREATE TABLE reminder_state (
  event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  start_ms INTEGER NOT NULL,
  dismissed INTEGER NOT NULL DEFAULT 0,
  snooze_until INTEGER,
  PRIMARY KEY (event_id, start_ms)
);
"#,
    ),
    (
        2,
        r#"
-- Dismiss / snooze for task reminders (tasks.reminder_at), keyed by task + reminder time.
CREATE TABLE task_reminder_state (
  task_id INTEGER NOT NULL,
  reminder_at INTEGER NOT NULL,
  dismissed INTEGER NOT NULL DEFAULT 0,
  snooze_until INTEGER,
  PRIMARY KEY (task_id, reminder_at)
);
"#,
    ),
    (
        3,
        r#"
-- Outlook colour categories (comma-separated names) and Google's `created` (Clean Up keeps the oldest).
ALTER TABLE events ADD COLUMN categories TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN created_ms INTEGER;
"#,
    ),
    (
        4,
        r#"
-- A meeting's video-call details (Google conferenceData as text: calendar/conference.rs).
ALTER TABLE events ADD COLUMN conference TEXT NOT NULL DEFAULT '';
"#,
    ),
    (
        5,
        r#"
-- Your own subject for a meeting someone else organised: the organizer's, while `summary` is yours.
ALTER TABLE events ADD COLUMN summary_orig TEXT;
"#,
    ),
];
