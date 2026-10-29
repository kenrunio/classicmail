//! Reading PST files with Microsoft's `outlook-pst` crate (NDB / LTP / messaging layers, ANSI and
//! Unicode). Files are opened read-only with shared access (FILE_SHARE_READ | FILE_SHARE_WRITE |
//! FILE_SHARE_DELETE — Rust's default on Windows), never through the crate's `open()`, which also
//! asks for write access.
//!
//! The crate's objects are `Rc`-based (not `Send`): everything here runs on the archive worker
//! threads (`worker.rs`) and hands plain data back.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
    rc::Rc,
};

use outlook_pst::{
    AnsiPstFile, PstReader, UnicodePstFile,
    ltp::{
        prop_context::PropertyValue,
        table_context::{TableContext, TableRowData},
    },
    messaging::{
        attachment::{AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment},
        folder::Folder,
        message::{AnsiMessage, Message, UnicodeMessage},
        store::{AnsiStore, Store, UnicodeStore},
    },
    ndb::node_id::NodeId,
};

use super::props::{PVal, Props, tag};

pub enum Pst {
    Unicode(Rc<UnicodeStore>),
    Ansi(Rc<AnsiStore>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Unicode,
    Ansi,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Unicode => "unicode",
            Format::Ansi => "ansi",
        }
    }
}

/// Windows "file is in use" errors (Outlook keeps open PSTs locked).
pub fn is_sharing_violation(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32) | Some(33))
}

pub fn locked_error() -> anyhow::Error {
    crate::user_err!("locked", "This archive is open in Outlook. Close Outlook (or remove it there) and try again.")
}

/// Map an open error to a message for the user.
pub fn open_error(path: &Path, e: std::io::Error) -> anyhow::Error {
    if is_sharing_violation(&e) {
        return locked_error();
    }
    match e.kind() {
        std::io::ErrorKind::NotFound => crate::user_err!("not_found", "The file {} could not be found.", path.display()),
        std::io::ErrorKind::PermissionDenied => crate::user_err!("denied", "ClassicMail can't read {} (access denied).", path.display()),
        _ => crate::user_err!("open", "{} could not be opened: {e}", path.display()),
    }
}

/// Read the header and say which flavour of PST this is.
pub fn sniff(path: &Path) -> anyhow::Result<Format> {
    let mut f = File::open(path).map_err(|e| open_error(path, e))?;
    let mut h = [0u8; 16];
    f.read_exact(&mut h).map_err(|_| crate::user_err!("not_pst", "{} is not an Outlook data file (.pst).", path.display()))?;
    if &h[0..4] != b"!BDN" {
        return Err(crate::user_err!("not_pst", "{} is not an Outlook data file (.pst).", path.display()));
    }
    let ver = u16::from_le_bytes([h[10], h[11]]);
    match ver {
        14 | 15 => Ok(Format::Ansi),
        23 => Ok(Format::Unicode),
        36 | 37 => Err(crate::user_err!(
            "unsupported",
            "{} uses the 4K-page format of newer Outlook offline files, which ClassicMail can't read yet.",
            path.display()
        )),
        v => Err(crate::user_err!("unsupported", "{} has an unknown data file version ({v}).", path.display())),
    }
}

fn reader(path: &Path) -> anyhow::Result<Box<dyn PstReader>> {
    let f = File::open(path).map_err(|e| open_error(path, e))?;
    Ok(Box::new(BufReader::with_capacity(16 * 1024, f)))
}

pub fn open(path: &Path) -> anyhow::Result<Pst> {
    let fmt = sniff(path)?;
    let bad = |e: std::io::Error| crate::user_err!("corrupt", "{} could not be read: {e}", path.display());
    Ok(match fmt {
        Format::Unicode => Pst::Unicode(UnicodeStore::read(Rc::new(UnicodePstFile::read_from(reader(path)?).map_err(bad)?)).map_err(bad)?),
        Format::Ansi => Pst::Ansi(AnsiStore::read(Rc::new(AnsiPstFile::read_from(reader(path)?).map_err(bad)?)).map_err(bad)?),
    })
}

/// Convert a crate value (`Object` / multi-valued ones are not needed).
pub fn pval(v: &PropertyValue) -> PVal {
    match v {
        PropertyValue::Integer16(x) => PVal::Int(*x as i64),
        PropertyValue::Integer32(x) => PVal::Int(*x as i64),
        PropertyValue::Integer64(x) | PropertyValue::Currency(x) => PVal::Int(*x),
        PropertyValue::ErrorCode(_) => PVal::Other,
        PropertyValue::Boolean(b) => PVal::Bool(*b),
        PropertyValue::Time(t) => PVal::Time(*t),
        PropertyValue::String8(s) => PVal::Str8(s.buffer().to_vec()),
        PropertyValue::Unicode(s) => PVal::Str(String::from_utf16_lossy(s.buffer())),
        PropertyValue::Binary(b) => PVal::Bin(b.buffer().to_vec()),
        _ => PVal::Other,
    }
}

/// One table row as properties (only `want` columns when given).
pub fn row_props(t: &dyn TableContext, row: &TableRowData, want: Option<&[u16]>) -> Props {
    let ctx = t.context();
    let mut p = Props::default();
    let Ok(cols) = row.columns(ctx) else { return p };
    for (col, val) in ctx.columns().iter().zip(cols.iter()) {
        let id = col.prop_id();
        if want.is_some_and(|w| !w.contains(&id)) {
            continue;
        }
        let Some(val) = val else { continue };
        if let Ok(v) = t.read_column(val, col.prop_type()) {
            let v = pval(&v);
            if v != PVal::Other {
                p.set(id, v);
            }
        }
    }
    p
}

#[derive(Debug, Clone)]
pub struct FolderInfo {
    pub node_id: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub container_class: String,
    pub content_count: i64,
    pub unread: i64,
}

fn props_of_map<'a>(it: impl Iterator<Item = (&'a u16, &'a PropertyValue)>) -> Props {
    let mut p = Props::default();
    for (k, v) in it {
        let v = pval(v);
        if v != PVal::Other {
            p.set(*k, v);
        }
    }
    p
}

impl Pst {
    pub fn store(&self) -> Rc<dyn Store> {
        match self {
            Pst::Unicode(s) => s.clone(),
            Pst::Ansi(s) => s.clone(),
        }
    }

    pub fn format(&self) -> Format {
        match self {
            Pst::Unicode(_) => Format::Unicode,
            Pst::Ansi(_) => Format::Ansi,
        }
    }

    pub fn display_name(&self) -> String {
        self.store().properties().display_name().unwrap_or_default()
    }

    fn open_folder(&self, nid: u32) -> anyhow::Result<Rc<dyn Folder>> {
        let store = self.store();
        let eid = store.properties().make_entry_id(NodeId::from(nid))?;
        Ok(store.open_folder(&eid)?)
    }

    /// Every folder under the IPM subtree, parents before children (`parent` None = top level).
    pub fn folders(&self) -> anyhow::Result<Vec<FolderInfo>> {
        let store = self.store();
        let ipm = store.properties().ipm_sub_tree_entry_id()?;
        let root = store.open_folder(&ipm)?;
        let mut out = Vec::new();
        let mut queue: Vec<(Rc<dyn Folder>, Option<u32>)> = vec![(root, None)];
        let mut guard = 0;
        while let Some((f, parent)) = queue.pop() {
            let Some(ht) = f.hierarchy_table() else { continue };
            let mut kids = Vec::new();
            for row in ht.rows_matrix() {
                let nid = u32::from(row.id());
                guard += 1;
                if guard > 100_000 {
                    break;
                }
                match self.open_folder(nid) {
                    Ok(sub) => {
                        let fp = sub.properties();
                        let p = props_of_map(fp.iter());
                        out.push(FolderInfo {
                            node_id: nid,
                            parent,
                            name: p.text(tag::DISPLAY_NAME).unwrap_or_else(|| "(no name)".into()),
                            container_class: p.text(tag::CONTAINER_CLASS).unwrap_or_default(),
                            content_count: fp.content_count().unwrap_or(0) as i64,
                            unread: fp.unread_count().unwrap_or(0) as i64,
                        });
                        kids.push((sub, Some(nid)));
                    }
                    Err(e) => log::warn!("archive: folder {nid:#x}: {}", crate::logging::err(&e)),
                }
            }
            // depth-first in table order
            kids.reverse();
            queue.extend(kids);
        }
        Ok(out)
    }

    /// The rows of a folder's contents table (`f(row index, node id, props)`; return false to stop).
    /// The first `skip` rows (already indexed) are counted but not read.
    pub fn folder_rows(
        &self,
        folder_nid: u32,
        want: Option<&[u16]>,
        skip: usize,
        mut f: impl FnMut(usize, u32, Props) -> bool,
    ) -> anyhow::Result<usize> {
        let folder = self.open_folder(folder_nid)?;
        let Some(ct) = folder.contents_table() else { return Ok(0) };
        let mut n = 0;
        for (i, row) in ct.rows_matrix().enumerate() {
            n += 1;
            if i < skip {
                continue;
            }
            let p = row_props(ct.as_ref(), row, want);
            if !f(i, u32::from(row.id()), p) {
                break;
            }
        }
        Ok(n)
    }

    /// Number of rows in a folder's contents table.
    pub fn folder_row_count(&self, folder_nid: u32) -> anyhow::Result<usize> {
        let folder = self.open_folder(folder_nid)?;
        Ok(folder.contents_table().map(|t| t.rows_matrix().count()).unwrap_or(0))
    }

    /// A message with recipients and attachments (data included when `with_data`).
    pub fn message(&self, nid: u32, with_data: bool) -> anyhow::Result<FullMessage> {
        macro_rules! read {
            ($store:expr, $M:ty, $A:ty) => {{
                let store = $store.clone();
                let eid = store.properties().make_entry_id(NodeId::from(nid))?;
                let msg = <$M>::read(store, &eid, None)?;
                let mut full = from_message(msg.as_ref());
                if let Some(at) = msg.attachment_table() {
                    for (index, row) in at.rows_matrix().enumerate() {
                        let sub = NodeId::from(u32::from(row.id()));
                        let row_p = row_props(at.as_ref(), row, None);
                        match <$A>::read(msg.clone(), sub, None) {
                            Ok(a) => full.attachments.push(attachment(index, a.as_ref(), with_data)),
                            Err(e) => {
                                log::debug!("archive: attachment {index} of {nid:#x}: {}", crate::logging::redact(&e.to_string()));
                                full.attachments.push(AttachmentInfo::from_props(index, &row_p));
                            }
                        }
                    }
                }
                Ok(full)
            }};
        }
        match self {
            Pst::Unicode(s) => read!(s, UnicodeMessage, UnicodeAttachment),
            Pst::Ansi(s) => read!(s, AnsiMessage, AnsiAttachment),
        }
    }

    /// Only the properties of a message (no attachment data): message text indexing.
    pub fn message_props(&self, nid: u32, want: &[u16]) -> anyhow::Result<Props> {
        let store = self.store();
        let eid = store.properties().make_entry_id(NodeId::from(nid))?;
        let m = store.open_message(&eid, Some(want))?;
        Ok(props_of_map(m.properties().iter()))
    }
}

#[derive(Debug, Clone)]
pub struct Recipient {
    /// 1 To, 2 Cc, 3 Bcc
    pub kind: i64,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Default)]
pub struct AttachmentInfo {
    pub index: usize,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub hidden: bool,
    /// afEmbeddedMessage
    pub embedded: Option<Box<FullMessage>>,
    pub data: Option<Vec<u8>>,
}

impl AttachmentInfo {
    fn from_props(index: usize, p: &Props) -> Self {
        let filename = p
            .text(tag::ATTACH_LONG_FILENAME)
            .or_else(|| p.text(tag::ATTACH_FILENAME))
            .or_else(|| p.text(tag::DISPLAY_NAME))
            .unwrap_or_else(|| format!("attachment{}", index + 1));
        let mime_type =
            p.text(tag::ATTACH_MIME).unwrap_or_else(|| mime_guess::from_path(&filename).first_or_octet_stream().essence_str().to_string());
        AttachmentInfo {
            index,
            filename,
            mime_type,
            size: p.int(tag::ATTACH_SIZE).unwrap_or(0),
            content_id: p.text(tag::ATTACH_CONTENT_ID),
            hidden: p.bool(tag::ATTACH_HIDDEN).unwrap_or(false),
            embedded: None,
            data: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FullMessage {
    pub props: Props,
    pub recipients: Vec<Recipient>,
    pub attachments: Vec<AttachmentInfo>,
}

fn from_message(m: &dyn Message) -> FullMessage {
    let props = props_of_map(m.properties().iter());
    let mut recipients = Vec::new();
    if let Some(rt) = m.recipient_table() {
        for row in rt.rows_matrix() {
            let mut p = row_props(rt.as_ref(), row, None);
            if let Some(cp) = props.codepage() {
                p.set(tag::MESSAGE_CODEPAGE, PVal::Int(cp as i64));
            }
            let email = super::props::smtp_of(p.text(tag::ADDRTYPE), p.text(tag::EMAIL), p.text(tag::SMTP_ADDRESS)).unwrap_or_default();
            let name = p.text(tag::DISPLAY_NAME).unwrap_or_else(|| email.clone());
            recipients.push(Recipient { kind: p.int(tag::RECIPIENT_TYPE).unwrap_or(1), name, email });
        }
    }
    FullMessage { props, recipients, attachments: Vec::new() }
}

fn attachment(index: usize, a: &dyn Attachment, with_data: bool) -> AttachmentInfo {
    let mut p = props_of_map(a.properties().iter().filter(|(k, _)| **k != tag::ATTACH_DATA));
    if let Some(cp) = props_of_map(a.message().properties().iter().filter(|(k, _)| **k == tag::MESSAGE_CODEPAGE)).codepage() {
        p.set(tag::MESSAGE_CODEPAGE, PVal::Int(cp as i64));
    }
    let mut info = AttachmentInfo::from_props(index, &p);
    match a.data() {
        Some(AttachmentData::Binary(b)) => {
            if info.size <= 0 || info.size < b.buffer().len() as i64 {
                info.size = b.buffer().len() as i64;
            }
            if with_data {
                info.data = Some(b.buffer().to_vec());
            }
        }
        Some(AttachmentData::Message(m)) => {
            let inner = from_message(m.as_ref());
            let subj = inner.props.str(tag::SUBJECT).map(|s| super::props::clean_subject(&s)).unwrap_or_default();
            if p.text(tag::ATTACH_LONG_FILENAME).is_none() && p.text(tag::ATTACH_FILENAME).is_none() {
                info.filename = p.text(tag::DISPLAY_NAME).unwrap_or(if subj.is_empty() { "Message".into() } else { subj });
            }
            if !info.filename.to_ascii_lowercase().ends_with(".eml") {
                info.filename.push_str(".eml");
            }
            info.mime_type = "message/rfc822".into();
            info.embedded = Some(Box::new(inner));
        }
        None => {}
    }
    info
}
