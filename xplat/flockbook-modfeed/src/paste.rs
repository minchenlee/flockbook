//! Swift: `Paste.swift`. `paste-image`: the clipboard's image as a PNG file, so the mod can attach it like a picked
//! file (herdr-app's mod API reads only clipboard text).

use chrono::{DateTime, Local};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Files older than this in the paste folder are removed on each paste.
pub const KEEP: Duration = Duration::from_secs(24 * 3600);

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn run(cmd: &str, args: &[&str]) -> Option<Vec<u8>> {
    let o = Command::new(cmd).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    (o.status.success() && !o.stdout.is_empty()).then_some(o.stdout)
}

/// The clipboard's image as PNG data, or None when it holds none.
/// Swift reads NSPasteboard (PNG, or TIFF converted). Here: macOS `osascript` (PNG only), Linux `wl-paste` or `xclip`.
pub fn clipboard_png() -> Option<Vec<u8>> {
    if cfg!(target_os = "macos") {
        // «data PNGf89504E47…»
        let out = run("osascript", &["-e", "the clipboard as «class PNGf»"])?;
        let s = String::from_utf8_lossy(&out);
        let hex = s.trim().strip_prefix("«data PNGf")?.strip_suffix('»')?;
        let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
        return bytes.filter(|b| b.starts_with(PNG_MAGIC));
    }
    if cfg!(all(unix, not(target_os = "macos"))) {
        let png = run("wl-paste", &["--no-newline", "--type", "image/png"])
            .or_else(|| run("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]));
        return png.filter(|b| b.starts_with(PNG_MAGIC));
    }
    // TODO(windows): read CF_DIB / PNG from the Windows clipboard. Until then, no image (the mod says so).
    None
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
