//! Swift: `Paste.swift`. `paste-image`: what ⌘V in the reply box attaches instead of typing text (herdr-app's mod
//! API reads only clipboard text). Copied files win (Finder puts their names as text too). Then plain text is the
//! box's own paste, also with an image beside it (Excel, Numbers and Word put a picture of the cells). Then an image,
//! as a PNG file: a screenshot, a browser's Copy Image (image and HTML); TIFF and other types become PNG on macOS.

use chrono::{DateTime, Local};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Files older than this in the paste folder are removed on each paste.
pub const KEEP: Duration = Duration::from_secs(24 * 3600);

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// What the clipboard holds that the reply box attaches.
#[derive(Debug, PartialEq, Eq)]
pub enum Clip {
    /// Local files (a Finder or file manager copy), in their order.
    Files(Vec<String>),
    /// An image, as PNG data.
    Image(Vec<u8>),
    /// Plain text (with or without an image), or empty: the box's own paste.
    Nothing,
}

#[cfg(unix)]
fn run(cmd: &str, args: &[&str]) -> Option<Vec<u8>> {
    let o = Command::new(cmd).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    (o.status.success() && !o.stdout.is_empty()).then_some(o.stdout)
}

/// The clipboard's files or image. macOS: AppKit's pasteboard through `osascript`. Linux: `wl-paste` or `xclip`.
pub fn clipboard() -> Clip {
    #[cfg(target_os = "macos")]
    return mac_clip(None);
    #[cfg(all(unix, not(target_os = "macos")))]
    return linux_clip(|t| {
        let wl = match t {
            None => run("wl-paste", &["--list-types"]),
            Some(t) => run("wl-paste", &["--no-newline", "--type", t]),
        };
        wl.or_else(|| run("xclip", &["-selection", "clipboard", "-t", t.unwrap_or("TARGETS"), "-o"]))
    });
    // TODO(windows): CF_HDROP files and CF_DIB / PNG images. Until then, nothing (the box pastes the text).
    #[allow(unreachable_code)]
    Clip::Nothing
}

/// JXA: files (`NSURL`, file URLs only) first; plain text next (nothing to attach); then an image: the PNG as it is,
/// or any image AppKit reads (TIFF, JPEG, HEIC, PDF...) drawn into PNG. Prints {"files":[...]}, {"png":"<base64>"}
/// or {}.
#[cfg(target_os = "macos")]
const MAC_SCRIPT: &str = r#"ObjC.import("AppKit");
function run(argv) {
  const pb = argv.length && argv[0] ? $.NSPasteboard.pasteboardWithName(argv[0]) : $.NSPasteboard.generalPasteboard;
  const urls = pb.readObjectsForClassesOptions($([$.NSURL]), $({ NSPasteboardURLReadingFileURLsOnlyKey: true }));
  const files = [];
  if (urls && !urls.isNil()) for (let i = 0; i < urls.count; i++) files.push(urls.objectAtIndex(i).path.js);
  if (files.length) return JSON.stringify({ files });
  if (pb.availableTypeFromArray($(["public.utf8-plain-text", "public.plain-text"])).js) return "{}";
  let png = pb.dataForType("public.png");
  if (png.isNil()) {
    if (!$.NSImage.canInitWithPasteboard(pb)) return "{}";
    const img = $.NSImage.alloc.initWithPasteboard(pb);
    if (img.isNil()) return "{}";
    const rep = $.NSBitmapImageRep.imageRepWithData(img.TIFFRepresentation);
    if (rep.isNil()) return "{}";
    png = rep.representationUsingTypeProperties(4, $({}));
    if (png.isNil()) return "{}";
  }
  return JSON.stringify({ png: png.base64EncodedStringWithOptions(0).js });
}"#;

/// macOS: the general pasteboard, or the named one (tests use a private pasteboard).
#[cfg(target_os = "macos")]
pub fn mac_clip(pasteboard: Option<&str>) -> Clip {
    let Some(out) = run("osascript", &["-l", "JavaScript", "-e", MAC_SCRIPT, pasteboard.unwrap_or("")]) else { return Clip::Nothing };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out) else { return Clip::Nothing };
    if let Some(files) = v["files"].as_array() {
        let files: Vec<String> = files.iter().filter_map(|f| f.as_str()).filter(|f| !f.is_empty()).map(String::from).collect();
        if !files.is_empty() {
            return Clip::Files(files);
        }
    }
    use base64::Engine;
    match v["png"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()) {
        Some(png) if png.starts_with(PNG_MAGIC) => Clip::Image(png),
        _ => Clip::Nothing,
    }
}

/// Linux: `read(None)` lists the clipboard's types, one per line; `read(Some(type))` gives that type's data.
/// Files: `text/uri-list`, or GNOME's `x-special/gnome-copied-files`. Then plain text (`text/plain...`,
/// `UTF8_STRING`, `STRING`, `TEXT`): nothing. Then the image: `image/png`.
pub fn linux_clip(read: impl Fn(Option<&str>) -> Option<Vec<u8>>) -> Clip {
    let types = read(None).map(|b| String::from_utf8_lossy(&b).lines().map(|l| l.trim().to_string()).collect::<Vec<_>>()).unwrap_or_default();
    let has = |t: &str| types.iter().any(|x| x == t);
    for t in ["text/uri-list", "x-special/gnome-copied-files"] {
        if has(t) {
            let files = read(Some(t)).map(|b| uri_list(&String::from_utf8_lossy(&b))).unwrap_or_default();
            if !files.is_empty() {
                return Clip::Files(files);
            }
        }
    }
    if types.iter().any(|t| t.starts_with("text/plain") || matches!(t.as_str(), "UTF8_STRING" | "STRING" | "TEXT")) {
        return Clip::Nothing;
    }
    match has("image/png").then(|| read(Some("image/png"))).flatten() {
        Some(png) if png.starts_with(PNG_MAGIC) => Clip::Image(png),
        _ => Clip::Nothing,
    }
}

/// The local paths in a URI list (RFC 2483): `file:` URLs on this machine, percent-decoded. Comments, other
/// schemes, other hosts and paths that are not UTF-8 are left out. GNOME's "copy"/"cut" first line too.
pub fn uri_list(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.trim())
        .filter_map(|l| l.strip_prefix("file://"))
        .filter_map(|rest| {
            let path = if rest.starts_with('/') { rest } else { rest.strip_prefix("localhost")? };
            path.starts_with('/').then(|| percent_decode(path)).flatten()
        })
        .collect()
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())?;
            out.push(h);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The name `save` gives a paste: "paste-YYYYMMDD-HHMMSS-xxxxxx.png". Only these are ever removed.
pub fn is_paste(name: &str) -> bool {
    let b = name.as_bytes();
    name.len() == 32
        && name.starts_with("paste-")
        && name.ends_with(".png")
        && b[6..14].iter().all(u8::is_ascii_digit)
        && b[14] == b'-'
        && b[15..21].iter().all(u8::is_ascii_digit)
        && b[21] == b'-'
        && b[22..28].iter().all(u8::is_ascii_hexdigit)
}

fn random_hex6() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    format!("{:06X}", h.finish() & 0xFF_FFFF)
}

/// Writes `png` into `dir` as a new file and returns its path; removes its own pastes older than `KEEP`. Other files
/// in `dir` are never touched.
pub fn save(png: &[u8], dir: &Path, now: SystemTime) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if !is_paste(&e.file_name().to_string_lossy()) {
                continue;
            }
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                if now.duration_since(m).is_ok_and(|d| d > KEEP) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    let stamp = DateTime::<Local>::from(now).format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("paste-{stamp}-{}.png", random_hex6()));
    let tmp = dir.join(format!(".paste-{}.tmp", std::process::id()));
    std::fs::write(&tmp, png)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_paste("paste-20261103-014000-A1B2C3.png"));
        assert!(!is_paste("paste-20261103-014000-A1B2C.png"));
        assert!(!is_paste("notes.png"));
    }
}
