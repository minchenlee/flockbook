//! paste-image: what a ⌘V in the reply box attaches. Copied files win, then an image (PNG, or any image type
//! turned into PNG); text alone is the box's own paste. macOS tests use a private named pasteboard, never the
//! clipboard of the person running them. Linux tests give the reader the clipboard's types and data.

use modfeedkit::paste::{self, Clip};
use std::collections::HashMap;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-rest";

fn linux(offer: &[(&str, &[u8])]) -> Clip {
    let m: HashMap<String, Vec<u8>> = offer.iter().map(|(t, d)| (t.to_string(), d.to_vec())).collect();
    let types = offer.iter().map(|(t, _)| *t).collect::<Vec<_>>().join("\n").into_bytes();
    paste::linux_clip(|t| match t {
        None => Some(types.clone()),
        Some(t) => m.get(t).cloned(),
    })
}

#[test]
fn uri_list_gives_local_paths() {
    let l = "# from the file manager\r\nfile:///home/me/Shot%201.png\r\nfile://localhost/tmp/notes.txt\nfile://other/x\nhttps://example.com/a.png\n\n";
    assert_eq!(paste::uri_list(l), vec!["/home/me/Shot 1.png", "/tmp/notes.txt"]);
    assert_eq!(paste::uri_list("file:///tmp/%E6%97%A5.png"), vec!["/tmp/日.png"]);
    assert!(paste::uri_list("file:///tmp/%FF").is_empty(), "not UTF-8: skipped");
    assert!(paste::uri_list("just text").is_empty());
}

#[test]
fn linux_files_win_over_image_and_text() {
    // A file manager copy: the file list, its names as text.
    let c = linux(&[("text/uri-list", b"file:///home/me/a.png\nfile:///home/me/b%20c.txt\n"), ("UTF8_STRING", b"a.png")]);
    assert_eq!(c, Clip::Files(vec!["/home/me/a.png".into(), "/home/me/b c.txt".into()]));
    // GNOME's own list.
    let c = linux(&[("x-special/gnome-copied-files", b"copy\nfile:///home/me/a.png"), ("text/plain", b"a.png")]);
    assert_eq!(c, Clip::Files(vec!["/home/me/a.png".into()]));
    // Files and an image of them: the files.
    let c = linux(&[("image/png", PNG), ("text/uri-list", b"file:///home/me/a.png")]);
    assert_eq!(c, Clip::Files(vec!["/home/me/a.png".into()]));
}

#[test]
fn linux_image_wins_over_text() {
    // A screenshot tool: PNG only.
    assert_eq!(linux(&[("image/png", PNG)]), Clip::Image(PNG.to_vec()));
    // A browser image: PNG, HTML and text.
    let c = linux(&[("text/html", b"<img src=x>"), ("UTF8_STRING", b"x"), ("image/png", PNG)]);
    assert_eq!(c, Clip::Image(PNG.to_vec()));
    // Text only, and data that is not PNG: nothing to attach.
    assert_eq!(linux(&[("UTF8_STRING", b"hello"), ("text/plain", b"hello")]), Clip::Nothing);
    assert_eq!(linux(&[("image/png", b"not a png")]), Clip::Nothing);
    assert_eq!(linux(&[]), Clip::Nothing);
    // A list of web links is text.
    assert_eq!(linux(&[("text/uri-list", b"https://example.com/a.png")]), Clip::Nothing);
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::process::Command;

    /// A private pasteboard filled by AppKit, as the source app would fill the clipboard. Released on drop.
    struct Board(String);
    impl Board {
        /// `what`: JXA statements run with `pb` (the pasteboard), `png`, `tiff` (a 4×4 image) and `dir` in scope.
        fn new(tag: &str, what: &str) -> Board {
            let name = format!("dev.flockbook.test.{tag}.{}", std::process::id());
            let script = format!(
                r#"ObjC.import("AppKit");
function run(argv) {{
  const pb = $.NSPasteboard.pasteboardWithName(argv[0]); pb.clearContents;
  const rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(null, 4, 4, 8, 4, true, false, "NSDeviceRGBColorSpace", 0, 0);
  const png = rep.representationUsingTypeProperties(4, $({{}}));
  const tiff = rep.TIFFRepresentation;
  const dir = argv[1];
  {what}
  return "ok";
}}"#
            );
            let o = Command::new("osascript").args(["-l", "JavaScript", "-e", &script, &name, env!("CARGO_TARGET_TMPDIR")]).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ok", "{}", String::from_utf8_lossy(&o.stderr));
            Board(name)
        }
        fn clip(&self) -> Clip {
            paste::mac_clip(Some(&self.0))
        }
    }
    impl Drop for Board {
        fn drop(&mut self) {
            let s = r#"ObjC.import("AppKit"); function run(a) { $.NSPasteboard.pasteboardWithName(a[0]).releaseGlobally; }"#;
            let _ = Command::new("osascript").args(["-l", "JavaScript", "-e", s, &self.0]).output();
        }
    }

    fn png_of(c: Clip) -> Vec<u8> {
        match c {
            Clip::Image(b) => b,
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn screenshot_png() {
        // ⌘⌃⇧4: PNG data, no text.
        let b = Board::new("png", r#"pb.setDataForType(png, "public.png");"#);
        let got = png_of(b.clip());
        assert!(got.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn tiff_only_becomes_png() {
        let b = Board::new("tiff", r#"pb.setDataForType(tiff, "public.tiff");"#);
        assert!(png_of(b.clip()).starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn finder_files_not_their_names() {
        // Finder: file URLs, the names as text, and the icon as TIFF.
        let b = Board::new(
            "finder",
            r#"const a = $.NSURL.fileURLWithPath(dir + "/Shot 1.png"), c = $.NSURL.fileURLWithPath(dir + "/notes.txt");
  pb.writeObjects($([a, c]));
  pb.addTypesOwner($(["public.utf8-plain-text", "public.tiff"]), null);
  pb.setStringForType("Shot 1.png\rnotes.txt", "public.utf8-plain-text");
  pb.setDataForType(tiff, "public.tiff");"#,
        );
        let dir = env!("CARGO_TARGET_TMPDIR");
        assert_eq!(b.clip(), Clip::Files(vec![format!("{dir}/Shot 1.png"), format!("{dir}/notes.txt")]));
    }

    #[test]
    fn browser_image_wins_over_its_text() {
        // A browser's "Copy Image": TIFF, HTML and the address as text.
        let b = Board::new(
            "browser",
            r#"pb.declareTypesOwner($(["public.html", "public.utf8-plain-text", "public.tiff"]), null);
  pb.setStringForType("<img src=\"https://example.com/a.png\">", "public.html");
  pb.setStringForType("https://example.com/a.png", "public.utf8-plain-text");
  pb.setDataForType(tiff, "public.tiff");"#,
        );
        assert!(png_of(b.clip()).starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn text_only_is_nothing() {
        let b = Board::new("text", r#"pb.setStringForType("hello", "public.utf8-plain-text");"#);
        assert_eq!(b.clip(), Clip::Nothing);
        let b = Board::new("link", r#"pb.writeObjects($([$.NSURL.URLWithString("https://example.com/a.png")]));"#);
        assert_eq!(b.clip(), Clip::Nothing, "a web link is text");
    }
}
