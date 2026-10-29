//! Search timing on a real mailbox: `CLASSICMAIL_BENCH_DB=<copy of classicmail.db> cargo test
//! search_bench -- --ignored --nocapture`. Read-only; never runs in the normal test suite.

use std::time::Instant;

use rusqlite::{Connection, OpenFlags};

use super::list::{self, ListParams, Source};

#[test]
#[ignore]
fn search_bench() {
    let Ok(path) = std::env::var("CLASSICMAIL_BENCH_DB") else { return };
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let inbox: i64 = c.query_row("SELECT id FROM folders WHERE role='inbox' ORDER BY account_id LIMIT 1", [], |r| r.get(0)).unwrap();
    let queries = ["c", "ca", "acme", "investor deck", "מצגת", "from:moshe", "subject:meeting", "hasattachments:yes", "the"];
    for conversations in [false, true] {
        for scope in ["folder", "all"] {
            for q in queries {
                let p = ListParams {
                    source: Source { folder_id: Some(inbox), ..Default::default() },
                    search: Some(q.to_string()),
                    scope: Some(scope.to_string()),
                    with_groups: true,
                    conversations,
                    ..Default::default()
                };
                let mut best = f64::MAX;
                let mut n = 0;
                for _ in 0..3 {
                    let t = Instant::now();
                    let r = list::list(&c, &p).unwrap();
                    best = best.min(t.elapsed().as_secs_f64() * 1000.0);
                    n = r.total;
                }
                println!("conv={conversations:5} scope={scope:6} {q:20} {best:8.1} ms  ({n} hits)");
            }
        }
    }
}

/// What the message list asks for after every change (Archive, Delete, new mail): the total with
/// the date groups, then the first page. Same use as `search_bench`, run `list_bench`.
#[test]
#[ignore]
fn list_bench() {
    let Ok(path) = std::env::var("CLASSICMAIL_BENCH_DB") else { return };
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut st = c.prepare("SELECT id, account_id FROM folders WHERE role='inbox' ORDER BY account_id").unwrap();
    let inboxes: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
    for (inbox, account) in inboxes {
        for conversations in [false, true] {
            for (what, limit, with_groups) in [("total+groups", 0, true), ("first page", 100, false)] {
                let p = ListParams {
                    source: Source { folder_id: Some(inbox), ..Default::default() },
                    with_groups,
                    conversations,
                    limit,
                    desc: true,
                    arrange: "date".into(),
                    ..Default::default()
                };
                let mut best = f64::MAX;
                let mut n = 0;
                for _ in 0..3 {
                    let t = Instant::now();
                    let r = list::list(&c, &p).unwrap();
                    best = best.min(t.elapsed().as_secs_f64() * 1000.0);
                    n = r.total;
                }
                println!("account {account} inbox conv={conversations:5} {what:13} {best:8.1} ms  ({n})");
            }
        }
    }
}
