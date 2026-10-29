//! Attachment preview in the Reading Pane (Outlook's attachment previewers): click an attachment
//! and the body shows it instead of the message.
//!
//! `mail.previewAttachment { id }` → `{ kind: "html", html }` — pictures, text, CSV / Excel sheets,
//! Word documents, HTML files and attached e-mails, as HTML for the sandboxed body frame (built here
//! from escaped text, or sanitised like a message) — or `{ kind: "pdf", url }` for the WebView's own
//! PDF viewer (`/pdf/<id>` on the attachment protocol), or `{ kind: "none", note }`.

use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Arc,
};

use calamine::Reader;
use quick_xml::{Reader as XmlReader, events::Event};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    merge::list::{cell_text, read_csv},
    mime::escape_html,
    render,
};
use crate::ctx::AppCtx;

/// Bigger files aren't previewed (open them instead).
const MAX_BYTES: usize = 30 * 1024 * 1024;
/// Rows / columns of a sheet shown (the rest is noted).
const MAX_ROWS: usize = 2000;
const MAX_COLS: usize = 60;

#[derive(Deserialize)]
pub(super) struct PreviewParams {
    id: i64,
}

#[derive(Debug, PartialEq)]
enum Kind {
    Image,
    Pdf,
    Text,
    Csv,
    /// a workbook, by its extension (xlsx / xlsm / xlsb / xls / ods)
    Sheet(String),
    Word,
    Html,
    Email,
    None,
}

fn kind_of(name: &str, mime: &str) -> Kind {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mime = mime.to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Kind::Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => Kind::Image,
        "csv" | "tsv" => Kind::Csv,
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => Kind::Sheet(ext),
        "docx" => Kind::Word,
        "htm" | "html" => Kind::Html,
        "eml" => Kind::Email,
        "txt" | "log" | "md" | "json" | "xml" | "ics" | "vcf" | "ini" | "cfg" | "yaml" | "yml" => Kind::Text,
        _ => match mime.as_str() {
            "application/pdf" => Kind::Pdf,
            "message/rfc822" => Kind::Email,
            "text/html" => Kind::Html,
            "text/csv" => Kind::Csv,
            m if m.starts_with("image/") && m != "image/svg+xml" => Kind::Image,
            m if m.starts_with("text/") => Kind::Text,
            _ => Kind::None,
        },
    }
}

pub(super) async fn preview_attachment(ctx: Arc<AppCtx>, p: PreviewParams) -> anyhow::Result<Value> {
    let (name, mime, size): (String, String, i64) = ctx
        .db
        .read(move |c| {
            Ok(c.query_row("SELECT filename, mime_type, size FROM attachments WHERE id=?1", [p.id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?)
        })
        .await?;
    let kind = kind_of(&name, &mime);
    if kind == Kind::None {
        return Ok(json!({ "kind": "none", "note": "There is no preview for this kind of file." }));
    }
    if size as usize > MAX_BYTES {
        return Ok(json!({ "kind": "none", "note": "This file is too large to preview." }));
    }
    let base = render::attach_base();
    match kind {
        Kind::Image => {
            return Ok(
                json!({ "kind": "html", "html": format!("<div style=\"text-align:center\"><img src=\"{base}/att/{}\" style=\"max-width:100%\"></div>", p.id) }),
            );
        }
        Kind::Pdf => return Ok(json!({ "kind": "pdf", "url": format!("{base}/pdf/{}", p.id) })),
        _ => {}
    }
    let (_, _, bytes) = super::attachment_bytes(&ctx, p.id).await?;
    let html = tokio::task::spawn_blocking(move || to_html(kind, &bytes)).await?;
    Ok(match html {
        Ok(h) => json!({ "kind": "html", "html": h }),
        Err(e) => json!({ "kind": "none", "note": format!("This file couldn't be previewed: {e}") }),
    })
}

fn to_html(kind: Kind, bytes: &[u8]) -> anyhow::Result<String> {
    Ok(match kind {
        Kind::Text => format!("<pre style=\"white-space:pre-wrap\" dir=\"auto\">{}</pre>", escape_html(&decode_text(bytes))),
        Kind::Csv => {
            let rows: Vec<Vec<String>> = read_csv(bytes)?.into_iter().map(|(_, r)| r).collect();
            table_html(&rows)
        }
        Kind::Sheet(ext) => sheets_html(bytes, &ext)?,
        Kind::Word => docx_html(bytes)?,
        Kind::Html => render::render_html(&decode_text(bytes), &HashMap::<String, i64>::new(), false).html,
        Kind::Email => email_html(bytes),
        Kind::Image | Kind::Pdf | Kind::None => String::new(),
    })
}

/// UTF-8 (BOM stripped), else Windows-1255 (Hebrew; Latin text reads the same).
fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1255.decode(bytes).0.into_owned(),
    }
}

const TABLE_STYLE: &str = "border-collapse:collapse;font-size:12px;margin:0 0 14px";
const CELL_STYLE: &str = "border:1px solid #c9d3e0;padding:2px 6px;vertical-align:top";

/// "1,234.5", "(12)", "-3%", "$4.2": a number as a spreadsheet shows it (right-aligned).
fn is_number(v: &str) -> bool {
    let t: String = v.trim().chars().filter(|c| !matches!(c, ',' | '%' | '$' | '(' | ')' | ' ')).collect();
    !t.is_empty() && t.parse::<f64>().is_ok()
}

/// A grid as a table: the first row as headings, columns beyond the last non-empty one dropped.
/// Cells stay on one line, as in the spreadsheet (a wide sheet scrolls sideways).
fn table_html(rows: &[Vec<String>]) -> String {
    let width = rows.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1)).max().unwrap_or(0).min(MAX_COLS);
    let mut out = format!("<table style=\"{TABLE_STYLE};white-space:nowrap\">");
    for (i, r) in rows.iter().take(MAX_ROWS).enumerate() {
        out.push_str("<tr>");
        for c in 0..width {
            let v = r.get(c).map(String::as_str).unwrap_or("");
            let tag = if i == 0 { "th" } else { "td" };
            let bg = if i == 0 {
                ";background:#eef3fb;text-align:left"
            } else if is_number(v) {
                ";text-align:right"
            } else {
                ""
            };
            out.push_str(&format!("<{tag} style=\"{CELL_STYLE}{bg}\" dir=\"auto\">{}</{tag}>", escape_html(v)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    if rows.len() > MAX_ROWS {
        out.push_str(&format!(
            "<p style=\"color:#666\">Showing the first {MAX_ROWS} of {} rows. Open the file to see the rest.</p>",
            rows.len()
        ));
    }
    out
}

/// Every sheet of a workbook, each under its name. Opened by its extension, like a file on disk
/// (guessing the format from the content fails for some workbooks).
fn sheets_html(bytes: &[u8], ext: &str) -> anyhow::Result<String> {
    use calamine::{Ods, Sheets, Xls, Xlsb, Xlsx, open_workbook_from_rs};
    let rs = Cursor::new(bytes.to_vec());
    let mut wb = match ext {
        "xls" => Sheets::Xls(open_workbook_from_rs::<Xls<_>, _>(rs)?),
        "xlsb" => Sheets::Xlsb(open_workbook_from_rs::<Xlsb<_>, _>(rs)?),
        "ods" => Sheets::Ods(open_workbook_from_rs::<Ods<_>, _>(rs)?),
        _ => Sheets::Xlsx(open_workbook_from_rs::<Xlsx<_>, _>(rs)?),
    };
    let names = wb.sheet_names().to_vec();
    let mut out = String::new();
    for name in &names {
        let Ok(range) = wb.worksheet_range(name) else { continue };
        let rows: Vec<Vec<String>> = range.rows().map(|r| r.iter().map(cell_text).collect()).collect();
        if names.len() > 1 {
            out.push_str(&format!("<h3 style=\"font-size:14px;margin:6px 0\" dir=\"auto\">{}</h3>", escape_html(name)));
        }
        if rows.iter().all(|r| r.iter().all(String::is_empty)) {
            out.push_str("<p style=\"color:#666\">(empty)</p>");
        } else {
            out.push_str(&table_html(&rows));
        }
    }
    Ok(out)
}

/// A Word document's text: paragraphs (headings), bold / italic / underline, line breaks and
/// tables. Pictures, text boxes and page layout are left out (open the file for those).
fn docx_html(bytes: &[u8]) -> anyhow::Result<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")?.read_to_string(&mut xml)?;
    let mut r = XmlReader::from_str(&xml);
    let mut out = String::new();
    // the paragraph being built: its tag (p / h1..h3), whether it's right-to-left, its runs
    let (mut para, mut tag, mut in_para) = (String::new(), "p", false);
    let (mut bold, mut italic, mut underline, mut in_text, mut pictures) = (false, false, false, false, 0);
    loop {
        match r.read_event()? {
            ev @ (Event::Start(_) | Event::Empty(_)) => {
                let start = matches!(ev, Event::Start(_));
                let (Event::Start(e) | Event::Empty(e)) = ev else { unreachable!() };
                let attr = |key: &[u8]| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == key)
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                };
                // w:b / w:i / w:u are on unless w:val says off ("0" / "false" / "none")
                let on = || !matches!(attr(b"val").as_deref(), Some("0" | "false" | "none"));
                match e.local_name().as_ref() {
                    b"p" => {
                        (para, tag, in_para) = (String::new(), "p", true);
                    }
                    b"pStyle" => {
                        tag = match attr(b"val").unwrap_or_default().to_ascii_lowercase().as_str() {
                            "heading1" | "title" => "h1",
                            "heading2" => "h2",
                            "heading3" => "h3",
                            _ => tag,
                        };
                    }
                    b"r" => (bold, italic, underline) = (false, false, false),
                    b"b" => bold = on(),
                    b"i" => italic = on(),
                    b"u" => underline = on(),
                    b"t" => in_text = start,
                    b"tab" if in_para => para.push_str("&emsp;"),
                    b"br" | b"cr" if in_para => para.push_str("<br>"),
                    b"drawing" | b"pict" => pictures += 1,
                    b"tbl" => out.push_str(&format!("<table style=\"{TABLE_STYLE}\">")),
                    b"tr" => out.push_str("<tr>"),
                    b"tc" => out.push_str(&format!("<td style=\"{CELL_STYLE}\" dir=\"auto\">")),
                    _ => {}
                }
            }
            Event::Text(t) if in_text => {
                let s = escape_html(&t.decode()?);
                let mut s = s;
                if underline {
                    s = format!("<u>{s}</u>");
                }
                if italic {
                    s = format!("<i>{s}</i>");
                }
                if bold {
                    s = format!("<b>{s}</b>");
                }
                para.push_str(&s);
            }
            Event::GeneralRef(g) if in_text => {
                // &amp; &lt; ... inside w:t
                if let Ok(Some(c)) = g.resolve_char_ref() {
                    para.push_str(&escape_html(&c.to_string()));
                } else {
                    let name = String::from_utf8_lossy(&g);
                    para.push_str(&escape_html(&super::mime::unescape_entities(&format!("&{name};"))));
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => {
                    in_para = false;
                    let body = if para.is_empty() { "&nbsp;".to_string() } else { std::mem::take(&mut para) };
                    out.push_str(&format!("<{tag} dir=\"auto\" style=\"margin:0 0 6px\">{body}</{tag}>"));
                }
                b"tc" => out.push_str("</td>"),
                b"tr" => out.push_str("</tr>"),
                b"tbl" => out.push_str("</table>"),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if pictures > 0 {
        out.push_str(&format!(
            "<p style=\"color:#666\">This document has {pictures} picture{} that the preview doesn't show. Open the file to see them.</p>",
            if pictures == 1 { "" } else { "s" }
        ));
    }
    Ok(out)
}

/// An attached e-mail: its From / Sent / To / Subject and body (sanitised like a message).
fn email_html(bytes: &[u8]) -> String {
    let Some(m) = mail_parser::MessageParser::default().parse(bytes) else {
        return "<p>This e-mail couldn't be read.</p>".into();
    };
    let addrs = |a: Option<&mail_parser::Address>| {
        a.map(|a| a.iter().map(|x| x.name().or(x.address()).unwrap_or("").to_string()).collect::<Vec<_>>().join("; ")).unwrap_or_default()
    };
    let date = m.date().map(|d| d.to_rfc822()).unwrap_or_default();
    let mut head = String::from("<table style=\"font-size:13px;margin:0 0 10px;border-bottom:1px solid #c9d3e0\">");
    for (k, v) in [
        ("From:", addrs(m.from())),
        ("Sent:", date),
        ("To:", addrs(m.to())),
        ("Cc:", addrs(m.cc())),
        ("Subject:", m.subject().unwrap_or("").to_string()),
    ] {
        if !v.is_empty() {
            head.push_str(&format!(
                "<tr><td style=\"font-weight:bold;padding-right:8px;vertical-align:top\">{k}</td><td dir=\"auto\">{}</td></tr>",
                escape_html(&v)
            ));
        }
    }
    head.push_str("</table>");
    let body = match m.body_html(0) {
        Some(h) => render::render_html(&h, &HashMap::<String, i64>::new(), false).html,
        None => super::mime::text_to_html(&m.body_text(0).unwrap_or_default()),
    };
    head + &body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name_then_type() {
        assert_eq!(kind_of("Q3 results.PDF", "application/octet-stream"), Kind::Pdf);
        assert_eq!(kind_of("list.XLSX", ""), Kind::Sheet("xlsx".into()));
        assert_eq!(kind_of("memo.docx", ""), Kind::Word);
        assert_eq!(kind_of("scan", "image/jpeg"), Kind::Image);
        assert_eq!(kind_of("logo.svg", "image/svg+xml"), Kind::None);
        assert_eq!(kind_of("deck.pptx", ""), Kind::None);
        assert_eq!(kind_of("fwd", "message/rfc822"), Kind::Email);
    }

    #[test]
    fn text_and_csv_are_escaped() {
        let h = to_html(Kind::Text, b"<script>alert(1)</script> \xd7\xa9\xd7\x9c\xd7\x95\xd7\x9d").unwrap();
        assert!(h.contains("&lt;script&gt;") && !h.contains("<script>") && h.contains("שלום"), "{h}");
        let h = to_html(Kind::Csv, b"Name,Email\nKenny <b>,k@x.com\n").unwrap();
        assert!(h.contains("<th") && h.contains("Kenny &lt;b&gt;"), "{h}");
    }

    #[test]
    fn html_attachments_are_sanitised() {
        let h = to_html(Kind::Html, b"<p onclick=\"x()\">Hi</p><script>bad()</script><img src=\"https://t.example/p.gif\">").unwrap();
        assert!(h.contains("Hi") && !h.contains("onclick") && !h.contains("bad()") && !h.contains("t.example"), "{h}");
    }

    /// A minimal .docx written with the zip crate.
    fn docx(body: &str) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        let xml = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
        );
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn word_documents_show_text_headings_and_tables() {
        let d = docx(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Roadshow &amp; plan</w:t></w:r></w:p>
               <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r><w:r><w:t xml:space="preserve"> and plain &lt;x&gt;</w:t></w:r></w:p>
               <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        );
        let h = to_html(Kind::Word, &d).unwrap();
        assert!(h.contains("<h1") && h.contains("Roadshow &amp; plan"), "{h}");
        assert!(h.contains("<b>Bold</b> and plain &lt;x&gt;"), "{h}");
        assert!(h.contains("<table") && h.contains("Cell"), "{h}");
    }

    #[test]
    fn workbooks_show_every_sheet() {
        let rows = vec![vec!["Holder".to_string(), "Shares".to_string()], vec!["Fund <A>".to_string(), "1200".to_string()]];
        let h = to_html(Kind::Sheet("xlsx".into()), &crate::fake_google::gmail::seed_data::xlsx(&rows)).unwrap();
        assert!(h.contains("<th") && h.contains("Holder") && h.contains("Fund &lt;A&gt;") && h.contains("1200"), "{h}");
        // one line per cell, as in the spreadsheet; numbers right-aligned
        assert!(h.contains("white-space:nowrap"), "{h}");
        assert!(h.contains("text-align:right\" dir=\"auto\">1200<"), "{h}");
        if let Ok(dir) = std::env::var("CM_PREVIEW_OUT") {
            let mut wide = vec![
                (0..18)
                    .map(|i| if i == 0 { "Orion Consolidated P&L".to_string() } else { format!("Q{} 2025", i % 4 + 1) })
                    .collect::<Vec<_>>(),
            ];
            for r in 0..12 {
                wide.push(
                    (0..18)
                        .map(|i| if i == 0 { format!("Revenue line {r}") } else { format!("{},{:03}.{}", 97 + r, i * 37 % 1000, i % 10) })
                        .collect(),
                );
            }
            let h = to_html(Kind::Sheet("xlsx".into()), &crate::fake_google::gmail::seed_data::xlsx(&wide)).unwrap();
            std::fs::write(format!("{dir}/sheet.html"), h).unwrap();
        }
    }

    #[test]
    fn numbers_as_spreadsheets_show_them() {
        for n in ["1200", "1,234.5", "(12)", "-3%", "$4.2", " 7 "] {
            assert!(is_number(n), "{n}");
        }
        for t in ["", "Q1", "2024 YTD", "n/a", "-"] {
            assert!(!is_number(t), "{t}");
        }
    }

    #[test]
    fn attached_emails_show_header_and_body() {
        let eml = b"From: Sarah Cohen <sarah@x.com>\r\nTo: kenny@duvz.demo\r\nSubject: Q3 call\r\nContent-Type: text/plain\r\n\r\nSee you <then>.\r\n";
        let h = to_html(Kind::Email, eml).unwrap();
        assert!(h.contains("Sarah Cohen") && h.contains("Q3 call") && h.contains("See you &lt;then&gt;."), "{h}");
    }
}
