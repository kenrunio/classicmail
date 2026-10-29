//! SQLite access. One writer connection + a small pool of readers (WAL mode lets them run while a
//! sync is writing). All work runs on the blocking thread pool via `read` / `write`.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};

pub mod util;

pub type Migrations = &'static [(i64, &'static str)];

#[derive(Clone)]
pub struct Db(Arc<Inner>);

struct Inner {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

fn configure(c: &Connection) -> rusqlite::Result<()> {
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;")?;
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let writer = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        configure(&writer)?;
        let db = Db(Arc::new(Inner { path: path.to_owned(), writer: Mutex::new(writer), readers: Mutex::new(Vec::new()) }));
        db.migrate_all()?;
        Ok(db)
    }

    /// In-memory-ish database for tests (a temp file, so readers see the writer's data).
    pub fn open_temp() -> anyhow::Result<(Db, tempdir::TempDirGuard)> {
        let guard = tempdir::TempDirGuard::new()?;
        let db = Db::open(&guard.path().join("test.db"))?;
        Ok((db, guard))
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    fn reader(&self) -> anyhow::Result<Connection> {
        if let Some(c) = self.0.readers.lock().pop() {
            return Ok(c);
        }
        let c = Connection::open_with_flags(
            &self.0.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
        )?;
        configure(&c)?;
        Ok(c)
    }

    fn give_back(&self, c: Connection) {
        let mut r = self.0.readers.lock();
        if r.len() < 4 {
            r.push(c);
        }
    }

    /// Run a read-only closure on a reader connection.
    pub async fn read<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let c = db.reader()?;
            let out = f(&c);
            db.give_back(c);
            out
        })
        .await?
    }

    /// Run a closure on the single writer connection (wrap multi-statement work in a transaction).
    pub async fn write<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = db.0.writer.lock();
            f(&mut c)
        })
        .await?
    }

    /// Synchronous read on a reader connection, for window-event handlers that can't await
    /// (it never waits for the writer, so a long sync can't freeze the window).
    pub fn read_blocking<T>(&self, f: impl FnOnce(&Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let c = self.reader()?;
        let out = f(&c);
        self.give_back(c);
        out
    }

    /// Synchronous writer access — only for setup code and tests, never from async handlers.
    pub fn write_blocking<T>(&self, f: impl FnOnce(&mut Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut c = self.0.writer.lock();
        f(&mut c)
    }

    fn migrate_all(&self) -> anyhow::Result<()> {
        self.write_blocking(|c| {
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (module TEXT NOT NULL, version INTEGER NOT NULL, \
                 applied_at INTEGER NOT NULL, PRIMARY KEY (module, version));",
            )?;
            for (module, migs) in all_migrations() {
                apply(c, module, migs)?;
            }
            Ok(())
        })
    }
}

fn apply(c: &mut Connection, module: &str, migs: Migrations) -> anyhow::Result<()> {
    for (version, sql) in migs {
        let done: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE module=?1 AND version=?2)",
            rusqlite::params![module, version],
            |r| r.get(0),
        )?;
        if done {
            continue;
        }
        let tx = c.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {module}/{version}"))?;
        tx.execute(
            "INSERT INTO schema_migrations(module, version, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![module, version, util::now_ms()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Every module's migrations, in dependency order. Owned by the integrator.
fn all_migrations() -> Vec<(&'static str, Migrations)> {
    vec![
        ("core", CORE),
        ("mail", crate::mail::schema::MIGRATIONS),
        ("calendar", crate::calendar::schema::MIGRATIONS),
        ("contacts", crate::contacts::schema::MIGRATIONS),
        ("tasks", crate::tasks::schema::MIGRATIONS),
        ("rules", crate::rules::schema::MIGRATIONS),
        ("crm", CRM_REMOVED),
        ("ai", crate::ai::schema::MIGRATIONS),
        ("archive", crate::archive::schema::MIGRATIONS),
    ]
}

/// The CRM link was removed (the CRM matches e-mail itself now): its cache table and URL go.
/// Version 1 stands in for the old table on new databases, so both kinds end at version 2.
const CRM_REMOVED: Migrations = &[
    (1, "CREATE TABLE IF NOT EXISTS crm_cache (key TEXT PRIMARY KEY, value TEXT NOT NULL, fetched_at INTEGER NOT NULL);"),
    (2, "DROP TABLE IF EXISTS crm_cache; DELETE FROM settings WHERE key LIKE 'crm.%';"),
];

const CORE: Migrations = &[(
    1,
    r#"
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL DEFAULT '',
  provider TEXT NOT NULL CHECK (provider IN ('google','demo')),
  color TEXT NOT NULL DEFAULT '#4A7EBB',
  position INTEGER NOT NULL DEFAULT 0,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL
);
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE sync_state (
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (account_id, kind)
);
CREATE TABLE pending_ops (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  domain TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  next_try_at INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX pending_ops_acct ON pending_ops(account_id, domain, id);
"#,
)];

/// Minimal temp-dir helper so tests don't need an extra crate at runtime.
pub mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        pub fn new() -> anyhow::Result<Self> {
            let p = std::env::temp_dir().join(format!("classicmail-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p)?;
            Ok(TempDirGuard(p))
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
