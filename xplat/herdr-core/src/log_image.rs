//! Images stay as references into a local JSONL file. Remote URLs are never fetched.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub const MAX_LINE: usize = 64 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub offset: Option<u64>,
    pub index: usize,
    pub digest: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(width: u32) -> String {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(width.to_be_bytes());
        b.extend(2u32.to_be_bytes());
        STANDARD.encode(b)
    }
    #[test]
    fn extracts_tool_images_without_fetching_urls() {
        let v = serde_json::json!({"message":{"content":[{"type":"input_image","image_url":"https://invalid/image.png"},{"type":"tool_result","tool_use_id":"c","content":[{"type":"image","source":{"type":"base64","data":encoded(3)}}]}]}});
        let r = refs(&v, Some(9));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0.as_deref(), Some("c"));
        assert_eq!(
            (r[0].1.width, r[0].1.height, r[0].1.offset),
            (3, 2, Some(9))
        );
        assert!(header(&[0; 32]).is_none());
    }
    #[test]
    fn fingerprint_is_sha256_prefix() {
        assert_eq!(fingerprint("abc"), 0xba7816bf8f01cfea);
    }
    #[test]
    fn rejects_middle_offsets_truncation_and_same_size_rewrites() {
        let path = std::env::temp_dir().join(format!(
            "picture-ref-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let line = |w| {
            serde_json::json!({"message":{"content":[{"type":"image","data":encoded(w)}]}})
                .to_string()
        };
        std::fs::write(&path, format!("{}\n", line(3))).unwrap();
        let r = refs(&serde_json::from_str::<Value>(&line(3)).unwrap(), Some(0))[0]
            .1
            .clone();
        assert!(read(&r, &path).is_some());
        let mut middle = r.clone();
        middle.offset = Some(1);
        assert!(read(&middle, &path).is_none());
        std::fs::write(&path, format!("{}\n", line(4))).unwrap();
        assert!(read(&r, &path).is_none());
        std::fs::write(&path, "{").unwrap();
        assert!(read(&r, &path).is_none());
        // The file is a synthetic fixture in the OS temporary directory.
    }
}

pub fn fingerprint(base64: &str) -> u64 {
    let digest = Sha256::digest(base64.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().unwrap())
}

/// Reads dimensions without decoding raster pixels. All accepted dimensions obey the decode budget.
pub fn header(b: &[u8]) -> Option<(ImageFormat, u32, u32)> {
    let be16 = |i| u16::from_be_bytes([b[i], b[i + 1]]) as u32;
    let le16 = |i| u16::from_le_bytes([b[i], b[i + 1]]) as u32;
    let le24 = |i| le16(i) | ((b[i + 2] as u32) << 16);
    let h = if b.len() >= 24 && &b[..8] == b"\x89PNG\r\n\x1a\n" && &b[12..16] == b"IHDR" {
        (
            ImageFormat::Png,
            u32::from_be_bytes(b[16..20].try_into().ok()?),
            u32::from_be_bytes(b[20..24].try_into().ok()?),
        )
    } else if b.len() >= 10 && (&b[..6] == b"GIF87a" || &b[..6] == b"GIF89a") {
        (ImageFormat::Gif, le16(6), le16(8))
    } else if b.len() >= 30 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        match &b[12..16] {
            b"VP8X" => (ImageFormat::Webp, le24(24) + 1, le24(27) + 1),
            b"VP8L" if b[20] == 0x2f => {
                let bits = u32::from_le_bytes(b[21..25].try_into().ok()?);
                (
                    ImageFormat::Webp,
                    (bits & 0x3fff) + 1,
                    ((bits >> 14) & 0x3fff) + 1,
                )
            }
            b"VP8 " if &b[23..26] == b"\x9d\x01\x2a" => {
                (ImageFormat::Webp, le16(26) & 0x3fff, le16(28) & 0x3fff)
            }
            _ => return None,
        }
    } else if b.len() >= 4 && b[..2] == [0xff, 0xd8] {
        let mut i = 2;
        loop {
            if i + 8 >= b.len() || b[i] != 0xff {
                return None;
            }
            let m = b[i + 1];
            if m == 0xff {
                i += 1;
                continue;
            }
            if m == 0xd8 || m == 1 || (0xd0..=0xd7).contains(&m) {
                i += 2;
                continue;
            }
            if m == 0xda || m == 0xd9 {
                return None;
            }
            if (0xc0..=0xcf).contains(&m) && ![0xc4, 0xc8, 0xcc].contains(&m) {
                break (ImageFormat::Jpeg, be16(i + 7), be16(i + 5));
            }
            let len = be16(i + 2) as usize;
            if len < 2 {
                return None;
            }
            i += 2 + len;
        }
    } else {
        return None;
    };
    (h.1 > 0 && h.2 > 0 && h.1 <= 32768 && h.2 <= 32768 && h.1 as u64 * h.2 as u64 <= MAX_PIXELS)
        .then_some(h)
}

fn found<'a>(o: &'a Value) -> Vec<(Option<String>, &'a str)> {
    fn blocks<'a>(v: &'a Value, call: Option<&str>, out: &mut Vec<(Option<String>, &'a str)>) {
        let Some(bs) = v.as_array() else { return };
        for b in bs {
            let data = match b["type"].as_str() {
                Some("image") => {
                    if b["source"].is_object() {
                        (b["source"]["type"] == "base64")
                            .then(|| b["source"]["data"].as_str())
                            .flatten()
                    } else {
                        b["data"].as_str()
                    }
                }
                Some("input_image") => b["image_url"].as_str().and_then(|s| {
                    let (meta, data) = s.split_once(',')?;
                    (meta.starts_with("data:image/") && meta.ends_with(";base64")).then_some(data)
                }),
                _ => None,
            };
            if let Some(s) = data {
                out.push((call.map(str::to_string), s));
            } else if b["type"] == "tool_result" {
                blocks(&b["content"], b["tool_use_id"].as_str(), out)
            }
        }
    }
    let mut out = Vec::new();
    if let Some(p) = o.get("payload") {
        if p["type"] == "message" {
            blocks(&p["content"], None, &mut out)
        } else if p["type"].as_str().is_some_and(|t| t.ends_with("_output")) {
            blocks(&p["output"], p["call_id"].as_str(), &mut out)
        }
    } else if let Some(m) = o.get("message") {
        blocks(
            &m["content"],
            if m["role"] == "toolResult" {
                m["toolCallId"].as_str()
            } else {
                None
            },
            &mut out,
        )
    }
    out
}

/// `(tool_call_id, reference)` in log order. Invalid headers still consume an index.
pub fn refs(o: &Value, offset: Option<u64>) -> Vec<(Option<String>, ImageRef)> {
    found(o)
        .into_iter()
        .enumerate()
        .filter_map(|(index, (call, b64))| {
            let prefix = &b64.as_bytes()[..b64.len().min(87384) / 4 * 4];
            let bytes = STANDARD.decode(prefix).ok()?;
            let (format, width, height) = header(&bytes)?;
            Some((
                call,
                ImageRef {
                    format,
                    width,
                    height,
                    offset,
                    index,
                    digest: fingerprint(b64),
                },
            ))
        })
        .collect()
}

pub fn read(r: &ImageRef, path: &Path) -> Option<Vec<u8>> {
    let offset = r.offset?;
    let mut f = std::fs::File::open(path).ok()?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset - 1)).ok()?;
        let mut b = [0];
        f.read_exact(&mut b).ok()?;
        if b[0] != b'\n' {
            return None;
        }
    } else {
        f.seek(SeekFrom::Start(0)).ok()?;
    }
    let mut line = Vec::new();
    BufReader::new(f)
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut line)
        .ok()?;
    if line.len() > MAX_LINE {
        return None;
    }
    let o: Value = serde_json::from_slice(&line).ok()?;
    let images = found(&o);
    let (_, b64) = images.get(r.index)?;
    if fingerprint(b64) != r.digest {
        return None;
    }
    let bytes = STANDARD.decode(b64).ok()?;
    if header(&bytes)? != (r.format, r.width, r.height) {
        return None;
    }
    Some(bytes)
}
