//! RTF bodies of archived messages (after LZFu decompression):
//! * HTML encapsulated in RTF (`\fromhtml1`, [MS-OXRTFEX]) → the original HTML;
//! * RTF made from plain text (`\fromtext`) → the text;
//! * other RTF → light HTML: paragraphs / line breaks, bold, italic, underline, `\uN` Unicode and
//!   `\'xx` bytes decoded with the font's charset or the document's `\ansicpg` (Hebrew
//!   windows-1255 matters).
//!
//! Tables, pictures, fonts and colours are not rendered; their text still shows.

use super::codepage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rtf {
    /// de-encapsulated original HTML (`\fromhtml1`)
    Html(String),
    /// plain text (`\fromtext`)
    Text(String),
    /// our light conversion of real RTF (an HTML fragment)
    Rich(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Dest {
    #[default]
    Normal,
    /// skipped destination (`{\*\foo ...}`, `\info`, `\pict`, ...)
    Skip,
    FontTable,
}

#[derive(Clone, Debug)]
struct State {
    dest: Dest,
    /// inside `{\*\htmltag ...}`
    htmltag: bool,
    /// `\htmlrtf` (RTF-only content of an encapsulated HTML document)
    htmlrtf: bool,
    font: i32,
    uc: usize,
    bold: bool,
    italic: bool,
    underline: bool,
    /// `{\*` seen: the next control word is an ignorable destination
    star: bool,
    /// a control word was already seen in this group (destinations must come first)
    first: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            dest: Dest::Normal,
            htmltag: false,
            htmlrtf: false,
            font: -1,
            uc: 1,
            bold: false,
            italic: false,
            underline: false,
            star: false,
            first: true,
        }
    }
}

/// Destinations whose text is never shown.
const SKIP_DESTS: &[&str] = &[
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "objdata",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "fldinst",
    "xe",
    "tc",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "mmathPr",
    "pgdsctbl",
    "revtbl",
    "filetbl",
    "nonshppict",
    "shpinst",
    "userprops",
    "xmlnstbl",
    "wgrffmtfilter",
    "listtext",
    "pntext",
    "pntxta",
    "pntxtb",
    "author",
    "operator",
    "title",
    "subject",
    "keywords",
    "comment",
    "doccomm",
    "company",
    "template",
    "fchars",
    "lchars",
    "bkmkstart",
    "bkmkend",
    "atnid",
    "atnauthor",
    "annotation",
    "background",
    "shppict",
    "blipuid",
    "falt",
    "panose",
    "fname",
    "passwordhash",
    "ftnsep",
    "ftnsepc",
    "aftnsep",
    "aftnsepc",
    "mhtmltag",
    "htmlbase",
];

struct Conv {
    from_html: bool,
    from_text: bool,
    doc_cp: u32,
    fonts: std::collections::HashMap<i32, u32>,
    deff: i32,
    font_def: i32,
    out: String,
    /// bytes waiting to be decoded, and the code page they are in
    bytes: Vec<u8>,
    bytes_cp: u32,
    /// fallback characters still to skip after `\uN`
    uc_skip: usize,
    /// open `<span>` style in Rich mode (bold, italic, underline)
    span: Option<(bool, bool, bool)>,
}

impl Conv {
    fn cp_of(&self, st: &State) -> u32 {
        let f = if st.font >= 0 { st.font } else { self.deff };
        match self.fonts.get(&f) {
            Some(&cp) if cp != 0 => cp,
            _ => self.doc_cp,
        }
    }

    fn flush_bytes(&mut self, st: &State) {
        if self.bytes.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.bytes);
        let s = codepage::decode(&bytes, self.bytes_cp);
        self.emit_str(&s, st);
    }

    fn push_byte(&mut self, b: u8, st: &State) {
        if self.uc_skip > 0 {
            self.uc_skip -= 1;
            return;
        }
        if !self.visible(st) {
            return;
        }
        let cp = self.cp_of(st);
        if !self.bytes.is_empty() && cp != self.bytes_cp {
            self.flush_bytes(st);
        }
        self.bytes_cp = cp;
        self.bytes.push(b);
    }

    fn visible(&self, st: &State) -> bool {
        match st.dest {
            Dest::Skip | Dest::FontTable => false,
            Dest::Normal => {
                if self.from_html {
                    st.htmltag || !st.htmlrtf
                } else {
                    true
                }
            }
        }
    }

    /// Text in the current state (already decoded).
    fn emit_str(&mut self, s: &str, st: &State) {
        if s.is_empty() || !self.visible(st) {
            return;
        }
        if self.from_html || self.from_text {
            self.out.push_str(s);
            return;
        }
        let want = (st.bold, st.italic, st.underline);
        let want = if want == (false, false, false) { None } else { Some(want) };
        if self.span != want {
            if self.span.is_some() {
                self.out.push_str("</span>");
            }
            if let Some((b, i, u)) = want {
                let mut css = String::new();
                if b {
                    css.push_str("font-weight:bold;");
                }
                if i {
                    css.push_str("font-style:italic;");
                }
                if u {
                    css.push_str("text-decoration:underline;");
                }
                self.out.push_str(&format!("<span style=\"{css}\">"));
            }
            self.span = want;
        }
        for ch in s.chars() {
            match ch {
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '&' => self.out.push_str("&amp;"),
                '"' => self.out.push_str("&quot;"),
                '\u{a0}' => self.out.push_str("&nbsp;"),
                c => self.out.push(c),
            }
        }
    }

    /// Structural output (line breaks, tabs) in the current mode.
    fn emit_break(&mut self, st: &State, kind: Break) {
        self.flush_bytes(st);
        if !self.visible(st) {
            return;
        }
        if self.from_html || self.from_text {
            self.out.push_str(match kind {
                Break::Line => "\r\n",
                Break::Tab => "\t",
            });
        } else {
            match kind {
                Break::Line => {
                    if self.span.take().is_some() {
                        self.out.push_str("</span>");
                    }
                    self.out.push_str("<br>\n");
                }
                Break::Tab => self.out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Break {
    Line,
    Tab,
}

pub fn convert(rtf: &[u8]) -> Rtf {
    let head = &rtf[..rtf.len().min(4096)];
    let mut c = Conv {
        from_html: head.windows(9).any(|w| w == b"\\fromhtml"),
        from_text: head.windows(9).any(|w| w == b"\\fromtext"),
        doc_cp: 1252,
        fonts: Default::default(),
        deff: 0,
        font_def: -1,
        out: String::with_capacity(rtf.len()),
        bytes: Vec::new(),
        bytes_cp: 1252,
        uc_skip: 0,
        span: None,
    };
    let mut stack: Vec<State> = vec![State::default()];
    let n = rtf.len();
    let mut i = 0;
    let mut depth_guard = 0usize;
    while i < n {
        let b = rtf[i];
        match b {
            b'{' => {
                let st = stack.last().cloned().unwrap_or_default();
                c.flush_bytes(&st);
                if stack.len() < 2000 {
                    stack.push(State { star: false, first: true, ..st });
                } else {
                    depth_guard += 1;
                }
                c.uc_skip = 0;
                i += 1;
            }
            b'}' => {
                let st = stack.last().cloned().unwrap_or_default();
                c.flush_bytes(&st);
                if depth_guard > 0 {
                    depth_guard -= 1;
                } else if stack.len() > 1 {
                    stack.pop();
                }
                c.uc_skip = 0;
                i += 1;
            }
            b'\\' => {
                let Some(&next) = rtf.get(i + 1) else { break };
                if next.is_ascii_alphabetic() {
                    let start = i + 1;
                    let mut j = start;
                    while j < n && j - start < 32 && rtf[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let word = std::str::from_utf8(&rtf[start..j]).unwrap_or("");
                    let mut param: Option<i32> = None;
                    let pstart = j;
                    if j < n && (rtf[j] == b'-' || rtf[j].is_ascii_digit()) {
                        j += 1;
                        while j < n && j - pstart < 11 && rtf[j].is_ascii_digit() {
                            j += 1;
                        }
                        param = std::str::from_utf8(&rtf[pstart..j])
                            .ok()
                            .and_then(|s| s.parse::<i64>().ok())
                            .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
                    }
                    if j < n && rtf[j] == b' ' {
                        j += 1;
                    }
                    i = j;
                    if word == "bin" {
                        // binary data: skip it unseen
                        i = i.saturating_add(param.unwrap_or(0).max(0) as usize).min(n);
                        continue;
                    }
                    control_word(&mut c, &mut stack, word, param);
                } else {
                    i += 2;
                    let st = stack.last().cloned().unwrap_or_default();
                    match next {
                        b'\'' => {
                            let hex =
                                rtf.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                            if let Some(v) = hex {
                                c.push_byte(v, &st);
                                i += 2;
                            }
                        }
                        b'*' => {
                            if let Some(s) = stack.last_mut() {
                                s.star = true;
                            }
                        }
                        b'\\' | b'{' | b'}' => c.push_byte(next, &st),
                        b'~' => {
                            c.flush_bytes(&st);
                            if c.uc_skip > 0 {
                                c.uc_skip -= 1;
                            } else {
                                c.emit_str("\u{a0}", &st);
                            }
                        }
                        b'_' => c.push_byte(b'-', &st),
                        b'\r' | b'\n' => c.emit_break(&st, Break::Line),
                        b'\t' => c.emit_break(&st, Break::Tab),
                        _ => {} // \- optional hyphen, \: index sub-entry, \| formula...
                    }
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                let st = stack.last().cloned().unwrap_or_default();
                if let Some(s) = stack.last_mut() {
                    s.first = false;
                }
                c.push_byte(b, &st);
                i += 1;
            }
        }
    }
    let st = stack.last().cloned().unwrap_or_default();
    c.flush_bytes(&st);
    if c.span.take().is_some() {
        c.out.push_str("</span>");
    }
    if c.from_html {
        Rtf::Html(c.out)
    } else if c.from_text {
        Rtf::Text(c.out)
    } else {
        Rtf::Rich(c.out)
    }
}

fn control_word(c: &mut Conv, stack: &mut [State], word: &str, param: Option<i32>) {
    let Some(st) = stack.last_mut() else { return };
    let star = std::mem::replace(&mut st.star, false);
    let first = std::mem::replace(&mut st.first, false);
    let snapshot = st.clone();
    // destinations
    if star && first {
        if word == "htmltag" {
            c.flush_bytes(&snapshot);
            st.htmltag = true;
            st.dest = Dest::Normal;
        } else {
            c.flush_bytes(&snapshot);
            st.dest = Dest::Skip;
        }
        return;
    }
    if first && SKIP_DESTS.contains(&word) {
        c.flush_bytes(&snapshot);
        st.dest = Dest::Skip;
        return;
    }
    if word == "fonttbl" {
        c.flush_bytes(&snapshot);
        st.dest = Dest::FontTable;
        return;
    }
    if st.dest == Dest::FontTable {
        match word {
            "f" => c.font_def = param.unwrap_or(0),
            "fcharset" => {
                if let Some(cp) = codepage::for_charset(param.unwrap_or(0)) {
                    c.fonts.insert(c.font_def, cp);
                }
            }
            "cpg" => {
                if let Some(p) = param.filter(|p| *p > 0) {
                    c.fonts.insert(c.font_def, p as u32);
                }
            }
            _ => {}
        }
        return;
    }
    if st.dest == Dest::Skip {
        return;
    }
    let on = param.map(|p| p != 0).unwrap_or(true);
    match word {
        "ansicpg" => {
            if let Some(p) = param.filter(|p| *p > 0) {
                c.doc_cp = p as u32;
            }
        }
        "deff" => c.deff = param.unwrap_or(0),
        "f" => {
            c.flush_bytes(&snapshot);
            st.font = param.unwrap_or(0);
        }
        "uc" => st.uc = param.unwrap_or(1).clamp(0, 10) as usize,
        "u" => {
            c.flush_bytes(&snapshot);
            let mut v = param.unwrap_or(0);
            if v < 0 {
                v += 65536;
            }
            let s = char::from_u32(v as u32).map(|ch| ch.to_string()).unwrap_or_default();
            c.uc_skip = 0;
            c.emit_str(&s, &snapshot);
            c.uc_skip = snapshot.uc;
        }
        "htmlrtf" => {
            c.flush_bytes(&snapshot);
            st.htmlrtf = on;
        }
        "par" | "line" | "sect" | "page" | "row" => c.emit_break(&snapshot, Break::Line),
        "tab" => c.emit_break(&snapshot, Break::Tab),
        "cell" => {
            c.flush_bytes(&snapshot);
            c.emit_str(" ", &snapshot);
        }
        "b" => {
            c.flush_bytes(&snapshot);
            st.bold = on;
        }
        "i" => {
            c.flush_bytes(&snapshot);
            st.italic = on;
        }
        "ul" | "uld" | "uldb" | "ulw" | "ulth" | "uldash" => {
            c.flush_bytes(&snapshot);
            st.underline = on;
        }
        "ulnone" => {
            c.flush_bytes(&snapshot);
            st.underline = false;
        }
        "plain" => {
            c.flush_bytes(&snapshot);
            st.bold = false;
            st.italic = false;
            st.underline = false;
        }
        _ => {
            let sym = match word {
                "emdash" => "\u{2014}",
                "endash" => "\u{2013}",
                "bullet" => "\u{2022}",
                "lquote" => "\u{2018}",
                "rquote" => "\u{2019}",
                "ldblquote" => "\u{201C}",
                "rdblquote" => "\u{201D}",
                "emspace" | "enspace" | "qmspace" => " ",
                _ => "",
            };
            if !sym.is_empty() {
                c.flush_bytes(&snapshot);
                if c.uc_skip > 0 {
                    c.uc_skip -= 1;
                } else {
                    c.emit_str(sym, &snapshot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn de_encapsulates_html() {
        // the [MS-OXRTFEX] 3.1 example (abridged)
        let rtf = br#"{\rtf1\ansi\ansicpg1252\fromhtml1 \deff0{\fonttbl
{\f0\fswiss Arial;}
{\f1\fmodern Courier New;}
{\f2\fnil\fcharset2 Symbol;}
{\f3\fmodern\fcharset0 Courier New;}}
{\colortbl\red0\green0\blue0;\red0\green0\blue255;}
\uc1\pard\plain\deftab360 \f0\fs24
{\*\htmltag19 <html>}
{\*\htmltag34 <head>}
{\*\htmltag161 <title>}
{\*\htmltag241 </title>}
{\*\htmltag41 </head>}
{\*\htmltag50 <body>}\htmlrtf {\htmlrtf0
{\*\htmltag64 <p>}
{\*\htmltag84 <a href="http://example.com">}\htmlrtf {\field{\*\fldinst{HYPERLINK "http://example.com"}}{\fldrslt\cf1\ul \htmlrtf0 example\htmlrtf }\htmlrtf0 \htmlrtf }\htmlrtf0
{\*\htmltag92 </a>}
{\*\htmltag244 &nbsp;}\htmlrtf \htmlrtf0 \'e9t\'e9 \{x\}
{\*\htmltag72 </p>}\htmlrtf \par
}\htmlrtf0
{\*\htmltag58 </body>}
{\*\htmltag27 </html>}}"#;
        let Rtf::Html(h) = convert(rtf) else { panic!("not html") };
        assert_eq!(
            h,
            "<html><head><title></title></head><body><p><a href=\"http://example.com\">example</a>&nbsp;été {x}</p></body></html>"
        );
    }

    #[test]
    fn htmltag_with_hebrew_bytes_and_par() {
        let rtf = b"{\\rtf1\\ansi\\ansicpg1255\\fromhtml1 {\\*\\htmltag96 <div dir=rtl>}\\'f9\\'ec\\'e5\\'ed{\\*\\htmltag64 \\par </div>}}";
        assert_eq!(convert(rtf), Rtf::Html("<div dir=rtl>שלום\r\n</div>".into()));
    }

    #[test]
    fn plain_rtf_to_html() {
        let rtf = br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss Arial;}}{\colortbl;\red255\green0\blue0;}\viewkind4\uc1\pard\f0\fs20 Hello \b bold\b0  and \i italic\i0 <tag> & \ul under\ulnone\par Second line\line third\tab x\par}";
        let Rtf::Rich(h) = convert(rtf) else { panic!() };
        assert_eq!(
            h,
            "Hello <span style=\"font-weight:bold;\">bold</span> and <span style=\"font-style:italic;\">italic</span>&lt;tag&gt; &amp; <span style=\"text-decoration:underline;\">under</span><br>\nSecond line<br>\nthird&nbsp;&nbsp;&nbsp;&nbsp;x<br>\n"
        );
    }

    #[test]
    fn hebrew_by_ansicpg_and_by_font_charset() {
        // document code page 1255
        let a = b"{\\rtf1\\ansi\\ansicpg1255{\\fonttbl{\\f0 Arial;}}\\f0 \\'f9\\'ec\\'e5\\'ed \\'e5\\'e1\\'f8\\'eb\\'e4\\par}";
        assert_eq!(convert(a), Rtf::Rich("שלום וברכה<br>\n".into()));
        // document in 1252 but the run's font is Hebrew (\fcharset177)
        let b = b"{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0\\fswiss Arial;}{\\f1\\fnil\\fcharset177 David;}}\\f0 caf\\'e9 {\\f1 \\'f9\\'ec\\'e5\\'ed}}";
        assert_eq!(convert(b), Rtf::Rich("café שלום".into()));
    }

    #[test]
    fn unicode_escapes_skip_fallbacks() {
        let rtf = b"{\\rtf1\\ansi\\uc1 \\u1513?\\u1500\\'3f\\uc2\\u1493??\\u-4064 x}";
        // \u-4064 = U+F020 (private use, from Symbol fonts) — still decoded, not a crash
        let Rtf::Rich(h) = convert(rtf) else { panic!() };
        assert!(h.starts_with("שלו"), "{h}");
        assert!(!h.contains('?'), "{h}");
    }

    #[test]
    fn from_text_and_skipped_destinations() {
        let rtf = b"{\\rtf1\\ansi\\fromtext{\\*\\generator Riched20;}{\\info{\\author X}}line one\\par line two{\\pict\\wmetafile8 0102ff}\\bin3 \x00\x01\x02 end}";
        assert_eq!(convert(rtf), Rtf::Text("line one\r\nline two end".into()));
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let deep = "{".repeat(10_000) + "x" + &"}".repeat(10_000);
        let _ = convert(deep.as_bytes());
        let _ = convert(b"\\");
        let _ = convert(b"{\\'");
        let _ = convert(b"{\\u99999999999 \\bin999999999}");
        let _ = convert(b"}}}}{{{");
    }
}
