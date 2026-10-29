//! Compressed RTF ([MS-OXRTFCP], "LZFu"): `PR_RTF_COMPRESSED` → the RTF bytes.
//!
//! Written here rather than taken from the `compressed-rtf` crate because that one returns a
//! `String` (fails on the 8-bit bytes real-world RTF sometimes contains), panics on inputs shorter
//! than the 16-byte header and rejects size / CRC mismatches outright; archives from 2004 are not
//! always that tidy. We decode leniently: a bad CRC or size is ignored, a truncated stream yields
//! what was decoded so far.

const COMPRESSED: u32 = 0x7546_5A4C; // "LZFu"
const UNCOMPRESSED: u32 = 0x414C_454D; // "MELA"

/// The initial dictionary contents ([MS-OXRTFCP] 2.1.2.1), 207 bytes.
const PREBUF: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";

/// Upper bound for the output (a hostile RAWSIZE must not make us allocate gigabytes).
const MAX_OUT: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum LzfuError {
    TooShort,
    UnknownType(u32),
}

impl std::fmt::Display for LzfuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LzfuError::TooShort => write!(f, "compressed RTF is shorter than its header"),
            LzfuError::UnknownType(t) => write!(f, "unknown compressed RTF type 0x{t:08X}"),
        }
    }
}

impl std::error::Error for LzfuError {}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The CRC [MS-OXRTFCP] uses (CRC-32 table, initial value 0, no final XOR).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut crc = 0u32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// Header fields: (COMPSIZE, RAWSIZE, COMPTYPE, CRC).
pub fn header(data: &[u8]) -> Result<(u32, u32, u32, u32), LzfuError> {
    if data.len() < 16 {
        return Err(LzfuError::TooShort);
    }
    Ok((u32_at(data, 0), u32_at(data, 4), u32_at(data, 8), u32_at(data, 12)))
}

/// Decompress `PR_RTF_COMPRESSED`. Returns the raw RTF bytes (normally 7-bit ASCII).
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, LzfuError> {
    let (comp_size, raw_size, kind, _crc) = header(data)?;
    // COMPSIZE counts the bytes after itself; tolerate a wrong value
    let declared_end = (comp_size as usize).saturating_add(4);
    let end = if declared_end > 16 && declared_end <= data.len() { declared_end } else { data.len() };
    let body = &data[16..end];
    let cap = (raw_size as usize).min(MAX_OUT);
    match kind {
        UNCOMPRESSED => Ok(body[..body.len().min(raw_size as usize)].to_vec()),
        COMPRESSED => {
            let mut dict = [0u8; 4096];
            dict[..PREBUF.len()].copy_from_slice(PREBUF);
            let mut wp = PREBUF.len();
            let mut out: Vec<u8> = Vec::with_capacity(cap);
            let mut i = 0;
            'stream: while i < body.len() {
                let control = body[i];
                i += 1;
                for bit in 0..8 {
                    if control & (1 << bit) == 0 {
                        let Some(&b) = body.get(i) else { break 'stream };
                        i += 1;
                        out.push(b);
                        dict[wp] = b;
                        wp = (wp + 1) & 0xFFF;
                    } else {
                        if i + 1 >= body.len() {
                            break 'stream;
                        }
                        let token = u16::from_be_bytes([body[i], body[i + 1]]);
                        i += 2;
                        let offset = (token >> 4) as usize;
                        let len = (token & 0xF) as usize + 2;
                        if offset == wp {
                            break 'stream; // end marker
                        }
                        for k in 0..len {
                            let b = dict[(offset + k) & 0xFFF];
                            out.push(b);
                            dict[wp] = b;
                            wp = (wp + 1) & 0xFFF;
                        }
                    }
                    if out.len() >= MAX_OUT {
                        break 'stream;
                    }
                }
            }
            if raw_size as usize > 0 && out.len() > raw_size as usize {
                out.truncate(raw_size as usize);
            }
            Ok(out)
        }
        other => Err(LzfuError::UnknownType(other)),
    }
}

/// Compress by storing every byte as a literal (valid LZFu, no back-references). Tests and fixtures.
pub fn compress_literal(raw: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut wp = PREBUF.len();
    let mut end_written = false;
    for chunk in raw.chunks(8) {
        if chunk.len() == 8 {
            body.push(0u8);
            body.extend_from_slice(chunk);
            wp = (wp + 8) & 0xFFF;
        } else {
            // the last partial run + the end marker in the same control byte
            body.push(1u8 << chunk.len());
            body.extend_from_slice(chunk);
            wp = (wp + chunk.len()) & 0xFFF;
            body.extend_from_slice(&((wp as u16) << 4).to_be_bytes());
            end_written = true;
        }
    }
    if !end_written {
        body.push(1);
        body.extend_from_slice(&((wp as u16) << 4).to_be_bytes());
    }
    let mut out = Vec::with_capacity(body.len() + 16);
    out.extend_from_slice(&((body.len() + 12) as u32).to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&COMPRESSED.to_le_bytes());
    out.extend_from_slice(&crc32(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [MS-OXRTFCP] 3.1.1 "Example 1: Simple Compressed RTF".
    const EX1: &[u8] = &[
        0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63,
        0x70, 0x67, 0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64,
        0x7d, 0x0a, 0x80, 0x0f, 0xa0,
    ];

    /// [MS-OXRTFCP] 3.1.2 "Example 2: Reading a Token from the Dictionary That Crosses WritePosition".
    const EX2: &[u8] = &[
        0x1a, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xe2, 0xd4, 0x4b, 0x51, 0x41, 0x00, 0x04, 0x20, 0x57, 0x58,
        0x59, 0x5a, 0x0d, 0x6e, 0x7d, 0x01, 0x0e, 0xb0,
    ];

    #[test]
    fn spec_example_1() {
        let (_, raw, kind, crc) = header(EX1).unwrap();
        assert_eq!(kind, COMPRESSED);
        assert_eq!(crc, crc32(&EX1[16..]), "CRC of the spec bytes");
        let out = decompress(EX1).unwrap();
        assert_eq!(out.len(), raw as usize);
        assert_eq!(out, b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n");
    }

    #[test]
    fn spec_example_2_token_crossing_write_position() {
        assert_eq!(header(EX2).unwrap().3, crc32(&EX2[16..]));
        assert_eq!(decompress(EX2).unwrap(), b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}");
    }

    #[test]
    fn uncompressed_mela() {
        let raw = b"{\\rtf1 plain}";
        let mut d = Vec::new();
        d.extend_from_slice(&((raw.len() + 12) as u32).to_le_bytes());
        d.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        d.extend_from_slice(&UNCOMPRESSED.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(raw);
        assert_eq!(decompress(&d).unwrap(), raw);
    }

    #[test]
    fn literal_roundtrip_and_hostile_inputs() {
        for s in [&b""[..], b"x", b"12345678", b"{\\rtf1\\ansi\\ansicpg1255 \\'f9\\'ec\\'e5\\'ed}", &[0xffu8; 300][..]] {
            assert_eq!(decompress(&compress_literal(s)).unwrap(), s);
        }
        assert_eq!(decompress(&[1, 2, 3]), Err(LzfuError::TooShort));
        let mut bad = compress_literal(b"abc");
        bad[8] = b'X';
        assert!(matches!(decompress(&bad), Err(LzfuError::UnknownType(_))));
        // truncated stream: what was there so far
        let full = compress_literal(b"abcdefghijklmnop");
        assert_eq!(decompress(&full[..full.len() - 6]).unwrap(), b"abcdefghijklm");
        // wrong CRC / size are tolerated
        let mut c = compress_literal(b"hello");
        c[12] ^= 0xFF;
        c[0] = 0xFF;
        assert_eq!(decompress(&c).unwrap(), b"hello");
    }
}
