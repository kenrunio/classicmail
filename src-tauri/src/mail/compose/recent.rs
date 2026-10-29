//! "Recent items" for Attach File (like the new Outlook): files attached recently in ClassicMail
//! and Windows' own Recent Items (%APPDATA%\Microsoft\Windows\Recent\*.lnk: what Word, Excel and
//! the rest just opened or saved), together, the most recently saved or used first. A document
//! saved a minute ago heads the list. Only existing files are offered (no folders, no shortcuts).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    ctx::AppCtx,
    db::util::{get_setting, set_setting},
};

const SETTING: &str = "compose.recentAttachments";
const KEEP: usize = 30;
const MAX_ATTACH: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub size: u64,
    /// when it was last saved, used in Windows or attached here, whichever is latest (epoch ms)
    pub used: i64,
}

#[derive(Deserialize)]
pub struct Limit {
    limit: Option<usize>,
}

fn ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn describe(p: &Path, used: i64) -> Option<RecentFile> {
    let md = std::fs::metadata(p).ok()?;
    if !md.is_file() {
        return None;
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    // shortcuts, mail archives and anything over Gmail's 25 MB limit can't sensibly be attached
    if matches!(ext.as_str(), "lnk" | "url" | "pst" | "ost" | "tmp") || md.len() > MAX_ATTACH {
        return None;
    }
    let saved = md.modified().map(ms).unwrap_or(0);
    Some(RecentFile {
        path: p.to_string_lossy().into_owned(),
        name: p.file_name()?.to_string_lossy().into_owned(),
        folder: p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        size: md.len(),
        used: used.max(saved),
    })
}

/// Target of a .lnk shortcut (local path), if it can be read: as Windows itself reads it, else
/// from the file's own fields.
fn lnk_target(lnk_path: &Path) -> Option<PathBuf> {
    shell_target(lnk_path).or_else(|| parsed_target(lnk_path))
}

/// The shortcut's target as Windows resolves it (IShellLink). Word and Excel's shortcuts often
/// carry only an ID list (no local path field), which `parsed_target` can't read.
#[cfg(windows)]
fn shell_target(lnk_path: &Path) -> Option<PathBuf> {
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, STGM_READ,
            },
            UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink},
        },
        core::{HSTRING, Interface},
    };
    unsafe {
        let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let target = (|| {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk_path.as_os_str()), STGM_READ).ok()?;
            let mut buf = [0u16; 1024];
            link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).ok()?;
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..len]);
            (!s.is_empty()).then(|| PathBuf::from(s))
        })();
        if inited {
            CoUninitialize();
        }
        target
    }
}

#[cfg(not(windows))]
fn shell_target(_: &Path) -> Option<PathBuf> {
    None
}

/// Target of a .lnk shortcut read from the file's own fields (local path, or the relative path).
fn parsed_target(lnk_path: &Path) -> Option<PathBuf> {
    let link = lnk::ShellLink::open(lnk_path, lnk::encoding::WINDOWS_1252).ok()?;
    if let Some(info) = link.link_info()
        && let Some(base) = info.local_base_path_unicode().clone().or_else(|| info.local_base_path().map(str::to_string))
    {
        let suffix = info.common_path_suffix_unicode().clone().unwrap_or_else(|| info.common_path_suffix().to_string());
        let mut s = base;
        if !suffix.is_empty() {
            if !s.ends_with('\\') {
                s.push('\\');
            }
            s.push_str(&suffix);
        }
        return Some(PathBuf::from(s));
    }
    // fall back to the relative path stored next to the link (relative to the Recent folder)
    let rel = link.string_data().relative_path().clone()?;
    Some(lnk_path.parent()?.join(rel))
}

/// Windows Recent Items, newest first (up to `max` candidates looked at).
fn windows_recent(max: usize) -> Vec<(PathBuf, i64)> {
    let Some(appdata) = std::env::var_os("APPDATA") else { return vec![] };
    let dir = PathBuf::from(appdata).join("Microsoft").join("Windows").join("Recent");
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut links: Vec<(PathBuf, i64)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), ms(t)))
        })
        .collect();
    links.sort_by(|a, b| b.1.cmp(&a.1));
    links.truncate(max);
    links.into_iter().filter_map(|(l, t)| lnk_target(&l).map(|p| (p, t))).collect()
}

/// Remembered ClassicMail attachments: [(path, used ms)], newest first.
fn remembered(ctx: &AppCtx) -> impl std::future::Future<Output = anyhow::Result<Vec<(String, i64)>>> + '_ {
    ctx.db.read(|c| Ok(get_setting::<Vec<(String, i64)>>(c, SETTING)?.unwrap_or_default()))
}

pub async fn list(ctx: Arc<AppCtx>, p: Limit) -> anyhow::Result<Vec<RecentFile>> {
    let limit = p.limit.unwrap_or(12).clamp(1, 50);
    let mine = remembered(&ctx).await?;
    tokio::task::spawn_blocking(move || {
        Ok(newest(mine.into_iter().map(|(p, t)| (PathBuf::from(p), t)).chain(windows_recent(limit * 6)), limit))
    })
    .await?
}

/// The `limit` most recent existing files among `candidates` ((path, used ms), any order, both
/// sources together, so a long attachment history can't hide a document just saved).
fn newest(candidates: impl Iterator<Item = (PathBuf, i64)>, limit: usize) -> Vec<RecentFile> {
    let mut seen = HashSet::new();
    let mut out: Vec<RecentFile> =
        candidates.filter(|(p, _)| seen.insert(p.to_string_lossy().to_lowercase())).filter_map(|(p, used)| describe(&p, used)).collect();
    out.sort_by(|a, b| b.used.cmp(&a.used));
    out.truncate(limit);
    out
}

#[derive(Deserialize)]
pub struct InFolder {
    folder: String,
    limit: Option<usize>,
}

/// How deep and how many entries `in_folder` looks (a client's folder on Google Drive can be big).
const FOLDER_DEPTH: usize = 3;
const FOLDER_ENTRIES: usize = 4000;

/// The most recently saved files in `folder` and its subfolders (a few levels), newest first: the
/// client's folder at the top of Attach File when writing to them (see save_folders.rs). Office's
/// lock files ("~$…") and hidden files are left out.
pub async fn in_folder(_ctx: Arc<AppCtx>, p: InFolder) -> anyhow::Result<Vec<RecentFile>> {
    let limit = p.limit.unwrap_or(6).clamp(1, 30);
    tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        let mut dirs = vec![(PathBuf::from(&p.folder), 0usize)];
        let mut seen = 0;
        while let Some((dir, depth)) = dirs.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                seen += 1;
                if seen > FOLDER_ENTRIES {
                    break;
                }
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("~$") || name.starts_with('.') {
                    continue;
                }
                match e.file_type() {
                    Ok(t) if t.is_dir() && depth < FOLDER_DEPTH => dirs.push((e.path(), depth + 1)),
                    Ok(t) if t.is_file() => files.push((e.path(), 0)),
                    _ => {}
                }
            }
        }
        Ok(newest(files.into_iter(), limit))
    })
    .await?
}

#[derive(Deserialize)]
pub struct Paths {
    paths: Vec<String>,
}

/// The compose window reports files it attached so they head the list next time.
pub async fn remember(ctx: Arc<AppCtx>, p: Paths) -> anyhow::Result<()> {
    let now = crate::clock::now_ms();
    ctx.db
        .write(move |c| {
            let mut list: Vec<(String, i64)> = get_setting(c, SETTING)?.unwrap_or_default();
            for path in p.paths {
                if path.trim().is_empty() {
                    continue;
                }
                list.retain(|(x, _)| !x.eq_ignore_ascii_case(&path));
                list.insert(0, (path, now));
            }
            list.truncate(KEEP);
            set_setting(c, SETTING, &list)
        })
        .await
}

#[cfg(test)]
mod tests {

    use serde_json::json;

    #[tokio::test]
    async fn remembered_files_come_first_and_missing_ones_are_skipped() {
        let (ctx, guard) = crate::ctx::AppCtx::for_test().unwrap();
        let a = guard.path().join("deck Q3 — שלום.pptx");
        let b = guard.path().join("model.xlsx");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"yy").unwrap();
        let reg = crate::rpc::build();
        let gone = guard.path().join("deleted.pdf").to_string_lossy().into_owned();
        reg.call(ctx.clone(), "mail.compose.rememberAttachments", json!({ "paths": [gone, a.to_string_lossy(), b.to_string_lossy()] }))
            .await
            .unwrap();
        let out = reg.call(ctx, "mail.compose.recentFiles", json!({ "limit": 50 })).await.unwrap();
        let names: Vec<&str> = out.as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"model.xlsx") && names.contains(&"deck Q3 — שלום.pptx"), "{names:?}");
        assert!(!names.contains(&"deleted.pdf"));
    }

    /// A client's folder: its newest files first, from subfolders too, without Office's lock files.
    #[tokio::test]
    async fn a_clients_folder_newest_first() {
        let (ctx, guard) = crate::ctx::AppCtx::for_test().unwrap();
        let client = guard.path().join("Clients").join("Globex");
        let sub = client.join("Press Releases");
        std::fs::create_dir_all(&sub).unwrap();
        let at = |f: &std::path::Path, mins_ago: i64| {
            std::fs::write(f, b"x").unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis((crate::clock::now_ms() - mins_ago * 60_000) as u64);
            std::fs::File::options().write(true).open(f).unwrap().set_modified(t).unwrap();
        };
        at(&client.join("Deck.pptx"), 60);
        at(&sub.join("Press release V1.docx"), 1);
        at(&sub.join("~$ess release V1.docx"), 0);
        at(&client.join("Model.xlsx"), 30);
        let reg = crate::rpc::build();
        let out = reg.call(ctx, "mail.compose.folderFiles", json!({ "folder": client.to_string_lossy() })).await.unwrap();
        let names: Vec<&str> = out.as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Press release V1.docx", "Model.xlsx", "Deck.pptx"]);
    }

    /// Many files attached before don't crowd out a document saved just now (Windows Recent).
    #[test]
    fn a_document_just_saved_heads_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let old_day = crate::clock::now_ms() - 30 * 86_400_000;
        let mut attached = vec![];
        for i in 0..20 {
            let f = dir.path().join(format!("attached {i}.pdf"));
            std::fs::write(&f, b"x").unwrap();
            // saved long ago: the file's own time matches its old use
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis((old_day + i * 60_000) as u64);
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
            attached.push((f, old_day + i * 60_000));
        }
        let saved = dir.path().join("Q3Financials.xlsx");
        std::fs::write(&saved, b"new").unwrap();
        // Windows recorded it an hour ago, but it was saved again just now
        let from_windows = (saved.clone(), crate::clock::now_ms() - 3_600_000);
        let out = super::newest(attached.into_iter().chain([from_windows]), 12);
        assert_eq!(out.len(), 12);
        assert_eq!(out[0].name, "Q3Financials.xlsx");
        assert!(out[0].used >= crate::clock::now_ms() - 60_000, "the save time counts");
        assert_eq!(out[1].name, "attached 19.pdf");
    }
}
