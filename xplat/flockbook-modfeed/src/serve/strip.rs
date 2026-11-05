//! Metadata out of an uploaded picture, without decoding it (the helper does not re-encode, as the Mac hub does).
//! - JPEG: every APP1 segment (EXIF, XMP) goes. When the EXIF had an Orientation other than 1, a minimal APP1 with
//!   only that tag comes back, so a photo taken upright still shows upright. An EXIF that cannot be read just goes.
//! - PNG: the eXIf, tEXt, iTXt and zTXt chunks go.
//! - WebP: the EXIF and "XMP " chunks go, and VP8X no longer says it has them.
//!
//! A file whose own structure (segments, chunks) is broken is refused: it is not sent half cleaned.

pub const BROKEN: &str = "The image could not be read.";

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

/// The EXIF Orientation (tag 0x0112 in IFD0) of an APP1 payload ("Exif\0\0" + TIFF); None when there is none or
/// it cannot be read.
pub fn orientation(app1: &[u8]) -> Option<u16> {
    let t = app1.strip_prefix(b"Exif\0\0")?;
    let le = match t.get(0..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b: [u8; 2] = t.get(i..i + 2)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(i..i + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for k in 0..n.min(512) {
        let e = ifd.checked_add(2 + 12 * k)?;
        if u16_at(e)? == 0x0112 {
            // SHORT, count 1: the value sits in the entry.
            if u16_at(e + 2)? != 3 || u32_at(e + 4)? != 1 {
                return None;
            }
            let v = u16_at(e + 8)?;
            return (1..=8).contains(&v).then_some(v);
        }
    }
    None
}

/// An APP1 segment (marker included) with only IFD0's Orientation.
pub fn orientation_app1(v: u16) -> Vec<u8> {
    let mut p = b"Exif\0\0MM\0*".to_vec();
    p.extend(8u32.to_be_bytes()); // IFD0 right after the header
    p.extend(1u16.to_be_bytes()); // one entry
    p.extend(0x0112u16.to_be_bytes());
    p.extend(3u16.to_be_bytes()); // SHORT
    p.extend(1u32.to_be_bytes()); // count
    p.extend(v.to_be_bytes());
    p.extend([0, 0]);
    p.extend(0u32.to_be_bytes()); // no next IFD
    let mut s = vec![0xff, 0xe1];
    s.extend(((p.len() + 2) as u16).to_be_bytes());
    s.extend(p);
    s
}

pub fn jpeg(b: &[u8]) -> Result<Vec<u8>, &'static str> {
    if b.get(0..2) != Some(&[0xff, 0xd8]) {
        return Err(BROKEN);
    }
    let mut out = vec![0xff, 0xd8];
    // Where the Orientation segment goes: after SOI and a JFIF APP0.
    let mut put_at = 2;
    let mut turn: Option<u16> = None;
    let mut first = true;
    let mut i = 2;
    loop {
        if b.get(i) != Some(&0xff) {
            return Err(BROKEN);
        }
        let m = *b.get(i + 1).ok_or(BROKEN)?;
        if m == 0xff {
            i += 1; // fill byte
            continue;
        }
        if m == 0xd9 {
            out.extend(&b[i..i + 2]);
            break;
        }
        if (0xd0..=0xd7).contains(&m) || m == 0x01 {
            out.extend(&b[i..i + 2]);
            i += 2;
            continue;
        }
        let len = be16(b, i + 2).ok_or(BROKEN)? as usize;
        if len < 2 || i + 2 + len > b.len() {
            return Err(BROKEN);
        }
        let seg = &b[i..i + 2 + len];
        if m == 0xda {
            // Start of scan: the rest is image data (and any later segments, which carry no metadata we know of).
            out.extend(&b[i..]);
            break;
        }
        if m == 0xe1 {
            if turn.is_none() {
                turn = orientation(&seg[4..]).filter(|v| *v != 1);
            }
        } else {
            out.extend(seg);
            if first && m == 0xe0 {
                put_at = out.len();
            }
        }
        first = false;
        i += 2 + len;
    }
    if let Some(v) = turn {
        out.splice(put_at..put_at, orientation_app1(v));
    }
    Ok(out)
}

pub fn png(b: &[u8]) -> Result<Vec<u8>, &'static str> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !b.starts_with(SIG) {
        return Err(BROKEN);
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
    loop {
        let len = u32::from_be_bytes(b.get(i..i + 4).ok_or(BROKEN)?.try_into().map_err(|_| BROKEN)?) as usize;
        let end = i.checked_add(12).and_then(|x| x.checked_add(len)).ok_or(BROKEN)?;
        if end > b.len() {
            return Err(BROKEN);
        }
        let ty = &b[i + 4..i + 8];
        if ![&b"eXIf"[..], b"tEXt", b"iTXt", b"zTXt"].contains(&ty) {
            out.extend(&b[i..end]);
        }
        i = end;
        if ty == b"IEND" {
            return Ok(out);
        }
    }
}

pub fn webp(b: &[u8]) -> Result<Vec<u8>, &'static str> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return Err(BROKEN);
    }
    let size = u32::from_le_bytes(b[4..8].try_into().map_err(|_| BROKEN)?) as usize;
    let end = size.checked_add(8).filter(|e| *e <= b.len()).ok_or(BROKEN)?;
    let mut body = b"WEBP".to_vec();
    let mut i = 12;
    while i < end {
        let len = u32::from_le_bytes(b.get(i + 4..i + 8).ok_or(BROKEN)?.try_into().map_err(|_| BROKEN)?) as usize;
        let next = (i + 8).checked_add(len).ok_or(BROKEN)?;
        if next > end {
            return Err(BROKEN);
        }
        let padded = (next + (len & 1)).min(end);
        let ty = &b[i..i + 4];
        if ty != b"EXIF" && ty != b"XMP " {
            let at = body.len();
            body.extend(&b[i..padded]);
            if ty == b"VP8X" && len >= 1 {
                body[at + 8] &= !(0x08 | 0x04); // the EXIF and XMP flags
            }
        }
        i = padded;
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(m: u8, p: &[u8]) -> Vec<u8> {
        let mut s = vec![0xff, m];
        s.extend(((p.len() + 2) as u16).to_be_bytes());
        s.extend(p);
        s
    }

    /// EXIF, little-endian: IFD0 with Make, Orientation `turn`, and a GPS IFD pointer; then the GPS IFD (latitude ref).
    fn exif(turn: u16) -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        t.extend(3u16.to_le_bytes());
        let entry = |tag: u16, ty: u16, n: u32, v: [u8; 4]| [&tag.to_le_bytes()[..], &ty.to_le_bytes(), &n.to_le_bytes(), &v].concat();
        t.extend(entry(0x010f, 2, 4, *b"Cam\0"));
        t.extend(entry(0x0112, 3, 1, [turn as u8, 0, 0, 0]));
        let gps = 8 + 2 + 36 + 4;
        t.extend(entry(0x8825, 4, 1, (gps as u32).to_le_bytes()));
        t.extend(0u32.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        t.extend(entry(0x0001, 2, 2, *b"N\0\0\0"));
        t.extend(0u32.to_le_bytes());
        t.extend(b"GPSDATA-52.37N-4.89E");
        [b"Exif\0\0".as_slice(), &t].concat()
    }

    fn photo(app1: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
        let mut b = vec![0xff, 0xd8];
        b.extend(seg(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        for a in app1 {
            b.extend(seg(0xe1, a));
        }
        b.extend(seg(0xdb, &[0u8; 65]));
        b.extend(seg(0xc0, &[8, 0, 48, 0, 64, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]));
        let scan = [seg(0xda, &[3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3f, 0]), vec![0x12, 0x34, 0xff, 0x00, 0x56, 0xff, 0xd9]].concat();
        b.extend(&scan);
        (b, scan)
    }

    fn app1s(b: &[u8]) -> Vec<Vec<u8>> {
        let mut out = vec![];
        let mut i = 2;
        while b[i + 1] != 0xda {
            let len = be16(b, i + 2).unwrap() as usize;
            if b[i + 1] == 0xe1 {
                out.push(b[i + 4..i + 2 + len].to_vec());
            }
            i += 2 + len;
        }
        out
    }

    #[test]
    fn jpeg_keeps_only_the_turn() {
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPS 52.37</x:xmpmeta>".to_vec();
        let (b, scan) = photo(&[exif(6), xmp]);
        assert_eq!(orientation(&exif(6)), Some(6));
        let out = jpeg(&b).unwrap();
        assert!(!out.windows(3).any(|w| w == b"GPS"), "no GPS left");
        assert!(!out.windows(3).any(|w| w == b"Cam"), "no camera");
        let a = app1s(&out);
        assert_eq!(a.len(), 1);
        assert_eq!(orientation(&a[0]), Some(6), "the turn stays");
        assert!(out.ends_with(&scan), "the image data is untouched");
        assert_eq!(&out[2..4], &[0xff, 0xe0], "JFIF stays first");
        assert_eq!(herdr_core::log_image::header(&out).map(|h| (h.1, h.2)), Some((64, 48)));
    }

    #[test]
    fn jpeg_upright_or_broken_exif_goes_whole() {
        for app1 in [exif(1), b"Exif\0\0II*\0\xff\xff\xff\xffGPS".to_vec(), b"Exif\0\0garbage GPS".to_vec(), b"Exif\0".to_vec()] {
            let (b, scan) = photo(&[app1]);
            let out = jpeg(&b).unwrap();
            assert!(app1s(&out).is_empty());
            assert!(out.ends_with(&scan));
        }
        let (b, _) = photo(&[]);
        assert_eq!(jpeg(&b).unwrap(), b, "nothing to take out");
        let (mut cut, _) = photo(&[exif(6)]);
        cut.truncate(30);
        assert_eq!(jpeg(&cut), Err(BROKEN));
    }

    fn chunk(ty: &[u8], d: &[u8]) -> Vec<u8> {
        [&(d.len() as u32).to_be_bytes()[..], ty, d, &[0, 0, 0, 0]].concat()
    }

    #[test]
    fn png_text_chunks_go() {
        let ihdr = chunk(b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 4, 8, 6, 0, 0, 0]);
        let idat = chunk(b"IDAT", &[1, 2, 3]);
        let iend = chunk(b"IEND", &[]);
        let sig = b"\x89PNG\r\n\x1a\n".to_vec();
        let b = [sig.clone(), ihdr.clone(), chunk(b"tEXt", b"GPS\0here"), chunk(b"eXIf", &exif(6)[6..]), chunk(b"iTXt", b"x"), chunk(b"zTXt", b"y"), idat.clone(), iend.clone()].concat();
        assert_eq!(png(&b).unwrap(), [sig.clone(), ihdr.clone(), idat.clone(), iend.clone()].concat());
        let mut bad = [sig, ihdr].concat();
        bad.extend([0xff, 0, 0, 0]);
        bad.extend(b"IDAT");
        assert_eq!(png(&bad), Err(BROKEN));
    }

    #[test]
    fn webp_exif_and_xmp_go() {
        let ch = |ty: &[u8], d: &[u8]| {
            let mut c = [ty, &(d.len() as u32).to_le_bytes()[..], d].concat();
            if d.len() % 2 == 1 {
                c.push(0);
            }
            c
        };
        let riff = |chunks: &[Vec<u8>]| {
            let body = [b"WEBP".to_vec(), chunks.concat()].concat();
            [b"RIFF".to_vec(), (body.len() as u32).to_le_bytes().to_vec(), body].concat()
        };
        let vp8x = |flags: u8| ch(b"VP8X", &[flags, 0, 0, 0, 9, 0, 0, 4, 0, 0]);
        let data = ch(b"VP8L", &[0x2f, 1, 2]);
        let b = riff(&[vp8x(0x08 | 0x04 | 0x10), data.clone(), ch(b"EXIF", &exif(6)[6..]), ch(b"XMP ", b"GPS")]);
        let out = webp(&b).unwrap();
        assert_eq!(out, riff(&[vp8x(0x10), data]));
        assert_eq!(herdr_core::log_image::header(&out).map(|h| (h.1, h.2)), Some((10, 5)));
        let mut bad = b.clone();
        bad[4] = 0xff;
        assert_eq!(webp(&bad), Err(BROKEN));
    }
}
