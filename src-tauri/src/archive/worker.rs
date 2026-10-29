//! The PST reader objects are not `Send`, so archives are read on two dedicated threads:
//! * the **reader** serves on-demand requests (open a message, an attachment, the folder tree) and
//!   keeps a small cache of open handles;
//! * the **indexer** works through a queue of indexing jobs, one archive at a time, in the
//!   background (`index.rs`), with its own handle.

use std::{
    collections::VecDeque,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

use parking_lot::{Condvar, Mutex};

use super::pst::{self, Pst};
use crate::ctx::AppCtx;

/// (size, mtime ms) of a file — how we notice an archive changed.
pub fn file_stamp(path: &Path) -> std::io::Result<(i64, i64)> {
    let m = std::fs::metadata(path)?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    Ok((m.len() as i64, mtime))
}

/// Open PST handles, most recently used last. A handle is reopened when the file changed.
pub struct Handles {
    open: Vec<(PathBuf, (i64, i64), Pst, Instant)>,
    max: usize,
}

impl Handles {
    pub fn new(max: usize) -> Self {
        Handles { open: Vec::new(), max }
    }

    pub fn get(&mut self, path: &Path) -> anyhow::Result<&Pst> {
        let stamp = file_stamp(path).map_err(|e| pst::open_error(path, e))?;
        if let Some(i) = self.open.iter().position(|(p, s, _, _)| p == path && *s == stamp) {
            let mut e = self.open.remove(i);
            e.3 = Instant::now();
            self.open.push(e);
        } else {
            self.open.retain(|(p, ..)| p != path);
            let h = pst::open(path)?;
            self.open.push((path.to_path_buf(), stamp, h, Instant::now()));
            while self.open.len() > self.max {
                self.open.remove(0);
            }
        }
        Ok(&self.open.last().unwrap().2)
    }

    pub fn forget(&mut self, path: &Path) {
        self.open.retain(|(p, ..)| p != path);
    }

    /// Close handles nobody used for a while (so Outlook can take the file again).
    fn expire(&mut self, idle: std::time::Duration) {
        self.open.retain(|(.., t)| t.elapsed() < idle);
    }
}

type Job = Box<dyn FnOnce(&mut Handles) + Send>;

fn reader() -> &'static mpsc::Sender<Job> {
    static TX: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("archive-reader".into())
            .spawn(move || {
                let mut handles = Handles::new(4);
                loop {
                    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
                        Ok(job) => {
                            if std::panic::catch_unwind(AssertUnwindSafe(|| job(&mut handles))).is_err() {
                                // a panic inside the PST reader: start over with fresh handles
                                handles = Handles::new(4);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => handles.expire(std::time::Duration::from_secs(120)),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("archive reader thread");
        tx
    })
}

/// Run `f` with the archive at `path` on the reader thread.
pub async fn with_pst<T, F>(path: PathBuf, f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Pst) -> anyhow::Result<T> + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let job: Job = Box::new(move |h: &mut Handles| {
        let out = match h.get(&path) {
            Ok(p) => f(p),
            Err(e) => Err(e),
        };
        if out.is_err() {
            // a read error may mean the file changed under us: reopen next time
            h.forget(&path);
        }
        let _ = tx.send(out);
    });
    reader().send(job).map_err(|_| anyhow::anyhow!("archive reader stopped"))?;
    rx.await.map_err(|_| anyhow::anyhow!("The archive could not be read (internal error)."))?
}

/// Drop the reader's handle for `path` (archive closed).
pub fn forget(path: PathBuf) {
    let _ = reader().send(Box::new(move |h: &mut Handles| h.forget(&path)));
}

// ---- indexer ------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobKind {
    Headers,
    Bodies,
}

struct IndexQueue {
    jobs: VecDeque<(Arc<AppCtx>, i64, JobKind)>,
    /// the running job: its app context, archive id and cancel flag
    running: Option<(Arc<AppCtx>, i64, Arc<AtomicBool>)>,
}

struct Indexer {
    q: Mutex<IndexQueue>,
    cv: Condvar,
}

fn indexer() -> &'static Indexer {
    static IX: OnceLock<Indexer> = OnceLock::new();
    let ix = IX.get_or_init(|| Indexer { q: Mutex::new(IndexQueue { jobs: VecDeque::new(), running: None }), cv: Condvar::new() });
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new().name("archive-indexer".into()).spawn(indexer_loop).expect("archive indexer thread");
    });
    ix
}

fn indexer_loop() {
    let ix = indexer();
    loop {
        let (ctx, id, kind, cancel) = {
            let mut q = ix.q.lock();
            loop {
                if let Some((ctx, id, kind)) = q.jobs.pop_front() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    q.running = Some((ctx.clone(), id, cancel.clone()));
                    break (ctx, id, kind, cancel);
                }
                ix.cv.wait(&mut q);
            }
        };
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| match kind {
            JobKind::Headers => super::index::index_headers(&ctx, id, &cancel),
            JobKind::Bodies => super::index::index_bodies(&ctx, id, &cancel),
        }));
        match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                log::warn!("archive {id}: indexing ({kind:?}) failed: {}", crate::logging::err(&e));
                super::index::mark_error(&ctx, id, &e);
            }
            Err(_) => {
                log::error!("archive {id}: indexing ({kind:?}) panicked");
                super::index::mark_error(&ctx, id, &anyhow::anyhow!("The archive could not be read."));
            }
        }
        ix.q.lock().running = None;
    }
}

/// Queue a job (no duplicates).
pub fn enqueue(ctx: &Arc<AppCtx>, archive_id: i64, kind: JobKind) {
    let ix = indexer();
    let mut q = ix.q.lock();
    if q.jobs.iter().any(|(c, id, k)| *id == archive_id && *k == kind && Arc::ptr_eq(c, ctx)) {
        return;
    }
    q.jobs.push_back((ctx.clone(), archive_id, kind));
    ix.cv.notify_one();
}

/// Stop and unqueue everything for an archive (of this app context: ids are per database, and
/// tests run several contexts at once).
pub fn cancel(ctx: &Arc<AppCtx>, archive_id: i64) {
    let ix = indexer();
    let mut q = ix.q.lock();
    q.jobs.retain(|(c, id, _)| *id != archive_id || !Arc::ptr_eq(c, ctx));
    if let Some((c, id, flag)) = &q.running
        && *id == archive_id
        && Arc::ptr_eq(c, ctx)
    {
        flag.store(true, Ordering::SeqCst);
    }
}

/// Is a job queued or running for this archive?
pub fn busy(ctx: &Arc<AppCtx>, archive_id: i64) -> bool {
    let q = indexer().q.lock();
    let mine = |c: &Arc<AppCtx>, id: i64| id == archive_id && Arc::ptr_eq(c, ctx);
    q.running.as_ref().is_some_and(|(c, id, _)| mine(c, *id)) || q.jobs.iter().any(|(c, id, _)| mine(c, *id))
}

/// Wait until the indexer is idle (tests).
pub async fn wait_idle() {
    loop {
        {
            let q = indexer().q.lock();
            if q.jobs.is_empty() && q.running.is_none() {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
