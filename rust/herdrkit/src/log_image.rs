//! Pictures stored in session logs (base64 in the JSON). Items keep only a `Ref`; the bytes are read back from the log on demand.
//! Where they are: Claude Code `image` blocks (the owner's paste) and `image` blocks in `tool_result` content; Codex
//! `input_image` data URLs in a message or a tool's output; pi `image` blocks (`data` + `mimeType`) in a `toolResult`.
//! A picture given as a web URL is never fetched.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::parser::json::{self, Obj};

pub use crate::models::{ImageFormat as Format, ImageRef as Ref};

/// `Format.rawValue`
pub fn format_raw(f: Format) -> &'static str {
    match f {
        Format::Png => "png",
        Format::Jpeg => "jpeg",
        Format::Webp => "webp",
        Format::Gif => "gif",
    }
}

/// A picture's identity: the first 8 bytes of the SHA-256 of its base64 text.
pub fn fingerprint(base64: &str) -> u64 {
    let d = Sha256::digest(base64.as_bytes());
    d[..8].iter().fold(0u64, |a, b| a << 8 | *b as u64)
}

/// Longest line `read` takes (a tool output of several screenshots).
pub const MAX_LINE: usize = 64 * 1024 * 1024;

// MARK: size from the header

pub fn header(b: &[u8]) -> Option<(Format, i64, i64)> {
    fn ok(f: Format, w: i64, h: i64) -> Option<(Format, i64, i64)> {
        (w > 0 && h > 0).then_some((f, w, h))
    }
    let be16 = |i: usize| (b[i] as i64) << 8 | b[i + 1] as i64;
    let le16 = |i: usize| b[i] as i64 | (b[i + 1] as i64) << 8;
    let le24 = |i: usize| le16(i) | (b[i + 2] as i64) << 16;
    if b.len() >= 24 && b[0..8] == [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] && b[12..16] == *b"IHDR" {
        return ok(Format::Png, be16(16) << 16 | be16(18), be16(20) << 16 | be16(22));
    }
    if b.len() >= 10 && (b[0..6] == *b"GIF87a" || b[0..6] == *b"GIF89a") {
        return ok(Format::Gif, le16(6), le16(8));
    }
    if b.len() >= 16 && b[0..4] == *b"RIFF" && b[8..12] == *b"WEBP" {
        return match &b[12..16] {
            b"VP8X" => {
                if b.len() < 30 {
                    return None;
                }
                ok(Format::Webp, le24(24) + 1, le24(27) + 1)
            }
            b"VP8L" => {
                if b.len() < 25 || b[20] != 0x2f {
                    return None;
                }
                let bits = le16(21) | le16(23) << 16;
                ok(Format::Webp, (bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
            }
            b"VP8 " => {
                if b.len() < 30 || b[23..26] != [0x9d, 0x01, 0x2a] {
                    return None;
                }
                ok(Format::Webp, le16(26) & 0x3fff, le16(28) & 0x3fff)
            }
            _ => None,
        };
    }
    if b.len() < 4 || b[0] != 0xff || b[1] != 0xd8 {
        return None;
    }
    // JPEG: walk the segments to the frame header (SOF0-15 but DHT, JPG, DAC).
    let mut i = 2usize;
    while i + 8 < b.len() {
        if b[i] != 0xff {
            return None;
        }
        let m = b[i + 1];
        if m == 0xff {
            i += 1;
            continue;
        }
        if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) {
            i += 2;
            continue;
        }
        if m == 0xda || m == 0xd9 {
            return None; // scan data or the end before any frame
        }
        if (0xc0..=0xcf).contains(&m) && ![0xc4, 0xc8, 0xcc].contains(&m) {
            return ok(Format::Jpeg, be16(i + 7), be16(i + 5));
        }
        i += 2 + be16(i + 2) as usize;
    }
    None
}

/// Size from the start of base64 text: 48 bytes, or up to 64 KB for a JPEG whose frame sits after EXIF.
pub fn header_base64(base64: &str) -> Option<(Format, i64, i64)> {
    let bytes = base64.as_bytes();
    for chars in [64usize, 87_384] {
        let head = &bytes[..bytes.len().min(chars)];
        let d = decode(&head[..head.len() / 4 * 4])?;
        if let Some(h) = header(&d) {
            return Some(h);
        }
        if !(head.len() == chars && d.len() >= 2 && d[0] == 0xff && d[1] == 0xd8) {
            return None;
        }
    }
    None
}

/// `Data(base64Encoded:options: .ignoreUnknownCharacters)`: characters outside the alphabet are skipped; the rest
/// (with its `=` padding) must come in groups of four.
pub fn decode(text: &[u8]) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut vals: Vec<u32> = Vec::with_capacity(text.len());
    let mut pad = 0usize;
    for &c in text {
        if c == b'=' {
            pad += 1;
        } else if let Some(v) = val(c) {
            if pad > 0 {
                return None; // data after the padding
            }
            vals.push(v);
        }
    }
    if (vals.len() + pad) & 3 != 0 || pad > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(vals.len() / 4 * 3 + 2);
    for g in vals.chunks(4) {
        match g.len() {
            4 => {
                let n = g[0] << 18 | g[1] << 12 | g[2] << 6 | g[3];
                out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
            }
            3 => {
                let n = g[0] << 18 | g[1] << 12 | g[2] << 6;
                out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8]);
            }
            2 => out.push(((g[0] << 18 | g[1] << 12) >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

// MARK: finding them in a line

/// A picture in a line: its base64, and the tool call it answers (nil: the message's own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found<'a> {
    pub base64: &'a str,
    pub call: Option<&'a str>,
}

/// Pictures in a parsed line, in log order. The order is what `Ref.index` counts.
pub fn found_in(o: &Obj) -> Vec<Found<'_>> {
    if let Some(p) = json::obj(o.get("payload")) {
        return found_codex(p);
    }
    if let Some(m) = json::obj(o.get("message")) {
        return found_message(m);
    }
    vec![]
}

pub fn found_codex(p: &Obj) -> Vec<Found<'_>> {
    let ty = json::str_(p.get("type")).unwrap_or("");
    if ty == "message" {
        return found_blocks(p.get("content"), None);
    }
    if ty.ends_with("_output") {
        return found_blocks(p.get("output"), json::str_(p.get("call_id")));
    }
    vec![]
}

/// Claude Code and pi messages. A pi `toolResult` message answers its `toolCallId`.
pub fn found_message(m: &Obj) -> Vec<Found<'_>> {
    let call = if json::str_(m.get("role")) == Some("toolResult") { json::str_(m.get("toolCallId")) } else { None };
    found_blocks(m.get("content"), call)
}

fn found_blocks<'a>(v: Option<&'a Value>, call: Option<&'a str>) -> Vec<Found<'a>> {
    let Some(blocks) = json::objs(v) else { return vec![] };
    let mut out = vec![];
    for b in blocks {
        if let Some(s) = base64_of(b) {
            out.push(Found { base64: s, call });
        } else if json::str_(b.get("type")) == Some("tool_result") {
            if let Some(id) = json::str_(b.get("tool_use_id")) {
                out.extend(found_blocks(b.get("content"), Some(id)));
            }
        }
    }
    out
}

fn base64_of(b: &Obj) -> Option<&str> {
    match json::str_(b.get("type")) {
        Some("image") => {
            if let Some(src) = json::obj(b.get("source")) {
                return if json::str_(src.get("type")) == Some("base64") { json::str_(src.get("data")) } else { None };
            }
            json::str_(b.get("data"))
        }
        Some("input_image") => {
            let url = json::str_(b.get("image_url"))?;
            if !crate::parser::text::has_prefix(url, "data:image/") {
                return None;
            }
            let comma = url.find(',')?;
            if !crate::parser::text::has_suffix(&url[..comma], ";base64") {
                return None;
            }
            Some(&url[comma + 1..])
        }
        _ => None,
    }
}

/// Refs for the pictures whose header can be read; `index` still counts the others, as `data` does.
pub fn refs(found: &[Found<'_>], offset: Option<u64>) -> Vec<(Option<String>, Ref)> {
    found
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            header_base64(f.base64).map(|(format, width, height)| {
                (f.call.map(str::to_string), Ref { format, width, height, offset, index: i, digest: fingerprint(f.base64) })
            })
        })
        .collect()
}

// MARK: reading back

/// The bytes of the nth picture in a log line.
pub fn data(line: &str, index: usize) -> Option<Vec<u8>> {
    data_bytes(line.as_bytes(), index)
}

pub fn data_bytes(line: &[u8], index: usize) -> Option<Vec<u8>> {
    base64_at(line, index).and_then(|s| decode(s.as_bytes()))
}

pub fn base64_at(line: &[u8], index: usize) -> Option<String> {
    let o = match serde_json::from_slice::<Value>(line) {
        Ok(Value::Object(o)) => o,
        _ => return None,
    };
    found_in(&o).get(index).map(|f| f.base64.to_string())
}

/// The picture `r` points at in the log file. Nil unless `offset` starts a line and the picture there still has the
/// ref's fingerprint, format and size (a rewritten log does not show another picture).
pub fn read(r: &Ref, path: &Path) -> Option<Vec<u8>> {
    let offset = r.offset?;
    let mut f = std::fs::File::open(path).ok()?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset - 1)).ok()?;
        let mut one = [0u8; 1];
        if f.read(&mut one).ok()? != 1 || one[0] != 0x0a {
            return None;
        }
    } else {
        f.seek(SeekFrom::Start(0)).ok()?;
    }
    let mut line = Vec::new();
    let mut chunk = vec![0u8; 1 << 20];
    while line.len() < MAX_LINE {
        let n = f.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        if let Some(nl) = chunk[..n].iter().position(|&c| c == 0x0a) {
            line.extend_from_slice(&chunk[..nl]);
            break;
        }
        line.extend_from_slice(&chunk[..n]);
    }
    let b64 = base64_at(&line, r.index)?;
    if fingerprint(&b64) != r.digest {
        return None;
    }
    let d = decode(b64.as_bytes())?;
    let (format, width, height) = header(&d)?;
    (format == r.format && width == r.width && height == r.height).then_some(d)
}
