//! Deterministic demo mailboxes for the fake Gmail server (≈400 messages per user over the last six
//! months, heavier recently): client threads with PDF/XLSX attachments, investor meeting requests
//! (text/calendar METHOD:REQUEST), HTML newsletters with remote pictures, inline cid images, Hebrew
//! mail in UTF-8, windows-1255 and iso-8859-8, receipts, sent replies, drafts, junk and trash.

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{TimeZone, Timelike};
use mail_builder::{MessageBuilder, headers::raw::Raw, mime::MimePart};

use super::{FakeDraft, Store};
use crate::fake_google::seed::{DAY_MS, HOUR_MS, PEOPLE, Person, Rng, now_ms};

// ---- tiny file generators (valid PNG / PDF / XLSX bytes) -------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Uncompressed ("stored") zlib stream.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if data.is_empty() { vec![&[][..]] } else { data.chunks(65535).collect() };
    for (i, c) in chunks.iter().enumerate() {
        out.push(if i + 1 == chunks.len() { 1 } else { 0 });
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// RGB PNG from a pixel function.
pub fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend_from_slice(&px(x, y));
        }
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &zlib_stored(&raw));
    chunk(b"IEND", &[]);
    out
}

/// "Duvz Software" style logo: blue band with an orange accent.
pub fn logo_png() -> Vec<u8> {
    png(96, 28, |x, y| {
        if x < 6 {
            [0xF3, 0x9C, 0x12]
        } else if (8..=88).contains(&x) && (y == 13 || y == 14) && x % 12 < 9 {
            [0xFF, 0xFF, 0xFF]
        } else {
            let t = (y * 3) as u8;
            [0x1F, 0x4E_u8.saturating_add(t), 0x8C_u8.saturating_add(t)]
        }
    })
}

/// Small bar chart (inline newsletter / signature images).
pub fn chart_png(seed: u32) -> Vec<u8> {
    let bars = [12u32, 20, 17, 26, 31, 24, 35];
    png(140, 60, move |x, y| {
        let i = (x / 20) as usize;
        let h = bars[(i + seed as usize) % bars.len()] + 10;
        let in_bar = y < 58 && x % 20 > 4 && x % 20 < 16 && 58 - y < h;
        if y >= 58 {
            [0x60, 0x60, 0x60]
        } else if in_bar {
            [0x3A, 0x6F, 0xC4]
        } else {
            [0xFF, 0xFF, 0xFF]
        }
    })
}

fn pdf_escape(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii()).collect::<String>().replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

/// One-page PDF with a title and some lines of Helvetica text.
pub fn pdf(title: &str, lines: &[&str]) -> Vec<u8> {
    let mut content = format!("BT /F1 18 Tf 72 770 Td ({}) Tj ET\n", pdf_escape(title));
    let mut y = 735;
    for l in lines {
        content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({}) Tj ET\n", pdf_escape(l)));
        y -= 18;
    }
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes());
    out
}

/// Zip archive with "stored" entries.
fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let off = out.len() as u32;
        let crc = crc32(data);
        let n = name.as_bytes();
        let mut h = Vec::new();
        h.extend_from_slice(&0x04034b50u32.to_le_bytes());
        h.extend_from_slice(&[20, 0, 0, 0, 0, 0]); // version, flags, method=stored
        h.extend_from_slice(&[0, 0, 0x21, 0]); // time, date (1980-01-01)
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(n.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        let mut c = Vec::new();
        c.extend_from_slice(&0x02014b50u32.to_le_bytes());
        c.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(n.len() as u16).to_le_bytes());
        // extra len, comment len, disk number, internal attrs (2 each), external attrs (4)
        c.extend_from_slice(&[0; 12]);
        c.extend_from_slice(&off.to_le_bytes());
        c.extend_from_slice(n);
        central.extend_from_slice(&c);
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Minimal .xlsx workbook (one sheet, inline strings / numbers).
pub fn xlsx(rows: &[Vec<String>]) -> Vec<u8> {
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, row) in rows.iter().enumerate() {
        sheet.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, v) in row.iter().enumerate() {
            let col = (b'A' + c as u8) as char;
            if v.parse::<f64>().is_ok() {
                sheet.push_str(&format!("<c r=\"{col}{}\"><v>{v}</v></c>", r + 1));
            } else {
                sheet.push_str(&format!("<c r=\"{col}{}\" t=\"inlineStr\"><is><t>{}</t></is></c>", r + 1, xml_escape(v)));
            }
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    zip(&[
        ("[Content_Types].xml", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.to_vec()),
        ("_rels/.rels", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec()),
        ("xl/workbook.xml", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_vec()),
        ("xl/_rels/workbook.xml.rels", br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec()),
        ("xl/worksheets/sheet1.xml", sheet.into_bytes()),
    ])
}

const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

// ---- message building -------------------------------------------------------------------------

#[derive(Clone)]
struct Who {
    name: String,
    email: String,
}

impl Who {
    fn of(p: &Person) -> Who {
        Who { name: p.name(), email: p.email() }
    }
    fn new(name: &str, email: &str) -> Who {
        Who { name: name.into(), email: email.into() }
    }
    fn pair(&self) -> (String, String) {
        (self.name.clone(), self.email.clone())
    }
}

#[derive(Clone, Default)]
struct Att {
    mime: String,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Default)]
struct Spec {
    from: Option<Who>,
    to: Vec<Who>,
    cc: Vec<Who>,
    subject: String,
    text: Option<String>,
    html: Option<String>,
    /// (cid, mime, bytes) — goes into multipart/related with the HTML
    inline: Vec<(String, String, Vec<u8>)>,
    atts: Vec<Att>,
    /// raw text/calendar part (METHOD:REQUEST)
    ics: Option<String>,
    importance_high: bool,
    headers: Vec<(String, String)>,
    /// fully hand-built raw message (charset tests)
    raw: Option<String>,
}

fn rfc2822(ms: i64) -> String {
    chrono::Utc.timestamp_millis_opt(ms).single().unwrap_or_default().with_timezone(&chrono_tz::Asia::Jerusalem).to_rfc2822()
}

fn build_raw(sp: &Spec, date_ms: i64, msg_id: &str, in_reply_to: Option<&str>, refs: &[String]) -> String {
    if let Some(r) = &sp.raw {
        return r.clone();
    }
    let from = sp.from.clone().unwrap();
    let mut b = MessageBuilder::new()
        .from(from.pair())
        .subject(sp.subject.clone())
        .header("Date", Raw::new(rfc2822(date_ms)))
        .message_id(msg_id.to_string());
    if !sp.to.is_empty() {
        b = b.to(sp.to.iter().map(Who::pair).collect::<Vec<_>>());
    }
    if !sp.cc.is_empty() {
        b = b.cc(sp.cc.iter().map(Who::pair).collect::<Vec<_>>());
    }
    if let Some(r) = in_reply_to {
        b = b.in_reply_to(r.to_string());
        b = b.references(refs.to_vec());
    }
    if sp.importance_high {
        b = b.header("Importance", Raw::new("High")).header("X-Priority", Raw::new("1 (Highest)"));
    }
    for (k, v) in &sp.headers {
        b = b.header(k.clone(), Raw::new(v.clone()));
    }
    // body tree
    let text = sp.text.clone();
    let html = sp.html.clone();
    let mut alt: Vec<MimePart> = Vec::new();
    if let Some(t) = &text {
        alt.push(MimePart::new("text/plain", t.clone()));
    }
    if let Some(h) = &html {
        alt.push(MimePart::new("text/html", h.clone()));
    }
    if let Some(ics) = &sp.ics {
        alt.push(MimePart::new(
            mail_builder::headers::content_type::ContentType::new("text/calendar")
                .attribute("method", if ics.contains("METHOD:REPLY") { "REPLY" } else { "REQUEST" })
                .attribute("charset", "utf-8"),
            ics.clone(),
        ));
    }
    let mut body = if alt.len() == 1 { alt.pop().unwrap() } else { MimePart::new("multipart/alternative", alt) };
    if !sp.inline.is_empty() {
        let mut rel = vec![body];
        for (cid, mime, bytes) in &sp.inline {
            rel.push(MimePart::new(mime.clone(), bytes.clone()).inline().cid(cid.clone()));
        }
        body = MimePart::new("multipart/related", rel);
    }
    let mut atts: Vec<MimePart> =
        sp.atts.iter().map(|a| MimePart::new(a.mime.clone(), a.bytes.clone()).attachment(a.name.clone())).collect();
    if let Some(ics) = &sp.ics {
        atts.push(MimePart::new("application/ics", ics.clone().into_bytes()).attachment("invite.ics"));
    }
    if !atts.is_empty() {
        let mut parts = vec![body];
        parts.extend(atts);
        body = MimePart::new("multipart/mixed", parts);
    }
    b.body(body).write_to_string().unwrap_or_default()
}

// ---- content ----------------------------------------------------------------------------------

fn person(first: &str) -> &'static Person {
    PEOPLE.iter().find(|p| p.first == first).unwrap_or(&PEOPLE[0])
}

fn signature_text(p: &Who, title: &str, company: &str, phone: &str) -> String {
    format!("\n\nBest regards,\n{}\n{} | {}\nTel: {}\n", p.name, title, company, phone)
}

fn signature_html(p: &Person) -> String {
    format!(
        "<p style=\"margin-top:18px;font-family:Calibri,Arial,sans-serif;font-size:10pt;color:#444\">Best regards,<br><b>{}</b><br>{} | {}<br>Tel: {}</p>",
        p.name(),
        p.title,
        p.company,
        p.phone
    )
}

fn html_wrap(inner: &str) -> String {
    format!(
        "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body style=\"font-family:Calibri,Arial,sans-serif;font-size:11pt\">{inner}</body></html>"
    )
}

const COMPANIES: &[&str] = &["Orion Technologies", "Corvina Medical", "Sunfield Energy"];
const CONFERENCES: &[&str] = &[
    "Atlas TMT Conference (New York)",
    "Tel Aviv Tech Investor Day",
    "London Mid-Cap Forum",
    "Atlas Healthcare Summit (Boston)",
    "Frankfurt Small & Mid-Cap Week",
];

struct Ctx {
    me: Who,
    rng: Rng,
    now: i64,
    labels: HashMap<&'static str, String>,
    items: Vec<Item>,
    counter: u64,
    info: bool,
}

struct Item {
    date: i64,
    spec: Spec,
    labels: Vec<String>,
    thread: Option<usize>,
    reply_to: Option<usize>,
    draft: bool,
    msg_id: String,
}

impl Ctx {
    fn label(&self, k: &str) -> String {
        self.labels.get(k).cloned().unwrap_or_default()
    }

    fn msg_id(&mut self, domain: &str) -> String {
        self.counter += 1;
        format!("{:x}.{}.{}@{}", self.rng.next() & 0xffff_ffff, self.counter, self.rng.below(100000), domain)
    }

    /// A date `days_ago` back at a plausible working hour (never in the future).
    fn at(&mut self, days_ago: f64) -> i64 {
        let base = self.now - (days_ago * DAY_MS as f64) as i64;
        let local = chrono::Utc.timestamp_millis_opt(base).single().unwrap().with_timezone(&chrono_tz::Asia::Jerusalem);
        let hour = 7 + self.rng.below(12) as i64;
        let minute = self.rng.below(60) as i64;
        let t = base - (local.hour() as i64) * HOUR_MS - (local.minute() as i64) * 60_000 + hour * HOUR_MS + minute * 60_000;
        if t > self.now - 60_000 { self.now - (self.rng.below(40) as i64 + 2) * 60_000 } else { t }
    }

    /// Recency-weighted age in days (more mail recently).
    fn random_age(&mut self) -> f64 {
        let u = (self.rng.below(10_000) as f64) / 10_000.0;
        u.powf(2.2) * 180.0
    }

    fn inbound_flags(&mut self, date: i64) -> Vec<String> {
        let age = (self.now - date) as f64 / DAY_MS as f64;
        let mut l = Vec::new();
        let unread_pct = if age < 1.5 {
            70
        } else if age < 7.0 {
            25
        } else {
            3
        };
        if self.rng.chance(unread_pct) {
            l.push("UNREAD".into());
        }
        if self.rng.chance(6) {
            l.push("STARRED".into());
        }
        l
    }

    fn push(&mut self, date: i64, spec: Spec, labels: Vec<String>, thread: Option<usize>, reply_to: Option<usize>) -> usize {
        let domain = spec
            .from
            .as_ref()
            .map(|w| w.email.split('@').nth(1).unwrap_or("example.com").to_string())
            .unwrap_or_else(|| "example.com".into());
        let msg_id = self.msg_id(&domain);
        self.items.push(Item { date, spec, labels, thread, reply_to, draft: false, msg_id });
        self.items.len() - 1
    }

    /// Inbound message to me; `folder` = extra user label key; keeps INBOX unless archived.
    fn inbound(&mut self, date: i64, spec: Spec, folder: Option<&str>, thread: Option<usize>, reply_to: Option<usize>) -> usize {
        let mut labels = self.inbound_flags(date);
        let age = (self.now - date) / DAY_MS;
        let archived = folder.is_some() && age > 20 && self.rng.chance(55);
        if !archived {
            labels.push("INBOX".into());
        }
        if let Some(f) = folder {
            labels.push(self.label(f));
        }
        if self.rng.chance(30) {
            labels.push("IMPORTANT".into());
        }
        labels.push("CATEGORY_PERSONAL".into());
        self.push(date, spec, labels, thread, reply_to)
    }

    /// My reply in a thread (SENT, read).
    fn sent(&mut self, date: i64, spec: Spec, thread: Option<usize>, reply_to: Option<usize>) -> usize {
        self.push(date, spec, vec!["SENT".into()], thread, reply_to)
    }

    fn sig_me(&self) -> String {
        if self.info {
            "\n\nBest regards,\nDuvz Software\ninfo@duvz.demo | +972 3 555 0100\n".into()
        } else {
            "\n\nBest regards,\nKenny Green\nDuvz Software\nTel: +972 3 555 0101 | Mobile: +972 54 555 0101\n".into()
        }
    }
}

fn text_spec(from: Who, to: Vec<Who>, subject: &str, body: String) -> Spec {
    Spec { from: Some(from), to, subject: subject.into(), text: Some(body), ..Default::default() }
}

/// Client thread: draft earnings release / presentation with an attachment, my reply, their follow-up.
fn client_thread(c: &mut Ctx, age: f64) {
    let (company, people, folder) = match c.rng.below(3) {
        0 => ("Orion Technologies", [person("Noa"), person("Daniel")], "Clients/Orion Technologies"),
        1 => ("Corvina Medical", [person("Oren"), person("Rachel")], "Clients/Corvina Medical"),
        _ => ("Sunfield Energy", [person("Tamar"), person("Tamar")], "Clients"),
    };
    let p = *c.rng.pick(&people);
    let q = ["Q1", "Q2", "Q3", "Q4"][c.rng.below(4) as usize];
    let topic = c.rng.below(6);
    let (subject, body, att): (String, String, Option<Att>) = match topic {
        0 => (
            format!("{company} {q} 2026 earnings release - draft v{}", 2 + c.rng.below(4)),
            format!("Hi Kenny,\n\nAttached is the latest draft of the {q} press release. Legal has signed off on the forward-looking statements section; we still need your input on the highlights paragraph and the quote from the CEO.\n\nCould you send comments by tomorrow noon? We plan to file before market open on Thursday."),
            Some(Att { mime: "application/pdf".into(), name: format!("{} {q} 2026 Press Release DRAFT.pdf", company.split(' ').next().unwrap()), bytes: pdf(&format!("{company} - {q} 2026 Results (DRAFT)"), &["Revenue up 18% year over year", "Gross margin 61.4%", "Raising full-year guidance", "Conference call at 8:30 a.m. ET"]) }),
        ),
        1 => (
            format!("Investor presentation - {q} update"),
            "Kenny,\n\nPlease find the updated investor deck. Main changes are on slides 4-7 (market opportunity) and the new pipeline chart on slide 12.\n\nLet me know if the messaging works for the roadshow.".to_string(),
            Some(Att { mime: "application/pdf".into(), name: format!("{company} Investor Presentation {q} 2026.pdf"), bytes: pdf(&format!("{company} Investor Presentation"), &["Agenda", "Company overview", "Market opportunity", "Financial highlights", "Outlook"]) }),
        ),
        2 => (
            "Shareholder register - monthly update".to_string(),
            "Hi Kenny,\n\nAttached is this month's shareholder register analysis from the transfer agent. Notable changes: two new US holders and a reduction by one of the Israeli funds.\n\nWorth discussing on Sunday's call.".to_string(),
            Some(Att { mime: XLSX_MIME.into(), name: "Shareholder register analysis.xlsx".into(), bytes: xlsx(&[
                vec!["Holder".into(), "Shares".into(), "Change".into(), "% O/S".into()],
                vec!["Harbor Capital".into(), "1250000".into(), "150000".into(), "4.1".into()],
                vec!["Tamar Investments".into(), "980000".into(), "-120000".into(), "3.2".into()],
                vec!["Bluewater Fund".into(), "640000".into(), "640000".into(), "2.1".into()],
                vec!["Rhein Asset Management".into(), "410000".into(), "0".into(), "1.3".into()],
            ]) }),
        ),
        3 => (
            format!("{q} earnings call - Q&A prep"),
            "Kenny,\n\nHere is the list of likely analyst questions we put together. Can you add anything you heard from investors during the last NDR? Especially around gross margin and the China exposure.\n\nThanks".to_string(),
            None,
        ),
        4 => (
            "Website IR section - updates".to_string(),
            "Hi Kenny,\n\nThe web team asked which documents should be featured on the IR landing page after the results. Can you send the list?\n\nThanks".to_string(),
            None,
        ),
        _ => (
            "Board presentation - IR slide".to_string(),
            "Kenny,\n\nThe board meeting is next week. Could you prepare the usual IR slide: share price performance vs. peers, analyst coverage and top holder changes?\n\nThanks".to_string(),
            None,
        ),
    };
    let from = Who::of(p);
    let mut spec =
        text_spec(from.clone(), vec![c.me.clone()], &subject, format!("{body}{}", signature_text(&from, p.title, p.company, p.phone)));
    if c.rng.chance(40) {
        spec.cc = vec![Who::of(person("Maya"))];
    }
    if c.rng.chance(12) {
        spec.importance_high = true;
    }
    if let Some(a) = att {
        spec.atts.push(a);
    }
    // half of client mails come as HTML (Outlook-style) with an inline logo in the signature
    if c.rng.chance(45) {
        let txt = spec.text.clone().unwrap_or_default();
        let paras: String = body.split("\n\n").map(|para| format!("<p>{}</p>", para.replace('\n', "<br>"))).collect();
        spec.html = Some(html_wrap(&format!(
            "{paras}{}<p><img src=\"cid:logo-{}\" width=\"96\" height=\"28\" alt=\"{}\"></p>",
            signature_html(p),
            p.domain.split('.').next().unwrap(),
            p.company
        )));
        spec.inline.push((format!("logo-{}", p.domain.split('.').next().unwrap()), "image/png".into(), logo_png()));
        spec.text = Some(txt);
    }
    let d0 = c.at(age);
    let first = c.inbound(d0, spec, Some(folder), None, None);
    // my reply
    if c.rng.chance(65) {
        let d1 = d0 + (1 + c.rng.below(20) as i64) * HOUR_MS / 2;
        if d1 < c.now - 120_000 {
            let reply = format!(
                "Hi {},\n\nThanks - I went through it. A few comments:\n\n1. The highlights paragraph should lead with the guidance raise.\n2. Let's keep the CEO quote shorter (two sentences).\n3. Please double-check the share count in the table.\n\nOtherwise looks good to go.{}",
                p.first,
                c.sig_me()
            );
            let mut s = text_spec(c.me.clone(), vec![from.clone()], &format!("RE: {subject}"), reply);
            s.cc = c.items[first].spec.cc.clone();
            let r = c.sent(d1, s, Some(first), Some(first));
            // their follow-up
            if c.rng.chance(50) {
                let d2 = d1 + (1 + c.rng.below(10) as i64) * HOUR_MS / 2;
                if d2 < c.now - 60_000 {
                    let fu = format!(
                        "Thanks Kenny, all incorporated. Final version to follow.{}",
                        signature_text(&from, p.title, p.company, p.phone)
                    );
                    c.inbound(
                        d2,
                        text_spec(from.clone(), vec![c.me.clone()], &format!("RE: {subject}"), fu),
                        Some(folder),
                        Some(first),
                        Some(r),
                    );
                }
            }
        }
    }
}

fn investor_thread(c: &mut Ctx, age: f64) {
    let investors = ["Sarah", "David", "Michael", "Yael", "James", "Emily", "Thomas", "Anna", "Robert", "Sophie"];
    let p = person(c.rng.pick(&investors));
    let company = *c.rng.pick(COMPANIES);
    let conf = *c.rng.pick(CONFERENCES);
    let from = Who::of(p);
    let variants = [
        (
            format!("Meeting request: {company} management at {conf}"),
            format!(
                "Dear Kenny,\n\nWe would be interested in a 1x1 meeting with {company} management at the {conf}. Our team holds a position in the sector and would like to discuss the recent results and capital allocation.\n\nPlease let me know which slots are available."
            ),
        ),
        (
            format!("Question on {company} guidance"),
            "Hi Kenny,\n\nQuick question following the call: does the updated guidance include the contribution from the new contract announced last month, or is that upside?\n\nThanks in advance.".to_string(),
        ),
        (
            format!("{company} - follow-up from our meeting"),
            format!(
                "Kenny,\n\nThank you for arranging the meeting with {company} yesterday. Very helpful. Could you send the presentation and the model assumptions the CFO mentioned?\n\nMuch appreciated."
            ),
        ),
        (
            format!("Introduction - {}", p.company),
            format!(
                "Dear Kenny,\n\nI was given your details by a colleague. {} is looking at Israeli technology and healthcare names and we would welcome an introduction to your clients, in particular {company}.\n\nKind regards",
                p.company
            ),
        ),
        (
            "Non-deal roadshow - availability".to_string(),
            format!(
                "Hi Kenny,\n\nWe heard {company} may be on the road next month. If so, we would be glad to host them at our offices. Let me know the dates."
            ),
        ),
    ];
    let (subject, body) = c.rng.pick(&variants).clone();
    let mut spec =
        text_spec(from.clone(), vec![c.me.clone()], &subject, format!("{body}{}", signature_text(&from, p.title, p.company, p.phone)));
    if c.rng.chance(25) {
        spec.cc = vec![Who::new(&format!("{} Desk", p.company), &format!("research@{}", p.domain))];
    }
    let d0 = c.at(age);
    let first = c.inbound(d0, spec, Some("Investors"), None, None);
    if c.rng.chance(55) {
        let d1 = d0 + (2 + c.rng.below(30) as i64) * HOUR_MS / 3;
        if d1 < c.now - 120_000 {
            let reply = format!(
                "Dear {},\n\nThank you for your note. I will check with the company and get back to you shortly with availability.{}",
                p.first,
                c.sig_me()
            );
            c.sent(d1, text_spec(c.me.clone(), vec![from], &format!("RE: {subject}"), reply), Some(first), Some(first));
        }
    }
}

fn ics_for(uid: &str, organizer: &Who, me: &Who, summary: &str, location: &str, start_ms: i64, mins: i64, now: i64) -> String {
    let f = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).single().unwrap().format("%Y%m%dT%H%M%SZ").to_string();
    format!(
        "BEGIN:VCALENDAR\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\nVERSION:2.0\r\nCALSCALE:GREGORIAN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nDTSTART:{}\r\nDTEND:{}\r\nDTSTAMP:{}\r\nORGANIZER;CN={}:mailto:{}\r\nUID:{}\r\nATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN={}:mailto:{}\r\nCREATED:{}\r\nDESCRIPTION:{}\r\nLOCATION:{}\r\nSEQUENCE:0\r\nSTATUS:CONFIRMED\r\nSUMMARY:{}\r\nTRANSP:OPAQUE\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        f(start_ms),
        f(start_ms + mins * 60_000),
        f(now),
        organizer.name,
        organizer.email,
        uid,
        me.name,
        me.email,
        f(now),
        summary.replace(',', "\\,"),
        location.replace(',', "\\,"),
        summary.replace(',', "\\,")
    )
}

fn push_invite(c: &mut Ctx, org: Who, summary: &str, location: &str, start: i64, end: i64, uid: &str, sent: i64, unread: bool) {
    let mins = ((end - start) / 60_000).max(15);
    let ics = ics_for(uid, &org, &c.me, summary, location, start, mins, c.now);
    let start_local = chrono::Utc.timestamp_millis_opt(start).single().unwrap().with_timezone(&chrono_tz::Asia::Jerusalem);
    let when = start_local.format("%A, %B %-d, %Y %-I:%M %p").to_string();
    let text = format!(
        "{} has invited you to the following meeting.

{summary}
When: {when} (Jerusalem)
Where: {location}
Organizer: {} <{}>

Going? Yes - Maybe - No
",
        org.name, org.name, org.email
    );
    let html = html_wrap(&format!(
        "<table cellpadding=\"4\" style=\"border:1px solid #c7d6ea;font-size:10pt\"><tr><td colspan=\"2\" style=\"background:#e8f0fb;font-weight:bold\">{summary}</td></tr><tr><td><b>When</b></td><td>{when} (Jerusalem)</td></tr><tr><td><b>Where</b></td><td>{location}</td></tr><tr><td><b>Organizer</b></td><td>{} &lt;{}&gt;</td></tr></table>",
        org.name, org.email
    ));
    let spec = Spec {
        from: Some(org.clone()),
        to: vec![c.me.clone()],
        subject: format!("Invitation: {summary} @ {}", start_local.format("%a %b %-d, %Y %-I%P")),
        text: Some(text),
        html: Some(html),
        ics: Some(ics),
        ..Default::default()
    };
    let mut labels = vec!["INBOX".to_string(), "CATEGORY_PERSONAL".to_string(), "IMPORTANT".to_string()];
    if unread {
        labels.push("UNREAD".into());
    }
    c.push(sent, spec, labels, None, None);
}

fn parse_event_time(t: &Option<crate::google::calendar::EventDateTime>) -> Option<i64> {
    let t = t.as_ref()?;
    if let Some(dt) = &t.date_time {
        return chrono::DateTime::parse_from_rfc3339(dt).ok().map(|d| d.timestamp_millis());
    }
    let d = chrono::NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

/// An attendee's answer to one of your meetings ("Accepted: …", METHOD:REPLY): nothing to respond to.
fn meeting_reply(c: &mut Ctx) {
    let p = person("Michael");
    let from = Who::of(p);
    let f = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).single().unwrap().format("%Y%m%dT%H%M%SZ").to_string();
    let start = c.now + DAY_MS;
    let ics = format!(
        "BEGIN:VCALENDAR\r\nPRODID:Microsoft Exchange Server 2010\r\nVERSION:2.0\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nATTENDEE;PARTSTAT=ACCEPTED;CN={}:mailto:{}\r\nSUMMARY;LANGUAGE=en-US:Accepted: Orion weekly IR call update\r\nDTSTART:{}\r\nDTEND:{}\r\nUID:040000008200E00074C5B7101A82E00800000000ORIONWEEKLY\r\nDTSTAMP:{}\r\nLOCATION:https://zoom.us/j/8125550147?pwd=dEx4QmZh\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        from.name,
        from.email,
        f(start),
        f(start + 30 * 60_000),
        f(c.now)
    );
    let spec = Spec {
        from: Some(from),
        to: vec![c.me.clone()],
        subject: "Accepted: Orion weekly IR call update".into(),
        text: Some("See you then.".into()),
        ics: Some(ics),
        ..Default::default()
    };
    let sent = c.at(0.3);
    c.push(sent, spec, vec!["INBOX".into(), "CATEGORY_PERSONAL".into(), "UNREAD".into()], None, None);
}

/// Meeting requests (text/calendar METHOD:REQUEST) for the invitations the fake Calendar seeds, so
/// Accept / Decline in the reading pane finds the matching event (same iCalUID).
fn meeting_requests(c: &mut Ctx, name: &str) {
    let email = c.me.email.clone();
    let invites = crate::fake_google::calendar::seeded_invites(&email, name);
    let mut n = 0;
    for e in invites {
        let (Some(uid), Some(start)) = (e.ical_uid.clone(), parse_event_time(&e.start)) else { continue };
        let end = parse_event_time(&e.end).unwrap_or(start + 3_600_000);
        let org = e
            .organizer
            .as_ref()
            .map(|o| Who::new(o.display_name.as_deref().unwrap_or(""), o.email.as_deref().unwrap_or("")))
            .unwrap_or_else(|| Who::of(person("Sarah")));
        // sent a few days before the meeting (and never in the future)
        let age = (((c.now - start) as f64 / DAY_MS as f64) + 3.0 + n as f64 * 0.7).max(0.05 + n as f64 * 0.3);
        let sent = c.at(age);
        let unread = age < 2.0;
        push_invite(c, org, e.summary.as_deref().unwrap_or("Meeting"), e.location.as_deref().unwrap_or(""), start, end, &uid, sent, unread);
        n += 1;
    }
    if n > 0 {
        return;
    }
    let reqs: [(&str, &str, &str, f64, i64, i64); 4] = [
        ("Sarah", "Orion Technologies - Q3 results follow-up call", "Zoom (link in description)", 0.2, 3, 45),
        ("Michael", "Northgate / Corvina Medical management meeting", "Northgate Advisors, 12 Bishopsgate, London", 1.3, 6, 60),
        ("Chris", "Atlas TMT Conference - IR coordination call", "Microsoft Teams", 4.0, 2, 30),
        ("Yael", "Tamar Investments - quarterly review", "Tamar Investments, Rothschild Blvd 22, Tel Aviv", 9.0, 12, 60),
    ];
    for (i, (first, summary, location, age, days_ahead, mins)) in reqs.iter().enumerate() {
        let org = Who::of(person(first));
        let sent = c.at(*age);
        let day =
            chrono::Utc.timestamp_millis_opt(c.now + days_ahead * DAY_MS).single().unwrap().with_timezone(&chrono_tz::Asia::Jerusalem);
        let start = chrono_tz::Asia::Jerusalem
            .with_ymd_and_hms(
                chrono::Datelike::year(&day),
                chrono::Datelike::month(&day),
                chrono::Datelike::day(&day),
                10 + (i as u32 * 2) % 7,
                0,
                0,
            )
            .single()
            .unwrap()
            .timestamp_millis();
        push_invite(c, org, summary, location, start, start + mins * 60_000, &format!("cm-invite-{}@duvz.demo", i + 1), sent, *age < 2.0);
    }
}

fn conference_mail(c: &mut Ctx, age: f64) {
    let p = person("Chris");
    let from = Who::of(p);
    let conf = *c.rng.pick(CONFERENCES);
    let variants = [
        (format!("{conf} - your 1x1 schedule"), "Hi Kenny,\n\nAttached is the current 1x1 schedule for your companies. Please review and confirm by Friday; requests received after that will be waitlisted.".to_string(), true),
        (format!("{conf} - hotel block closing soon"), "Dear Kenny,\n\nA reminder that the discounted hotel block for the conference closes next week. Use the group code ATLAS26 when booking.".to_string(), false),
        (format!("{conf} - presentation slot confirmed"), "Hi Kenny,\n\nGood news - your client's presentation slot is confirmed for Day 1 at 2:40 pm in Room B. Please send the final deck 48 hours in advance.".to_string(), false),
        (format!("Registration confirmation - {conf}"), "Thank you for registering. Your badge will be available at the registration desk. Please bring photo ID.".to_string(), false),
    ];
    let (subject, body, with_xlsx) = c.rng.pick(&variants).clone();
    let mut spec =
        text_spec(from.clone(), vec![c.me.clone()], &subject, format!("{body}{}", signature_text(&from, p.title, p.company, p.phone)));
    if with_xlsx {
        spec.atts.push(Att {
            mime: XLSX_MIME.into(),
            name: "1x1 schedule.xlsx".into(),
            bytes: xlsx(&[
                vec!["Time".into(), "Company".into(), "Investor".into(), "Room".into()],
                vec!["09:00".into(), "Orion Technologies".into(), "Harbor Capital".into(), "12A".into()],
                vec!["09:45".into(), "Orion Technologies".into(), "Bluewater Fund".into(), "12A".into()],
                vec!["10:30".into(), "Corvina Medical".into(), "Northgate Advisors".into(), "14C".into()],
                vec!["11:15".into(), "Corvina Medical".into(), "Summit Securities".into(), "14C".into()],
            ]),
        });
    }
    let d = c.at(age);
    c.inbound(d, spec, Some("Conferences"), None, None);
}

fn internal_mail(c: &mut Ctx, age: f64) {
    let (p, subject, body) = match c.rng.below(4) {
        0 => (
            person("Maya"),
            format!("Investor targeting list - {}", c.rng.pick(COMPANIES)),
            "Hi Kenny,\n\nI finished the targeting list for the US roadshow - 42 funds, ranked by sector exposure and recent activity. Attached. The top 15 are highlighted.",
        ),
        1 => (
            person("Maya"),
            "Weekly IR activity report".to_string(),
            "Kenny,\n\nThe weekly report is attached: 9 investor meetings, 3 new analyst inquiries, website traffic up 12% after the results.",
        ),
        2 => (
            person("Tom"),
            "Birchwood Partners - monthly sync".to_string(),
            "Kenny,\n\nCan we do our monthly catch-up on Tuesday at 17:00? I'd like to go over the new client proposals and the invoicing for Q3.",
        ),
        _ => (
            person("Tom"),
            "Proposal for new client - draft".to_string(),
            "Hi Kenny,\n\nI drafted the proposal for the new medical device client. Please review the fee section before I send it.",
        ),
    };
    let from = Who::of(p);
    let mut spec =
        text_spec(from.clone(), vec![c.me.clone()], &subject, format!("{body}{}", signature_text(&from, p.title, p.company, p.phone)));
    if subject.starts_with("Investor targeting") || subject.starts_with("Weekly") {
        spec.atts.push(Att {
            mime: XLSX_MIME.into(),
            name: if subject.starts_with("Weekly") { "IR activity report.xlsx".into() } else { "Targeting list.xlsx".into() },
            bytes: xlsx(&[
                vec!["Fund".into(), "City".into(), "AUM ($bn)".into(), "Priority".into()],
                vec!["Harbor Capital".into(), "New York".into(), "42".into(), "A".into()],
                vec!["Bluewater Fund".into(), "Boston".into(), "18".into(), "A".into()],
                vec!["Eastbridge Research".into(), "Chicago".into(), "7".into(), "B".into()],
            ]),
        });
    }
    let d = c.at(age);
    c.inbound(d, spec, None, None, None);
}

const HEB_SUBJECTS: &[(&str, &str)] = &[
    (
        "סיכום פגישה עם המשקיעים",
        "קני שלום,\n\nמצרף סיכום קצר של הפגישה מאתמול:\n1. המשקיעים מרוצים מהתוצאות הרבעוניות.\n2. ביקשו פירוט נוסף על תחזית הרווחיות.\n3. נקבעה פגישת המשך בעוד שבועיים.\n\nתודה רבה,",
    ),
    ("דוח רבעוני - טיוטה לעיון", "היי קני,\n\nמצורפת טיוטת הדוח הרבעוני. נשמח להערות עד יום חמישי.\n\nבברכה,"),
    ("כנס משקיעים בתל אביב - לוח זמנים", "שלום קני,\n\nלוח הזמנים לכנס מצורף. הפגישות עם המשקיעים יתקיימו בקומה 3.\n\nתודה,"),
    ("שאלה לגבי ההנפקה", "קני,\n\nרציתי לשאול האם יש עדכון לגבי מועד ההנפקה? מספר משקיעים פנו אלינו.\n\nתודה מראש,"),
    ("Re: פגישה ביום ראשון", "מעולה, נתראה ביום ראשון בעשר במשרד.\n\nלהתראות,"),
];

fn hebrew_mail(c: &mut Ctx, age: f64) {
    let people = ["Yael", "Lior", "Avi", "Tamar", "Oren"];
    let p = person(c.rng.pick(&people));
    let (subject, body) = *c.rng.pick(HEB_SUBJECTS);
    let from = Who::of(p);
    let heb_name = match p.first {
        "Yael" => "יעל פרידמן",
        "Lior" => "ליאור בן דוד",
        "Avi" => "אבי רוזן",
        "Tamar" => "תמר פרץ",
        _ => "אורן כץ",
    };
    let from = Who::new(heb_name, &from.email);
    let mut spec = text_spec(from.clone(), vec![c.me.clone()], subject, format!("{body}\n{heb_name}\n{}\n", p.company));
    if c.rng.chance(50) {
        let paras: String = body.split("\n\n").map(|para| format!("<p>{}</p>", para.replace('\n', "<br>"))).collect();
        spec.html = Some(format!(
            "<html><body dir=\"rtl\" style=\"font-family:Arial,sans-serif;font-size:11pt\">{paras}<p>{heb_name}<br>{}</p></body></html>",
            p.company
        ));
    }
    if subject.starts_with("דוח") {
        spec.atts.push(Att {
            mime: "application/pdf".into(),
            name: "דוח רבעוני - טיוטה.pdf".into(),
            bytes: pdf("Quarterly report - draft", &["Hebrew report (demo)"]),
        });
    }
    let d = c.at(age);
    c.inbound(d, spec, None, None, None);
}

/// A plain-text message in windows-1255 and an HTML one in iso-8859-8 (quoted-printable).
fn legacy_hebrew(c: &mut Ctx) {
    let me = c.me.clone();
    let wrap76 = |s: String| s.as_bytes().chunks(76).map(|ch| String::from_utf8_lossy(ch).into_owned()).collect::<Vec<_>>().join("\r\n");
    let enc = |s: &str| encoding_rs::WINDOWS_1255.encode(s).0.into_owned();
    // 1. windows-1255 / base64
    {
        let subj = format!("=?windows-1255?B?{}?=", STANDARD.encode(enc("הצעת מחיר לשירותי קשרי משקיעים")));
        let from_name = format!("=?windows-1255?B?{}?=", STANDARD.encode(enc("אבי רוזן")));
        let body = "שלום קני,\r\n\r\nבהמשך לשיחתנו, מצורפת הצעת המחיר לשירותי קשרי משקיעים לשנת 2027.\r\nהמחירים כוללים מע\"מ.\r\n\r\nבברכה,\r\nאבי רוזן\r\nירושלים אקוויטי\r\n";
        let date = c.at(0.6);
        let id = c.msg_id("jlmequity.example");
        let raw = format!(
            "From: {from_name} <avi.rosen@jlmequity.example>\r\nTo: {} <{}>\r\nSubject: {subj}\r\nDate: {}\r\nMessage-ID: <{id}>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=\"windows-1255\"\r\nContent-Transfer-Encoding: base64\r\nX-Mailer: Microsoft Outlook 12.0\r\n\r\n{}\r\n",
            me.name,
            me.email,
            rfc2822(date),
            wrap76(STANDARD.encode(enc(body)))
        );
        let spec = Spec { raw: Some(raw), ..Default::default() };
        let labels = vec!["INBOX".into(), "UNREAD".into(), "CATEGORY_PERSONAL".into()];
        c.push(date, spec, labels, None, None);
    }
    // 2. iso-8859-8 / quoted-printable HTML
    {
        let qp = |s: &str| -> String {
            let bytes = encoding_rs::WINDOWS_1255.encode(s).0.into_owned();
            let mut out = String::new();
            let mut line = 0;
            for b in bytes {
                let piece = if b == b'\n' {
                    line = 0;
                    "\r\n".to_string()
                } else if b == b'\r' {
                    continue;
                } else if (33..=126).contains(&b) && b != b'=' || b == b' ' {
                    (b as char).to_string()
                } else {
                    format!("={b:02X}")
                };
                if line + piece.len() > 73 && piece != "\r\n" {
                    out.push_str("=\r\n");
                    line = 0;
                }
                line += piece.len();
                out.push_str(&piece);
            }
            out
        };
        let html = "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-8\"></head>\n<body dir=\"rtl\"><p>קני שלום,</p>\n<p>אנו שמחים להזמין אותך לאירוע ההשקה של הקרן החדשה שלנו, שיתקיים ביום רביעי הבא במלון דן תל אביב.</p>\n<p>נא לאשר הגעה.</p>\n<p>בברכה,<br>יעל פרידמן<br>תמר השקעות</p></body></html>\n";
        let subj = format!("=?iso-8859-8?Q?{}?=", qp("הזמנה לאירוע השקה").replace(' ', "_").replace("=\r\n", ""));
        let date = c.at(2.4);
        let id = c.msg_id("tamarinv.example");
        let raw = format!(
            "From: Yael Friedman <yael.friedman@tamarinv.example>\r\nTo: {} <{}>\r\nSubject: {subj}\r\nDate: {}\r\nMessage-ID: <{id}>\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=iso-8859-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{}\r\n",
            me.name,
            me.email,
            rfc2822(date),
            qp(html)
        );
        let spec = Spec { raw: Some(raw), ..Default::default() };
        c.push(date, spec, vec!["INBOX".into(), "CATEGORY_PERSONAL".into()], None, None);
    }
}

fn newsletter_html(title: &str, color: &str, host: &str, issue: u64, big: bool, inline_cid: Option<&str>) -> String {
    let mut articles = String::new();
    let heads = [
        (
            "TASE closes higher as tech rallies",
            "The TA-35 index gained 1.2% led by technology and banking shares, while bond yields eased after the central bank decision.",
        ),
        (
            "IPO window reopens for Israeli issuers",
            "Three dual-listed companies filed confidentially this month, according to market sources, signalling renewed appetite from US investors.",
        ),
        (
            "Earnings season: what investors want to hear",
            "Guidance, margins and capital allocation top the list of investor questions, according to our survey of 120 portfolio managers.",
        ),
        (
            "ESG reporting: new disclosure requirements",
            "Regulators published draft rules that would require mid-cap issuers to report climate-related risks from 2028.",
        ),
        (
            "Shekel steady ahead of rate decision",
            "The shekel traded in a narrow range against the dollar as markets priced in a hold at next week's meeting.",
        ),
    ];
    let repeat = if big { 14 } else { 1 };
    for r in 0..repeat {
        for (i, (h, t)) in heads.iter().enumerate() {
            articles.push_str(&format!(
                "<tr><td style=\"padding:12px 18px;border-bottom:1px solid #e3e8ef\"><table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr>\
                 <td width=\"120\" valign=\"top\"><img src=\"https://{host}/img/{issue}/story{}.jpg\" width=\"110\" height=\"74\" alt=\"\" style=\"display:block;border:0\"></td>\
                 <td valign=\"top\" style=\"font-family:Georgia,serif\"><a href=\"https://{host}/story/{issue}-{}\" style=\"color:{color};font-size:16px;font-weight:bold;text-decoration:none\">{h}</a>\
                 <p style=\"margin:6px 0 0;font:13px/1.5 Arial,sans-serif;color:#333\">{t}</p></td></tr></table></td></tr>",
                i + r * 5,
                i + r * 5
            ));
        }
    }
    let chart = match inline_cid {
        Some(cid) => format!(
            "<tr><td style=\"padding:12px 18px\"><b style=\"font:13px Arial\">Index performance this week</b><br><img src=\"cid:{cid}\" width=\"140\" height=\"60\" alt=\"chart\"></td></tr>"
        ),
        None => String::new(),
    };
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         body{{margin:0;background:#f2f4f7}} .wrap{{width:640px;margin:0 auto;background:#ffffff}} .hdr td{{background:{color};color:#fff}} a.btn{{background:{color};color:#fff;padding:8px 16px;border-radius:3px;text-decoration:none}}\
         </style></head><body><img src=\"https://{host}/open.gif?u=kenny&i={issue}\" width=\"1\" height=\"1\" alt=\"\">\
         <table class=\"wrap\" width=\"640\" align=\"center\" cellpadding=\"0\" cellspacing=\"0\" style=\"background:#fff\">\
         <tr class=\"hdr\"><td style=\"padding:0\"><img src=\"https://{host}/img/header.png\" width=\"640\" height=\"90\" alt=\"{title}\" style=\"display:block\"></td></tr>\
         <tr><td style=\"padding:14px 18px;font:bold 20px Georgia,serif;color:#222\">{title} &middot; Issue {issue}</td></tr>\
         {chart}{articles}\
         <tr><td style=\"padding:18px;text-align:center\"><a class=\"btn\" href=\"https://{host}/read\">Read the full issue</a></td></tr>\
         <tr><td style=\"padding:14px 18px;font:11px Arial;color:#888;background:#f7f8fa\">You are receiving this because you subscribed to {title}. \
         <a href=\"https://{host}/unsubscribe\" style=\"color:#888\">Unsubscribe</a> | <a href=\"https://{host}/preferences\" style=\"color:#888\">Preferences</a></td></tr>\
         </table></body></html>"
    )
}

fn newsletter(c: &mut Ctx, age: f64, issue: u64) {
    let pubs = [
        ("Capital Markets Weekly", "news@capmarkets-weekly.example", "#1d4f91", "media.capmarkets-weekly.example"),
        ("TASE Morning Brief", "brief@tase-brief.example", "#0b7a5f", "cdn.tase-brief.example"),
        ("IR Insight Magazine", "editors@irinsight.example", "#8a2b52", "img.irinsight.example"),
    ];
    let (title, addr, color, host) = *c.rng.pick(&pubs);
    let big = issue % 7 == 3;
    let with_chart = issue % 5 == 1;
    let cid = format!("chart{issue}@{}", host);
    let html = newsletter_html(title, color, host, issue, big, with_chart.then_some(cid.as_str()));
    let text = format!(
        "{title} - Issue {issue}\n\nTASE closes higher as tech rallies\nIPO window reopens for Israeli issuers\nEarnings season: what investors want to hear\n\nRead online: https://{host}/read\nUnsubscribe: https://{host}/unsubscribe\n"
    );
    let mut spec = Spec {
        from: Some(Who::new(title, addr)),
        to: vec![c.me.clone()],
        subject: format!(
            "{title}: {}",
            ["Markets wrap", "IPO window reopens", "Earnings season preview", "ESG rules ahead", "Week in review"][(issue % 5) as usize]
        ),
        text: Some(text),
        html: Some(html),
        headers: vec![("List-Unsubscribe".into(), format!("<https://{host}/unsubscribe>")), ("Precedence".into(), "bulk".into())],
        ..Default::default()
    };
    if with_chart {
        spec.inline.push((cid, "image/png".into(), chart_png(issue as u32)));
    }
    let d = c.at(age);
    let mut labels = vec!["CATEGORY_UPDATES".to_string()];
    let a = (c.now - d) / DAY_MS;
    if a < 3 && c.rng.chance(60) || a < 1 {
        labels.push("UNREAD".into());
    }
    if a < 45 || c.rng.chance(30) {
        labels.push("INBOX".into());
    }
    c.push(d, spec, labels, None, None);
}

fn receipt(c: &mut Ctx, age: f64, n: u64) {
    let vendors = [
        ("CloudMeet Pro", "billing@cloudmeet.example", "CloudMeet Pro - Business plan (monthly)", "39.00"),
        ("SkyRail Tickets", "receipts@skyrail.example", "Tel Aviv - Jerusalem, return", "47.50"),
        ("DocuStream", "invoices@docustream.example", "DocuStream Team - 3 seats", "75.00"),
        ("PrintHub", "orders@printhub.example", "Business cards x500", "120.00"),
        ("Atlas Conferences", "registration@atlasconf.example", "Conference pass - Atlas TMT Conference", "1450.00"),
    ];
    let (vendor, addr, item, amount) = *c.rng.pick(&vendors);
    let d = c.at(age);
    let date_s = chrono::Utc.timestamp_millis_opt(d).single().unwrap().format("%B %-d, %Y").to_string();
    let html = html_wrap(&format!(
        "<div style=\"max-width:560px;border:1px solid #ddd;padding:20px\"><h2 style=\"margin:0 0 10px;color:#333\">Receipt from {vendor}</h2>\
         <p style=\"color:#666\">Receipt #{n:06} &middot; {date_s}</p>\
         <table width=\"100%\" cellpadding=\"6\" style=\"border-collapse:collapse;font-size:14px\">\
         <tr style=\"background:#f4f4f4\"><th align=\"left\">Description</th><th align=\"right\">Amount</th></tr>\
         <tr><td style=\"border-bottom:1px solid #eee\">{item}</td><td align=\"right\" style=\"border-bottom:1px solid #eee\">${amount}</td></tr>\
         <tr><td align=\"right\"><b>Total</b></td><td align=\"right\"><b>${amount}</b></td></tr></table>\
         <p style=\"font-size:12px;color:#888\">Paid with card ending 4242. Questions? Reply to this e-mail.</p></div>"
    ));
    let spec = Spec {
        from: Some(Who::new(vendor, addr)),
        to: vec![c.me.clone()],
        subject: format!("Your receipt from {vendor} #{n:06}"),
        text: Some(format!("Receipt from {vendor}\nReceipt #{n:06} - {date_s}\n{item}: ${amount}\nTotal: ${amount}\n")),
        html: Some(html),
        atts: if n.is_multiple_of(2) {
            vec![Att {
                mime: "application/pdf".into(),
                name: format!("Receipt-{n:06}.pdf"),
                bytes: pdf(&format!("{vendor} - Receipt #{n:06}"), &[item, &format!("Total: USD {amount}")]),
            }]
        } else {
            vec![]
        },
        ..Default::default()
    };
    let mut labels = vec![c.label("Receipts"), "CATEGORY_UPDATES".to_string()];
    if (c.now - d) < 10 * DAY_MS {
        labels.push("INBOX".into());
    }
    c.push(d, spec, labels, None, None);
}

fn social(c: &mut Ctx, age: f64) {
    let subjects = [
        "You appeared in 14 searches this week",
        "Daniel Mizrahi commented on your post",
        "New jobs matching your profile",
        "Your network update",
    ];
    let s = *c.rng.pick(&subjects);
    let html = html_wrap(&format!(
        "<p><img src=\"https://static.pronet.example/logo.png\" width=\"90\" height=\"24\" alt=\"ProNet\"></p><h3>{s}</h3><p><a href=\"https://pronet.example/notifications\">See all activity</a></p>"
    ));
    let spec = Spec {
        from: Some(Who::new("ProNet", "notifications@pronet.example")),
        to: vec![c.me.clone()],
        subject: s.into(),
        html: Some(html),
        ..Default::default()
    };
    let d = c.at(age);
    let mut labels = vec!["CATEGORY_SOCIAL".to_string()];
    if (c.now - d) < 20 * DAY_MS {
        labels.push("INBOX".into());
    }
    c.push(d, spec, labels, None, None);
}

fn spam(c: &mut Ctx, age: f64) {
    let subjects = [
        "You have won a $1,000 gift card!!!",
        "URGENT: verify your account now",
        "Cheap meds - 90% off",
        "Investment opportunity: guaranteed 40% returns",
    ];
    let s = *c.rng.pick(&subjects);
    let spec = text_spec(
        Who::new("Prize Center", "winner@promo-deals.example"),
        vec![c.me.clone()],
        s,
        "Click here to claim: http://promo-deals.example/claim\n".into(),
    );
    let d = c.at(age);
    c.push(d, spec, vec!["SPAM".into(), "UNREAD".into()], None, None);
}

fn sent_standalone(c: &mut Ctx, age: f64) {
    let investors = ["Sarah", "James", "Thomas", "Robert", "Emily"];
    let p = person(c.rng.pick(&investors));
    let company = *c.rng.pick(COMPANIES);
    let subj = format!("{company} - materials from today's call");
    let body = format!(
        "Dear {},\n\nThank you for joining the call with {company} today. As promised, please find the presentation link below:\nhttps://ir.{}.example/presentations\n\nHappy to set up a follow-up with the CFO.{}",
        p.first,
        company.split(' ').next().unwrap().to_lowercase(),
        c.sig_me()
    );
    let mut spec = text_spec(c.me.clone(), vec![Who::of(p)], &subj, body);
    if c.rng.chance(30) {
        spec.cc = vec![Who::of(person("Maya"))];
    }
    let d = c.at(age);
    c.sent(d, spec, None, None);
}

fn info_inquiry(c: &mut Ctx, age: f64) {
    let names = [
        ("Jonathan Price", "j.price@quartz-capital.example"),
        ("Hila Mor", "hila.mor@mor-advisors.example"),
        ("Peter Novak", "pnovak@danube-invest.example"),
        ("Mei Lin", "mei.lin@pacificbridge.example"),
        ("רונית שגיא", "ronit@sagi-ventures.example"),
    ];
    let (n, e) = *c.rng.pick(&names);
    let heb = n.chars().any(|ch| ('\u{0590}'..='\u{05FF}').contains(&ch));
    let (subject, body) = if heb {
        (
            "פנייה מאתר האינטרנט",
            format!("שם: {n}\nדוא\"ל: {e}\nהודעה: שלום, אשמח לקבל פרטים על שירותי קשרי המשקיעים שלכם עבור חברה בתחום הסייבר.\n"),
        )
    } else {
        (
            "Inquiry from website contact form",
            format!(
                "Name: {n}\nE-mail: {e}\nCompany: {}\nMessage: Hello, we are an investment firm interested in meeting management of your client companies during our Israel trip next month. Please advise.\n",
                e.split('@').nth(1).unwrap().split('.').next().unwrap()
            ),
        )
    };
    let spec = Spec {
        from: Some(Who::new("Duvz Software Website", "noreply@duvz.example")),
        to: vec![c.me.clone()],
        subject: subject.into(),
        text: Some(body),
        headers: vec![("Reply-To".into(), format!("{n} <{e}>"))],
        ..Default::default()
    };
    let d = c.at(age);
    let folder = if c.rng.chance(30) { Some("Investors") } else { Some("Website") };
    c.inbound(d, spec, folder, None, None);
}

fn drafts(c: &mut Ctx) {
    let p = person("Sarah");
    let s1 = text_spec(
        c.me.clone(),
        vec![Who::of(p)],
        "Orion - proposed meeting slots",
        format!(
            "Hi Sarah,\n\nThe CFO is available on Tuesday at 10:00 or Wednesday at 15:00 (NY time). Let me know what works{}",
            c.sig_me()
        ),
    );
    let d1 = c.at(0.1);
    let i = c.push(d1, s1, vec!["DRAFT".into()], None, None);
    c.items[i].draft = true;
    let s2 = text_spec(c.me.clone(), vec![Who::of(person("Tom"))], "Q4 planning", "Tom,\n\nSome thoughts for Q4:\n- \n".to_string());
    let d2 = c.at(3.0);
    let i = c.push(d2, s2, vec!["DRAFT".into()], None, None);
    c.items[i].draft = true;
}

fn trash(c: &mut Ctx) {
    for i in 0..4 {
        let age = 2.0 + i as f64 * 9.0;
        let spec = text_spec(
            Who::new("Office Supplies Co.", "sales@officesupplies.example"),
            vec![c.me.clone()],
            &format!("Autumn sale - {}% off toner", 20 + i * 5),
            "Stock up now and save.\n".into(),
        );
        let d = c.at(age);
        c.push(d, spec, vec!["TRASH".into(), "CATEGORY_PROMOTIONS".into()], None, None);
    }
}

pub fn seed(email: &str, name: &str) -> Store {
    // Test builds: other domains' tests (compose) expect a fresh, empty mailbox; mail-core's own
    // tests ask for the demo mailbox with a "+seed" address.
    if cfg!(test) && !email.contains("+seed") {
        return Store::empty(email, name);
    }
    rich_seed(email, name)
}

fn rich_seed(email: &str, name: &str) -> Store {
    let mut store = Store::empty(email, name);
    let now = now_ms();
    let info = email.starts_with("info@") || email.starts_with("info+");
    let mut labels = HashMap::new();
    for l in ["Clients", "Clients/Orion Technologies", "Clients/Corvina Medical", "Investors", "Conferences", "Receipts"] {
        labels.insert(l, store.ensure_label(l));
    }
    if info {
        labels.insert("Website", store.ensure_label("Website"));
    }
    let me = Who::new(name, email);
    let mut c = Ctx { me, rng: Rng::for_key(&format!("gmail:{email}")), now, labels, items: Vec::new(), counter: 0, info };

    // guaranteed recent mail: several today and yesterday
    for k in 0..6 {
        let age = 0.02 + k as f64 * 0.06;
        match k % 3 {
            0 => client_thread(&mut c, age),
            1 => investor_thread(&mut c, age),
            _ => internal_mail(&mut c, age),
        }
    }
    for k in 0..5 {
        let age = 1.0 + k as f64 * 0.12;
        match k % 2 {
            0 => client_thread(&mut c, age),
            _ => hebrew_mail(&mut c, age),
        }
    }
    meeting_requests(&mut c, name);
    meeting_reply(&mut c);
    legacy_hebrew(&mut c);
    drafts(&mut c);
    trash(&mut c);
    for k in 0..3 {
        let age = 3.0 + k as f64 * 40.0;
        spam(&mut c, age);
    }
    let target = if info { 380 } else { 410 };
    let mut issue = 100;
    let mut receipt_no = 4100;
    while c.items.len() < target {
        let age = c.random_age();
        let roll = c.rng.below(100);
        if info {
            match roll {
                0..=29 => info_inquiry(&mut c, age),
                30..=44 => {
                    issue += 1;
                    newsletter(&mut c, age, issue)
                }
                45..=54 => conference_mail(&mut c, age),
                55..=64 => investor_thread(&mut c, age),
                65..=74 => hebrew_mail(&mut c, age),
                75..=84 => {
                    receipt_no += 1;
                    receipt(&mut c, age, receipt_no)
                }
                85..=92 => client_thread(&mut c, age),
                _ => social(&mut c, age),
            }
        } else {
            match roll {
                0..=21 => client_thread(&mut c, age),
                22..=37 => investor_thread(&mut c, age),
                38..=47 => {
                    issue += 1;
                    newsletter(&mut c, age, issue)
                }
                48..=55 => conference_mail(&mut c, age),
                56..=64 => internal_mail(&mut c, age),
                65..=72 => hebrew_mail(&mut c, age),
                73..=79 => {
                    receipt_no += 1;
                    receipt(&mut c, age, receipt_no)
                }
                80..=86 => sent_standalone(&mut c, age),
                87..=92 => social(&mut c, age),
                _ => {
                    if c.rng.chance(15) {
                        spam(&mut c, age)
                    } else {
                        investor_thread(&mut c, age)
                    }
                }
            }
        }
    }
    let items = std::mem::take(&mut c.items);
    let labels = c.labels.clone();
    commit(&mut store, items);
    // a Hebrew label and one with quotes (IMAP modified UTF-7 / quoting in the fast download)
    let investors = labels.get("Investors").cloned().unwrap_or_default();
    let conferences = labels.get("Conferences").cloned().unwrap_or_default();
    let hebrew = store.ensure_label("לקוחות/ישראל");
    let quoted = store.ensure_label("Conferences/\"Tel Aviv\" 2026");
    let ids: Vec<(String, bool, bool)> =
        store.messages.values().map(|m| (m.id.clone(), m.label_ids.contains(&investors), m.label_ids.contains(&conferences))).collect();
    for (i, (id, inv, conf)) in ids.into_iter().enumerate() {
        let m = store.messages.get_mut(&id).expect("message");
        if inv && i % 3 == 0 {
            m.label_ids.push(hebrew.clone());
        }
        if conf && i % 2 == 0 {
            m.label_ids.push(quoted.clone());
        }
    }
    store
}

/// Insert items oldest first (so history ids follow time), wiring threads and reply headers.
fn commit(store: &mut Store, items: Vec<Item>) {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| (items[i].date, i));
    let mut thread_ids: HashMap<usize, String> = HashMap::new();
    for i in order {
        let it = &items[i];
        let (in_reply_to, refs) = match it.reply_to {
            Some(r) => {
                let parent = &items[r];
                let mut refs = vec![items[it.thread.unwrap_or(r)].msg_id.clone()];
                if parent.msg_id != refs[0] {
                    refs.push(parent.msg_id.clone());
                }
                (Some(parent.msg_id.clone()), refs)
            }
            None => (None, vec![]),
        };
        let raw = build_raw(&it.spec, it.date, &it.msg_id, in_reply_to.as_deref(), &refs);
        let thread = it.thread.and_then(|t| thread_ids.get(&t).cloned());
        let labels: Vec<String> = it.labels.iter().filter(|l| !l.is_empty()).cloned().collect();
        let id = store.add_message(raw, labels, thread, it.date);
        let tid = store.messages[&id].thread_id.clone();
        thread_ids.insert(i, tid.clone());
        if let Some(t) = it.thread {
            thread_ids.entry(t).or_insert(tid);
        }
        if it.draft {
            let did = format!("r{}", 5_000_000_000u64 + store.next_id % 1_000_000_007);
            store.drafts.insert(did.clone(), FakeDraft { id: did, message_id: id });
        }
    }
    // history from seeding isn't interesting to clients; start fresh
    store.history.clear();
    store.history_floor = store.history_id;
}

/// A new plausible Inbox message, dated `now_ms`.
pub fn deliver_random(store: &mut Store, now_ms: i64) -> String {
    let mut rng = Rng::for_key(&format!("deliver:{}:{}:{}", store.email, store.history_id, now_ms));
    let me = Who::new(&super::super::seed::display_name_for(&store.email), &store.email);
    let p = &PEOPLE[rng.below(PEOPLE.len() as u64) as usize];
    let p = if p.domain == "duvz.demo" { &PEOPLE[0] } else { p };
    let company = *rng.pick(COMPANIES);
    let options: Vec<(String, String)> = vec![
        (
            format!("{company} - quick question"),
            format!(
                "Hi Kenny,\n\nDo you have a minute today to discuss the {company} roadshow schedule? A couple of investors asked to move their slots."
            ),
        ),
        (
            format!("Re: {company} results call"),
            "Thanks for the invite - I'll dial in. Could you also send the slides beforehand?".to_string(),
        ),
        (
            "Meeting next week".to_string(),
            "Kenny, are you available for a coffee next week in Tel Aviv? I'd like to hear your view on the IPO market.".to_string(),
        ),
        (
            format!("{company}: updated model"),
            "Hi,\n\nI've updated our model after the call. Happy to share the key changes if helpful.".to_string(),
        ),
        ("עדכון קצר".to_string(), "קני, רק עדכון קצר - המשקיעים אישרו את הפגישה ליום שלישי.".to_string()),
    ];
    let (subject, body) = options[rng.below(options.len() as u64) as usize].clone();
    let from = Who::of(p);
    let spec = text_spec(from.clone(), vec![me], &subject, format!("{body}{}", signature_text(&from, p.title, p.company, p.phone)));
    let msg_id = format!("{:x}.live@{}", rng.next(), p.domain);
    let raw = build_raw(&spec, now_ms, &msg_id, None, &[]);
    let id = store.add_message(raw, vec!["INBOX".into(), "UNREAD".into(), "CATEGORY_PERSONAL".into()], None, now_ms);
    store.trim_history();
    id
}

/// Many simple messages spread over two years (performance tests). `thread_pct`: share of the
/// messages that are replies in an earlier bulk message's thread (Show as Conversations).
pub fn bulk(store: &mut Store, count: usize, now_ms: i64, thread_pct: u64) {
    let mut rng = Rng::for_key(&format!("bulk:{}:{}", store.email, store.messages.len()));
    let me = Who::new(&super::super::seed::display_name_for(&store.email), &store.email);
    let inbox = store.ensure_label("Bulk");
    let mut recent: Vec<(String, String)> = Vec::new();
    for i in 0..count {
        let p = &PEOPLE[rng.below(PEOPLE.len() as u64) as usize];
        let date = now_ms - (rng.below(730 * 24) as i64) * HOUR_MS - i as i64;
        let reply = if thread_pct > 0 && !recent.is_empty() && rng.chance(thread_pct) {
            Some(recent[rng.below(recent.len() as u64) as usize].clone())
        } else {
            None
        };
        let subject = match &reply {
            Some((_, s)) => format!("RE: {s}"),
            None => format!("{} update #{i}", p.company),
        };
        let spec = text_spec(
            Who::of(p),
            vec![me.clone()],
            &subject,
            format!(
                "Message {i} from {}.

Lorem ipsum dolor sit amet, investor relations update.
",
                p.name()
            ),
        );
        let raw = build_raw(&spec, date, &format!("bulk{i}.{}@{}", rng.next() % 100000, p.domain), None, &[]);
        let mut labels = vec![if i % 3 == 0 { inbox.clone() } else { "INBOX".to_string() }];
        if rng.chance(5) {
            labels.push("UNREAD".into());
        }
        let id = store.add_message(raw, labels, reply.as_ref().map(|r| r.0.clone()), date);
        if reply.is_none() && thread_pct > 0 {
            recent.push((store.messages[&id].thread_id.clone(), subject));
            if recent.len() > 200 {
                recent.remove(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_rich_and_deterministic() {
        let a = seed("kenny+seed@duvz.demo", "Kenny Green");
        assert!(a.messages.len() >= 400, "{}", a.messages.len());
        let has = |l: &str| a.messages.values().filter(|m| m.label_ids.iter().any(|x| x == l)).count();
        assert!(has("INBOX") > 100);
        assert!(has("SENT") > 20);
        assert!(has("TRASH") >= 4);
        assert!(has("SPAM") >= 3);
        assert_eq!(has("DRAFT"), 2);
        assert_eq!(a.drafts.len(), 2);
        assert!(has("UNREAD") > 10);
        assert!(has("STARRED") > 5);
        assert!(a.messages.values().any(|m| m.raw.contains("METHOD:REQUEST")));
        assert!(a.messages.values().any(|m| m.raw.contains("windows-1255")));
        assert!(a.messages.values().any(|m| m.raw.contains("iso-8859-8")));
        assert!(a.messages.values().any(|m| m.raw.contains("In-Reply-To")));
        assert!(a.messages.values().any(|m| m.raw.len() > 60_000), "a big newsletter");
        let today = now_ms() - DAY_MS / 2;
        assert!(a.messages.values().filter(|m| m.internal_date > today).count() >= 3);
        assert!(a.messages.values().all(|m| m.internal_date <= now_ms()));
        let b = seed("kenny+seed@duvz.demo", "Kenny Green");
        assert_eq!(a.messages.len(), b.messages.len());
        // xlsx / pdf / png look valid
        assert!(pdf("x", &["y"]).starts_with(b"%PDF-1.4"));
        assert!(logo_png().starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(xlsx(&[vec!["a".into()]]).starts_with(b"PK"));
    }

    #[test]
    fn deliver_adds_unread_inbox_message() {
        let mut s = Store::empty("kenny@duvz.demo", "Kenny Green");
        let id = deliver_random(&mut s, now_ms());
        let m = &s.messages[&id];
        assert!(m.label_ids.contains(&"INBOX".to_string()) && m.label_ids.contains(&"UNREAD".to_string()));
        assert_eq!(s.history.len(), 1);
    }
}
