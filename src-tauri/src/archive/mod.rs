//! Outlook archives: read-only viewing of Outlook data files (.pst) next to the mailboxes.
//!
//! Layout: `pst` (the `outlook-pst` crate behind a small API; ANSI + Unicode files, shared
//! read-only access), `worker` (reader thread with cached handles + background indexer thread),
//! `index` (folder tree and header rows into SQLite + FTS, resumable, change detection), `props`
//! (MAPI property rules: headers, body choice), `lzfu` + `rtf` (compressed RTF → HTML / text),
//! `codepage`, `list` (`archive.listMessages`, same shape as `mail.listMessages`), `api` (rpc).
//!
//! Nothing here ever writes to a PST file.

pub mod api;
pub mod codepage;
pub mod index;
pub mod list;
pub mod lzfu;
pub mod props;
pub mod pst;
pub mod rtf;
pub mod schema;
pub mod worker;

#[cfg(test)]
mod tests;

use crate::rpc::Registry;

pub fn register(r: &mut Registry) {
    api::register(r);
}
