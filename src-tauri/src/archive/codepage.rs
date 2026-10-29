//! Windows code pages → encoding_rs (ANSI PST strings, RTF `\ansicpg` / `\fcharset`, PR_INTERNET_CPID).

use encoding_rs::Encoding;

/// encoding_rs encoding for a Windows code page number.
pub fn encoding(cp: u32) -> Option<&'static Encoding> {
    let label = match cp {
        65001 => "utf-8",
        1200 => "utf-16le",
        1201 => "utf-16be",
        874 => "windows-874",
        932 => "shift_jis",
        936 | 51936 => "gbk",
        54936 => "gb18030",
        949 | 51949 => "euc-kr",
        950 => "big5",
        1250 => "windows-1250",
        1251 => "windows-1251",
        1252 | 20127 | 28591 => "windows-1252",
        1253 => "windows-1253",
        1254 => "windows-1254",
        1255 => "windows-1255",
        1256 => "windows-1256",
        1257 => "windows-1257",
        1258 => "windows-1258",
        10000 => "macintosh",
        20866 => "koi8-r",
        21866 => "koi8-u",
        28592 => "iso-8859-2",
        28593 => "iso-8859-3",
        28594 => "iso-8859-4",
        28595 => "iso-8859-5",
        28596 => "iso-8859-6",
        28597 => "iso-8859-7",
        // logical-order Hebrew is what mailers really send
        28598 | 38598 => "windows-1255",
        28599 => "windows-1254",
        28603 => "iso-8859-13",
        28605 => "iso-8859-15",
        50220..=50222 => "iso-2022-jp",
        51932 => "euc-jp",
        _ => return None,
    };
    Encoding::for_label(label.as_bytes())
}

/// RTF `\fcharsetN` → code page (0 = ANSI → the document's, 2 = Symbol → none).
pub fn for_charset(charset: i32) -> Option<u32> {
    Some(match charset {
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return None,
    })
}

/// Decode bytes in code page `cp` (unknown → windows-1252).
pub fn decode(bytes: &[u8], cp: u32) -> String {
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let enc = encoding(cp).unwrap_or(encoding_rs::WINDOWS_1252);
    enc.decode(bytes).0.into_owned()
}

/// The code page of an 8-bit string from an ANSI PST that carries no code page of its own:
/// windows-1255 when the high bytes look like Hebrew letters (runs in 0xE0..=0xFA), else 1252.
/// Archives of an Israeli user are far more often Hebrew than accented Latin in such runs.
pub fn guess(bytes: &[u8]) -> u32 {
    if bytes.is_ascii() {
        return 1252;
    }
    let high = bytes.iter().filter(|b| **b >= 0x80).count();
    let heb = bytes.iter().filter(|b| (0xE0..=0xFA).contains(*b)).count();
    let mut run = 0;
    let mut best = 0;
    for b in bytes {
        if (0xE0..=0xFA).contains(b) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    if best >= 3 && heb * 10 >= high * 8 { 1255 } else { 1252 }
}

/// Code pages that are often claimed for text that is really Hebrew (Israeli mailers of the 2000s
/// labelled windows-1255 text as ISO-8859-1).
pub fn is_weak_latin(cp: u32) -> bool {
    matches!(cp, 1252 | 28591 | 20127)
}

/// Hebrew that was decoded as Latin-1 / windows-1252 ("ùìåí" for "שלום") → the Hebrew, when the
/// text clearly looks like that: most of its non-ASCII letters are in à..ú with runs of three or
/// more, and it can be turned back into the original bytes. None when it doesn't look mis-decoded.
pub fn repair_hebrew(s: &str) -> Option<String> {
    let mut letters = 0usize;
    let mut in_range = 0usize;
    let mut run = 0usize;
    let mut best = 0usize;
    for c in s.chars() {
        let u = c as u32;
        let hit = (0xE0..=0xFA).contains(&u);
        if hit {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
        if u >= 0x80 && c.is_alphabetic() {
            letters += 1;
            if hit {
                in_range += 1;
            }
        }
    }
    if best < 3 {
        return None;
    }
    if in_range * 10 >= letters * 9 {
        let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(s);
        if !unmappable {
            return Some(encoding_rs::WINDOWS_1255.decode(&bytes).0.into_owned());
        }
    }
    // mixed text (a reply quoting a mis-decoded original): fix only the runs of à..ú letters,
    // when there is clear evidence (a run of four or more)
    if best < 4 {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while j < chars.len() && (0xE0..=0xFA).contains(&(chars[j] as u32)) {
            j += 1;
        }
        if j - i >= 2 {
            out.extend(chars[i..j].iter().map(|c| char::from_u32(0x05D0 + (*c as u32 - 0xE0)).unwrap_or(*c)));
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_hebrew_and_latin() {
        assert_eq!(decode(&[0xf9, 0xec, 0xe5, 0xed], 1255), "שלום");
        assert_eq!(decode(b"caf\xe9", 1252), "café");
        assert_eq!(decode(b"caf\xe9", 0), "café");
        assert_eq!(decode("שלום".as_bytes(), 65001), "שלום");
        assert_eq!(guess(&[0xf9, 0xec, 0xe5, 0xed]), 1255);
        assert_eq!(guess(b"caf\xe9 cr\xe8me br\xfbl\xe9e"), 1252);
        assert_eq!(guess(b"plain"), 1252);
        assert_eq!(for_charset(177), Some(1255));
        assert_eq!(for_charset(0), None);
        assert_eq!(repair_hebrew("ùìåí åáøëä").as_deref(), Some("שלום וברכה"));
        assert_eq!(repair_hebrew("<p dir=rtl>ùìåí&nbsp;Kenny</p>").as_deref(), Some("<p dir=rtl>שלום&nbsp;Kenny</p>"));
        assert_eq!(repair_hebrew("crème brûlée, déjà vu"), None);
        assert_eq!(repair_hebrew("plain ascii"), None);
        // mixed: real Hebrew + a mis-decoded quote
        assert_eq!(repair_hebrew("שלום. > ùìåí åáøëä").as_deref(), Some("שלום. > שלום וברכה"));
        assert_eq!(repair_hebrew("שלום ùìå"), None);
    }
}
