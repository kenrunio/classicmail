//! Reading a mail-merge list: the first sheet of an Excel / OpenDocument workbook, or a CSV file.
//! The first non-empty row holds the column titles; the e-mail column is the one titled `Email`
//! or starting with `Email ` (any case).

use std::path::Path;

use calamine::{Data, Reader};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeList {
    pub file_name: String,
    pub headers: Vec<String>,
    /// index into `headers`
    pub email_column: usize,
    /// cell text, one Vec per data row (same length as `headers`)
    pub rows: Vec<Vec<String>>,
    /// 1-based row number in the file of each entry in `rows` (for messages to the user)
    pub row_numbers: Vec<usize>,
}

impl MergeList {
    pub fn email(&self, row: usize) -> &str {
        self.rows[row][self.email_column].as_str()
    }
}

pub fn load(path: &Path) -> anyhow::Result<MergeList> {
    let file_name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let grid = match ext.as_str() {
        "csv" | "txt" => {
            let bytes = std::fs::read(path).map_err(|e| open_error(&file_name, &e))?;
            read_csv(&bytes)?
        }
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => read_workbook(path, &file_name)?,
        _ => return Err(crate::user_err!("bad_list", "{file_name} is not an Excel or CSV file.")),
    };
    from_grid(file_name, grid)
}

fn open_error(file_name: &str, e: &std::io::Error) -> anyhow::Error {
    // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION: Excel has the file open
    if matches!(e.raw_os_error(), Some(32 | 33)) {
        crate::user_err!("bad_list", "{file_name} is open in another program. Close it in Excel and try again.")
    } else {
        crate::user_err!("bad_list", "Can't open {file_name}: {e}")
    }
}

fn read_workbook(path: &Path, file_name: &str) -> anyhow::Result<Vec<(usize, Vec<String>)>> {
    let mut wb = calamine::open_workbook_auto(path).map_err(|e| match e {
        calamine::Error::Io(io) => open_error(file_name, &io),
        other => crate::user_err!("bad_list", "Can't read {file_name}: {other}"),
    })?;
    let range = wb
        .worksheet_range_at(0)
        .ok_or_else(|| crate::user_err!("bad_list", "{file_name} has no worksheets."))?
        .map_err(|e| crate::user_err!("bad_list", "Can't read {file_name}: {e}"))?;
    let first_row = range.start().map(|(r, _)| r as usize).unwrap_or(0);
    Ok(range.rows().enumerate().map(|(i, r)| (first_row + i + 1, r.iter().map(cell_text).collect())).collect())
}

/// A cell as Excel shows it (roughly): whole numbers without ".0", dates as dd/mm/yyyy.
pub fn cell_text(d: &Data) -> String {
    match d {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => float_text(*f),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(t) if dt.is_datetime() => {
                if t.time() == chrono::NaiveTime::MIN {
                    t.format("%d/%m/%Y").to_string()
                } else {
                    t.format("%d/%m/%Y %H:%M").to_string()
                }
            }
            _ => float_text(dt.as_f64()),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
    }
}

fn float_text(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { f.to_string() }
}

/// Decode (UTF-8, BOM stripped; else Windows-1255), detect the delimiter, split into rows.
pub fn read_csv(bytes: &[u8]) -> anyhow::Result<Vec<(usize, Vec<String>)>> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1255.decode(bytes).0.into_owned(),
    };
    let header_line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let delimiter = (*b",;\t").into_iter().max_by_key(|d| header_line.bytes().filter(|b| b == d).count()).unwrap_or(b',');
    let mut rdr = csv::ReaderBuilder::new().has_headers(false).flexible(true).delimiter(delimiter).from_reader(text.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| crate::user_err!("bad_list", "The CSV file can't be read: {e}"))?;
        let line = rec.position().map(|p| p.line() as usize).unwrap_or(out.len() + 1);
        out.push((line, rec.iter().map(|s| s.trim().to_string()).collect()));
    }
    Ok(out)
}

/// Is this column title the e-mail column? `Email`, `E-mail`, `Email Address`, `email 1`…
pub fn is_email_header(h: &str) -> bool {
    let h = h.trim().to_lowercase();
    let h = h.strip_prefix("e-mail").map(|r| format!("email{r}")).unwrap_or(h);
    h == "email" || h.starts_with("email ")
}

pub fn from_grid(file_name: String, grid: Vec<(usize, Vec<String>)>) -> anyhow::Result<MergeList> {
    let mut it = grid.into_iter().filter(|(_, r)| r.iter().any(|c| !c.is_empty()));
    let (_, headers) = it.next().ok_or_else(|| crate::user_err!("bad_list", "{file_name} is empty."))?;
    let headers: Vec<String> = headers.into_iter().map(|h| h.trim().to_string()).collect();
    let email_column = headers.iter().position(|h| is_email_header(h)).ok_or_else(|| {
        crate::user_err!("bad_list", "The list needs a column titled Email (or starting with \"Email \", e.g. Email Address).")
    })?;
    let width = headers.len();
    let mut rows = Vec::new();
    let mut row_numbers = Vec::new();
    for (n, mut r) in it {
        r.resize(width, String::new());
        rows.push(r);
        row_numbers.push(n);
    }
    Ok(MergeList { file_name, headers, email_column, rows, row_numbers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[&str]]) -> Vec<(usize, Vec<String>)> {
        rows.iter().enumerate().map(|(i, r)| (i + 1, r.iter().map(|s| s.to_string()).collect())).collect()
    }

    #[test]
    fn email_header_detection() {
        for h in ["Email", "email", " EMAIL ", "Email Address", "email 1", "E-mail", "E-mail address"] {
            assert!(is_email_header(h), "{h}");
        }
        for h in ["Emails", "Emailaddress", "Work email", "", "mail"] {
            assert!(!is_email_header(h), "{h}");
        }
    }

    #[test]
    fn grid_skips_blank_rows_and_pads() {
        let l = from_grid(
            "x.csv".into(),
            grid(&[
                &["", ""],
                &["First Name", "Email Address", "Company"],
                &["Dan", "dan@x.com"],
                &["", "", ""],
                &["Ruth", "ruth@y.co.il", "Y"],
            ]),
        )
        .unwrap();
        assert_eq!(l.headers, vec!["First Name", "Email Address", "Company"]);
        assert_eq!(l.email_column, 1);
        assert_eq!(l.rows.len(), 2);
        assert_eq!(l.rows[0], vec!["Dan", "dan@x.com", ""]);
        assert_eq!(l.row_numbers, vec![3, 5]);
        assert_eq!(l.email(1), "ruth@y.co.il");
    }

    #[test]
    fn grid_without_email_column_is_an_error() {
        let e = from_grid("x.csv".into(), grid(&[&["Name", "Mail"], &["a", "b"]])).unwrap_err();
        assert!(e.to_string().contains("Email"), "{e}");
    }

    #[test]
    fn csv_utf8_bom_semicolons() {
        let bytes = "\u{feff}שם;Email\nדני;dan@x.com\n\"Cohen; Ruth\";ruth@y.com\n".as_bytes();
        let g = read_csv(bytes).unwrap();
        let l = from_grid("x.csv".into(), g).unwrap();
        assert_eq!(l.headers, vec!["שם", "Email"]);
        assert_eq!(l.rows[0], vec!["דני", "dan@x.com"]);
        assert_eq!(l.rows[1][0], "Cohen; Ruth");
    }

    #[test]
    fn csv_windows_1255() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1255.encode("Email,שם פרטי\ndan@x.com,דני\n");
        let l = from_grid("x.csv".into(), read_csv(&bytes).unwrap()).unwrap();
        assert_eq!(l.headers[1], "שם פרטי");
        assert_eq!(l.rows[0][1], "דני");
    }

    #[test]
    fn csv_tabs() {
        let l = from_grid("x.txt".into(), read_csv(b"Email\tFirst\na@b.com\tAl\n").unwrap()).unwrap();
        assert_eq!(l.rows[0], vec!["a@b.com", "Al"]);
    }

    #[test]
    fn xlsx_values_as_excel_shows_them() {
        use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("list.xlsx");
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        let date = Format::new().set_num_format("dd/mm/yyyy");
        ws.write(0, 0, "First Name").unwrap();
        ws.write(0, 1, "Email").unwrap();
        ws.write(0, 2, "Shares").unwrap();
        ws.write(0, 3, "Meeting").unwrap();
        ws.write(0, 4, "Price").unwrap();
        ws.write(1, 0, "משה").unwrap();
        ws.write(1, 1, " moshe@acme.example ").unwrap();
        ws.write(1, 2, 1500.0).unwrap();
        ws.write_datetime_with_format(1, 3, ExcelDateTime::from_ymd(2026, 10, 14).unwrap(), &date).unwrap();
        ws.write(1, 4, 12.5).unwrap();
        wb.save(&path).unwrap();
        let l = load(&path).unwrap();
        assert_eq!(l.file_name, "list.xlsx");
        assert_eq!(l.email_column, 1);
        assert_eq!(l.rows[0], vec!["משה", "moshe@acme.example", "1500", "14/10/2026", "12.5"]);
        assert_eq!(l.row_numbers, vec![2]);
    }

    #[test]
    fn unsupported_extension() {
        let e = load(Path::new("C:/nope/list.pdf")).unwrap_err();
        assert!(e.to_string().contains("not an Excel or CSV"), "{e}");
    }
}
