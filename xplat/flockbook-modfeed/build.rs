// Embeds the herdr-app mod (layout, themes, mods/flockbook, keys.json) for `flockbook-modfeed install`, so
// the helper alone can install the mod. The files are the repo's herdr-app/ folder at build time.
use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())).map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if p.is_dir() {
            walk(&p, base, out);
            continue;
        }
        // herdr-app writes jsconfig.json and *.d.ts at a mod's top level (herdr-app/.gitignore); never ours.
        let generated = p.parent() == Some(base.join("mods/flockbook").as_path()) && (name == "jsconfig.json" || name.ends_with(".d.ts"));
        if name == ".DS_Store" || generated {
            continue;
        }
        let rel = p.strip_prefix(base).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        out.push((rel, p));
    }
}

fn main() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../herdr-app");
    assert!(base.join("keys.json").is_file(), "herdr-app/ must sit next to xplat/");
    let mut files = vec![("layouts/flockbook.json".to_string(), base.join("layouts/flockbook.json"))];
    let themes = base.join("themes");
    println!("cargo:rerun-if-changed={}", themes.display());
    let mut names: Vec<_> = fs::read_dir(&themes).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    names.sort();
    for p in names {
        files.push((format!("themes/{}", p.file_name().unwrap().to_string_lossy()), p));
    }
    walk(&base.join("mods/flockbook"), &base, &mut files);
    files.sort();
    let mut src = String::from("/// Every file `install` puts in the home, as (path relative to the home, bytes), sorted by path.\npub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, p) in &files {
        println!("cargo:rerun-if-changed={}", p.display());
        src.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", p.display().to_string()));
    }
    src.push_str("];\n");
    let keys = base.join("keys.json");
    println!("cargo:rerun-if-changed={}", keys.display());
    src.push_str(&format!("/// herdr-app/keys.json.\npub static KEYS: &[u8] = include_bytes!({:?});\n", keys.display().to_string()));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let known = known_files(&out);
    src.push_str(&format!("/// The files of every earlier Flockbook (scripts/known-files.mjs): `<sha256>  <version>  <path>` lines.\npub static KNOWN: &[u8] = include_bytes!({:?});\n", known.display().to_string()));
    fs::write(out.join("embedded.rs"), src).unwrap();
}

/// The known-files table: FLOCKBOOK_KNOWN_FILES when set (a file made by the script), else the script run here
/// on this checkout's git history. Without node or the history, an empty table (a warning), unless
/// FLOCKBOOK_REQUIRE_KNOWN_FILES=1 (the release's helpers): then the build fails.
fn known_files(out: &Path) -> PathBuf {
    println!("cargo:rerun-if-env-changed=FLOCKBOOK_KNOWN_FILES");
    println!("cargo:rerun-if-env-changed=FLOCKBOOK_REQUIRE_KNOWN_FILES");
    if let Ok(p) = std::env::var("FLOCKBOOK_KNOWN_FILES") {
        let p = PathBuf::from(p);
        println!("cargo:rerun-if-changed={}", p.display());
        assert!(p.is_file(), "FLOCKBOOK_KNOWN_FILES={} is not a file", p.display());
        return p;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/known-files.mjs");
    println!("cargo:rerun-if-changed={}", script.display());
    let dest = out.join("known-files.txt");
    let made = std::process::Command::new("node").arg(&script).arg("--out").arg(&dest).output();
    match made {
        Ok(o) if o.status.success() => dest,
        other => {
            let why = match other {
                Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
                Err(e) => format!("cannot run node: {e}"),
            };
            if std::env::var("FLOCKBOOK_REQUIRE_KNOWN_FILES").as_deref() == Ok("1") {
                panic!("the known-files table: {why}");
            }
            println!("cargo:warning=no known-files table ({why}); this helper treats every older Flockbook file as the person's");
            fs::write(&dest, "").unwrap();
            dest
        }
    }
}
