use crate::db::Migrations;

pub const MIGRATIONS: Migrations = &[
    (
        1,
        r#"
CREATE TABLE contact_groups (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  resource_name TEXT NOT NULL,
  name TEXT NOT NULL,
  group_type TEXT NOT NULL DEFAULT 'USER_CONTACT_GROUP',
  UNIQUE (account_id, resource_name)
);

CREATE TABLE contacts (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  resource_name TEXT,                       -- NULL until the create reaches Google
  etag TEXT,
  display_name TEXT NOT NULL DEFAULT '',
  given_name TEXT NOT NULL DEFAULT '',
  middle_name TEXT NOT NULL DEFAULT '',
  family_name TEXT NOT NULL DEFAULT '',
  prefix TEXT NOT NULL DEFAULT '',
  suffix TEXT NOT NULL DEFAULT '',
  nickname TEXT NOT NULL DEFAULT '',
  file_as TEXT NOT NULL DEFAULT '',
  company TEXT NOT NULL DEFAULT '',
  job_title TEXT NOT NULL DEFAULT '',
  department TEXT NOT NULL DEFAULT '',
  office TEXT NOT NULL DEFAULT '',
  profession TEXT NOT NULL DEFAULT '',
  manager TEXT NOT NULL DEFAULT '',
  assistant TEXT NOT NULL DEFAULT '',
  spouse TEXT NOT NULL DEFAULT '',
  emails TEXT NOT NULL DEFAULT '[]',        -- [{value, type, displayName}]
  phones TEXT NOT NULL DEFAULT '[]',        -- [{type, value}]  (Google types: work, home, mobile, workFax...)
  addresses TEXT NOT NULL DEFAULT '[]',     -- [{type, street, city, region, postalCode, country}]
  mailing_address TEXT NOT NULL DEFAULT '', -- type of the address marked "This is the mailing address"
  urls TEXT NOT NULL DEFAULT '[]',          -- [{type, value}]
  ims TEXT NOT NULL DEFAULT '[]',           -- [{protocol, username}]
  birthday TEXT,                            -- YYYY-MM-DD (or --MM-DD without a year)
  anniversary TEXT,
  notes TEXT NOT NULL DEFAULT '',
  photo_url TEXT,
  groups TEXT NOT NULL DEFAULT '[]',        -- contact group resource names (user groups only)
  starred INTEGER NOT NULL DEFAULT 0,
  updated INTEGER NOT NULL DEFAULT 0,
  dirty INTEGER NOT NULL DEFAULT 0,         -- local changes not yet pushed
  local_rev INTEGER NOT NULL DEFAULT 0,     -- bumped on each local edit (push race detection)
  UNIQUE (account_id, resource_name)
);
CREATE INDEX contacts_file_as ON contacts(account_id, file_as COLLATE NOCASE);

CREATE VIRTUAL TABLE contacts_fts USING fts5(
  name, company, emails, phones, other,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER contacts_fts_ai AFTER INSERT ON contacts BEGIN
  INSERT INTO contacts_fts(rowid, name, company, emails, phones, other) VALUES (
    new.id,
    new.display_name || ' ' || new.file_as || ' ' || new.nickname || ' ' || new.given_name || ' ' || new.family_name,
    new.company || ' ' || new.job_title || ' ' || new.department,
    coalesce((SELECT group_concat(value ->> 'value', ' ') FROM json_each(new.emails)), ''),
    coalesce((SELECT group_concat(value ->> 'value', ' ') FROM json_each(new.phones)), ''),
    new.notes || ' ' || coalesce((SELECT group_concat(coalesce(value ->> 'street', '') || ' ' || coalesce(value ->> 'city', '') || ' ' || coalesce(value ->> 'country', ''), ' ') FROM json_each(new.addresses)), '')
  );
END;
CREATE TRIGGER contacts_fts_ad AFTER DELETE ON contacts BEGIN
  DELETE FROM contacts_fts WHERE rowid = old.id;
END;
CREATE TRIGGER contacts_fts_au AFTER UPDATE ON contacts BEGIN
  DELETE FROM contacts_fts WHERE rowid = old.id;
  INSERT INTO contacts_fts(rowid, name, company, emails, phones, other) VALUES (
    new.id,
    new.display_name || ' ' || new.file_as || ' ' || new.nickname || ' ' || new.given_name || ' ' || new.family_name,
    new.company || ' ' || new.job_title || ' ' || new.department,
    coalesce((SELECT group_concat(value ->> 'value', ' ') FROM json_each(new.emails)), ''),
    coalesce((SELECT group_concat(value ->> 'value', ' ') FROM json_each(new.phones)), ''),
    new.notes || ' ' || coalesce((SELECT group_concat(coalesce(value ->> 'street', '') || ' ' || coalesce(value ->> 'city', '') || ' ' || coalesce(value ->> 'country', ''), ' ') FROM json_each(new.addresses)), '')
  );
END;

-- Google "other contacts": people you e-mailed but never saved (address autocomplete only).
CREATE TABLE other_contacts (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  resource_name TEXT NOT NULL,
  name TEXT NOT NULL DEFAULT '',
  email TEXT NOT NULL,
  UNIQUE (account_id, resource_name, email)
);
CREATE INDEX other_contacts_email ON other_contacts(email COLLATE NOCASE);
"#,
    ),
    (
        2,
        // "Date Added": Google doesn't expose a creation time, so remember when a contact first
        // appeared here. Existing rows get the best timestamp we have (their last update).
        r#"
ALTER TABLE contacts ADD COLUMN first_seen_at INTEGER NOT NULL DEFAULT 0;
UPDATE contacts SET first_seen_at = CASE WHEN updated > 0 THEN updated ELSE CAST(strftime('%s', 'now') AS INTEGER) * 1000 END;
"#,
    ),
];
