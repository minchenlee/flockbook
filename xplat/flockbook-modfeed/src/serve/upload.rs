//! Pictures the page uploads (`attach`, POST /api only) for its next `send` (the Mac hub's `HubSession.attach`).
//! Unlike the Mac hub, nothing is re-encoded: PNG, JPEG and WebP are kept, by their first bytes, never by a name or
//! MIME type, without their metadata (`strip`: a JPEG keeps only its Orientation). HEIC (an iPhone's own type) cannot be read here: the page is told to pick JPEG or PNG.
//!
//! The files go to `<herdr-app home>/flockbook/uploads` (mode 700), each mode 600 with a name made here
//! (`phone-YYYYMMDD-HHMMSS-xxxxxx.<ext>`). `send` takes only names this helper holds and types their paths. Only
//! files with such names are ever removed: after `KEEP`, or past `MAX_FILES` / `MAX_FOLDER_BYTES`, oldest first.
//!
//! One owner: the state is the helper's, over all connections (POST /api makes a session per request).

use chrono::{DateTime, Local};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// `HubImage.maxParts`, `maxPartBytes`, `maxBytes`, `maxSide`, `maxPixels`.
pub const MAX_PARTS: usize = 16;
pub const PART: usize = 640 << 10;
pub const MAX_BYTES: usize = 10 << 20;
pub const MAX_SIDE: i64 = 12_000;
pub const MAX_PIXELS: i64 = 50_000_000;
/// Images held for a `send` (the oldest goes past it), and the window limit (`HubUploadLimiter`).
pub const MAX_HELD: usize = 8;
pub const MAX_IMAGES: usize = 20;
pub const MAX_WINDOW_BYTES: usize = 100 << 20;
pub const WINDOW: Duration = Duration::from_secs(600);
/// The folder's own files (`HubImage.cacheMax*`).
pub const KEEP: Duration = Duration::from_secs(7 * 86_400);
pub const MAX_FILES: usize = 200;
pub const MAX_FOLDER_BYTES: u64 = 512 << 20;

pub const HEIC: &str = "This picture type cannot be sent; pick JPEG or PNG.";

struct Open {
    name: String,
    parts: usize,
    next: usize,
    data: Vec<u8>,
}

#[derive(Default)]
pub struct Uploads {
    open: Option<Open>,
    /// Upload name, file.
    held: Vec<(String, PathBuf)>,
    /// When, bytes counted, an image started.
    log: VecDeque<(Instant, usize, bool)>,
}

static UPLOADS: Mutex<Uploads> = Mutex::new(Uploads { open: None, held: Vec::new(), log: VecDeque::new() });

fn state() -> std::sync::MutexGuard<'static, Uploads> {
    UPLOADS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The folder for `home` (the herdr-app home).
pub fn folder(home: &Path) -> PathBuf {
    home.join("flockbook").join("uploads")
}

/// An upload name from the page: 1-32 letters, digits, `_` or `-`.
pub fn name_ok(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// HEIF brands of still images (`HubImage.heifBrands`).
fn heic(b: &[u8]) -> bool {
    b.len() >= 12 && &b[4..8] == b"ftyp" && [&b"heic"[..], b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1"].contains(&&b[8..12])
}

/// The file extension and size of a picture kept as it is; else why not.
pub fn kind(b: &[u8]) -> Result<(&'static str, i64, i64), &'static str> {
    use herdr_core::log_image::Format;
    if heic(b) {
        return Err(HEIC);
    }
    let ext = match herdr_core::log_image::header(b) {
        Some((Format::Png, ..)) => "png",
        Some((Format::Jpeg, ..)) => "jpg",
        Some((Format::Webp, ..)) => "webp",
        _ => return Err("Not a PNG, JPEG or WebP image."),
    };
    let (_, w, h) = herdr_core::log_image::header(b).expect("read above");
    if b.len() > MAX_BYTES || w > MAX_SIDE || h > MAX_SIDE || w.saturating_mul(h) > MAX_PIXELS {
        return Err("The image is too large (at most 12000 px a side, 50 megapixels, 10 MB).");
    }
    Ok((ext, w, h))
}

/// `b` without its metadata (`strip`), by its type; HEIC and other types as they are (`kind` refuses them).
pub fn clean(b: &[u8]) -> Result<Vec<u8>, &'static str> {
    use herdr_core::log_image::Format;
    match herdr_core::log_image::header(b) {
        Some((Format::Jpeg, ..)) => super::strip::jpeg(b),
        Some((Format::Png, ..)) => super::strip::png(b),
        Some((Format::Webp, ..)) => super::strip::webp(b),
        _ => Ok(b.to_vec()),
    }
}

/// A name `write` makes: "phone-YYYYMMDD-HHMMSS-xxxxxx.<png|jpg|webp>". Only these are ever removed.
pub fn own(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    let b = stem.as_bytes();
    ["png", "jpg", "webp"].contains(&ext)
        && b.len() == 28
        && stem.starts_with("phone-")
        && b[6..14].iter().all(u8::is_ascii_digit)
        && b[14] == b'-'
        && b[15..21].iter().all(u8::is_ascii_digit)
        && b[21] == b'-'
        && b[22..28].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

fn random_hex6() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:06x}", h.finish() & 0xFF_FFFF)
}

/// Writes `data` as a new file in `dir` (made mode 700): mode 600, created exclusively (never an existing file or a
/// link), whole before it has its name.
fn write(dir: &Path, data: &[u8], ext: &str, now: SystemTime) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let stamp = DateTime::<Local>::from(now).format("%Y%m%d-%H%M%S");
    for _ in 0..8 {
        let path = dir.join(format!("phone-{stamp}-{}.{ext}", random_hex6()));
        let tmp = dir.join(format!(".phone-{}-{}.tmp", std::process::id(), random_hex6()));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        let done = f.write_all(data).and_then(|_| f.sync_all());
        drop(f);
        if let Err(e) = done {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        // Never over a file already there.
        let taken = std::fs::hard_link(&tmp, &path);
        let _ = std::fs::remove_file(&tmp);
        match taken {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free name"))
}

/// Removes the folder's own files older than `KEEP`, then the oldest past `MAX_FILES` or `MAX_FOLDER_BYTES`, except
/// `keep` (held for a send). Other files are never touched.
pub fn purge(dir: &Path, now: SystemTime, keep: &[PathBuf]) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| own(&e.file_name().to_string_lossy()) && e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.metadata().ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path())))
        .collect();
    files.sort();
    let (mut count, mut bytes) = (files.len(), files.iter().map(|f| f.1).sum::<u64>());
    for (at, len, path) in files {
        let old = now.duration_since(at).is_ok_and(|d| d > KEEP);
        if !(old || count > MAX_FILES || bytes > MAX_FOLDER_BYTES) {
            break;
        }
        if keep.contains(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            count -= 1;
            bytes -= len;
        }
    }
}

type Refusal = (&'static str, String);

fn bad(why: &str) -> Refusal {
    ("badRequest", why.to_string())
}

/// One part of upload `name`. Ok(Some(attached)) after the last part, Ok(None) after another, Err((code, message)).
/// The checks are the Mac hub's, in its order: name, parts, order, the part's size before decoding, the window.
pub fn attach(dir: &Path, name: &str, part: usize, parts: usize, data: &str, now: Instant, wall: SystemTime) -> Result<Option<Value>, Refusal> {
    let mut st = state();
    let open = st.open.take();
    if !name_ok(name) {
        return Err(bad("Upload names are 1-32 letters, digits, _ or -."));
    }
    if !(1..=MAX_PARTS).contains(&parts) || part >= parts {
        return Err(bad(&format!("An image has 1-{MAX_PARTS} parts.")));
    }
    let order = "Parts of one image come in order, one image at a time.";
    match &open {
        Some(o) if o.name != name || o.parts != parts || o.next != part => return Err(bad(order)),
        None if part != 0 => return Err(bad(order)),
        None if st.held.iter().any(|h| h.0 == name) => return Err(bad(&format!("Upload {name} is already held."))),
        _ => {}
    }
    // Base64 of 640 KiB is under 874 KiB; a longer string is refused before decoding.
    if data.len() > PART.div_ceil(3) * 4 {
        return Err(bad("A part is at most 640 KiB."));
    }
    let have = open.as_ref().map_or(0, |o| o.data.len());
    if have + data.len() / 4 * 3 > MAX_BYTES {
        return Err(bad("An image is at most 10 MiB."));
    }
    while st.log.front().is_some_and(|e| now.duration_since(e.0) >= WINDOW) {
        st.log.pop_front();
    }
    let images = st.log.iter().filter(|e| e.2).count() + usize::from(part == 0);
    let bytes = st.log.iter().map(|e| e.1).sum::<usize>() + data.len() / 4 * 3;
    if images > MAX_IMAGES || bytes > MAX_WINDOW_BYTES {
        return Err(("rateLimited", "Too many images. Try again in a few minutes.".into()));
    }
    st.log.push_back((now, data.len() / 4 * 3, part == 0));
    use base64::Engine;
    let Ok(d) = base64::engine::general_purpose::STANDARD.decode(data) else { return Err(bad("Part data is not base64.")) };
    if d.len() > PART {
        return Err(bad("A part is at most 640 KiB."));
    }
    let mut all = open.map(|o| o.data).unwrap_or_default();
    all.extend(d);
    if part + 1 < parts {
        st.open = Some(Open { name: name.into(), parts, next: part + 1, data: all });
        return Ok(None);
    }
    // Metadata out (EXIF and XMP, text chunks; a JPEG keeps its Orientation), then the checks on what is kept.
    let all = clean(&all).map_err(bad)?;
    let (ext, width, height) = kind(&all).map_err(bad)?;
    let path = write(dir, &all, ext, wall).map_err(|_| ("failed", "The helper could not store the image.".to_string()))?;
    if st.held.len() >= MAX_HELD {
        let (_, old) = st.held.remove(0);
        let _ = std::fs::remove_file(old);
    }
    st.held.push((name.into(), path));
    let keep: Vec<PathBuf> = st.held.iter().map(|h| h.1.clone()).collect();
    purge(dir, wall, &keep);
    Ok(Some(json!({"type": "attached", "upload": name, "width": width, "height": height, "bytes": all.len()})))
}

/// The files of held uploads `names`, in order; else why nothing can be sent.
pub fn files(names: &[String]) -> Result<Vec<PathBuf>, String> {
    if names.len() > 4 {
        return Err("At most 4 images per message.".into());
    }
    if names.iter().enumerate().any(|(i, n)| names[..i].contains(n)) {
        return Err("An image is named twice.".into());
    }
    let st = state();
    names
        .iter()
        .map(|n| st.held.iter().find(|h| &h.0 == n).map(|h| h.1.clone()).filter(|p| p.is_file()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "Unknown image. Attach it again. Nothing was sent.".into())
}

/// Sent: the uploads are no longer held (their files stay for the agent; the purge removes them later).
pub fn sent(names: &[String]) {
    state().held.retain(|h| !names.contains(&h.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(own("phone-20261105-101500-a1b2c3.png"));
        assert!(own("phone-20261105-101500-a1b2c3.webp"));
        assert!(!own("phone-20261105-101500-a1b2c3.gif"));
        assert!(!own("paste-20261105-101500-a1b2c3.png"));
        assert!(!own("phone-20261105-101500-a1b2c.png"));
        assert!(!own("notes.png"));
        assert!(name_ok("img-1_A") && !name_ok("") && !name_ok("../x") && !name_ok(&"a".repeat(33)));
    }

    #[test]
    fn the_purge_removes_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let old = now - KEEP - Duration::from_secs(60);
        let mk = |n: &str, at: SystemTime| {
            let p = dir.path().join(n);
            std::fs::write(&p, b"x").unwrap();
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(at).unwrap();
            p
        };
        let a = mk("phone-20261020-101500-aaaaaa.png", old);
        let b = mk("phone-20261020-101500-bbbbbb.jpg", old);
        let c = mk("phone-20261105-101500-cccccc.png", now);
        let other = mk("notes.png", old);
        purge(dir.path(), now, std::slice::from_ref(&b));
        assert!(!a.exists(), "old and its own");
        assert!(b.exists(), "held");
        assert!(c.exists(), "new");
        assert!(other.exists(), "not its own");
    }

    #[test]
    fn kinds() {
        let png = |w: u32, h: u32| [&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13][..], b"IHDR", &w.to_be_bytes(), &h.to_be_bytes()].concat();
        assert_eq!(kind(&png(3, 4)), Ok(("png", 3, 4)));
        assert_eq!(kind(&[0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x30, 0x00, 0x40, 0x03, 0, 0, 0, 0, 0]), Ok(("jpg", 64, 48)));
        assert_eq!(kind(&[&[0, 0, 0, 24][..], b"ftypmif1", &[0u8; 8]].concat()), Err(HEIC));
        assert!(kind(b"GIF89a\x0a\x00\x14\x00").is_err());
        assert!(kind(&png(12_001, 1)).is_err());
    }
}
