//! `flockbook-modfeed install` over an older Flockbook that has no install.json (a copy from the Herdr layout
//! gallery, or an older zip): the files come from the release tags in git, so this needs the tags
//! (CI checks out with fetch-depth 0). Temp homes only.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files a release put in a home, as (path in the home, bytes), read from its tag. `folder` is the mod's
/// folder at that tag (herdr-mod/ in v0.7.0).
fn release(tag: &str, folder: &str, with_themes: bool) -> Vec<(String, Vec<u8>)> {
    let ls = Command::new("git").args(["ls-tree", "-r", "--name-only", tag, &format!("{folder}/")]).current_dir(repo()).output().unwrap();
    assert!(ls.status.success(), "git ls-tree {tag}: {} (the tests need the release tags: git fetch --tags)", String::from_utf8_lossy(&ls.stderr));
    let mut out = Vec::new();
    for name in String::from_utf8(ls.stdout).unwrap().lines() {
        let rel = &name[folder.len() + 1..];
        let take = rel == "layouts/flockbook.json" || rel.starts_with("mods/flockbook/") || (with_themes && rel.starts_with("themes/") && rel.ends_with(".json"));
        if !take {
            continue;
        }
        let show = Command::new("git").args(["show", &format!("{tag}:{name}")]).current_dir(repo()).output().unwrap();
        assert!(show.status.success());
        out.push((rel.to_string(), show.stdout));
    }
    assert!(out.iter().any(|(r, _)| r == "layouts/flockbook.json"), "{tag} has no layout under {folder}/");
    out
}

struct Home {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    app: PathBuf,
}

fn home_with(files: &[(String, Vec<u8>)]) -> Home {
    let tmp = tempfile::Builder::new().prefix("fb-upgrade-").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let app = root.join("app");
    for (rel, bytes) in files {
        let p = app.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    fs::create_dir_all(root.join("user")).unwrap();
    Home { _tmp: tmp, root, app }
}

impl Home {
    fn install(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
            .arg("install")
            .args(["--home", self.app.to_str().unwrap()])
            .args(extra)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root.join("user"))
            .output()
            .unwrap()
    }
}

fn both(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
}

/// What this version installs, from the repo's herdr-app/.
fn current(rel: &str) -> Vec<u8> {
    fs::read(repo().join("herdr-app").join(rel)).unwrap()
}

#[test]
fn a_gallery_copy_of_v0_7_0_is_upgraded_and_the_plan_says_so() {
    let old = release("v0.7.0", "herdr-mod", false);
    let h = home_with(&old);
    let dry = h.install(&["--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", both(&dry));
    let plan = String::from_utf8_lossy(&dry.stdout).into_owned();
    assert!(plan.contains("older Flockbook (v0.7.0)"), "{plan}");
    assert!(plan.contains("layouts/flockbook.json"), "{plan}");
    // Dry run: still the old bytes.
    assert_eq!(fs::read(h.app.join("layouts/flockbook.json")).unwrap(), old.iter().find(|(r, _)| r == "layouts/flockbook.json").unwrap().1);

    let r = h.install(&[]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    for rel in ["layouts/flockbook.json", "mods/flockbook/chat.js", "mods/flockbook/mod.json", "themes/flockbook.json"] {
        assert_eq!(fs::read(h.app.join(rel)).unwrap(), current(rel), "{rel}");
    }
    assert!(h.app.join("flockbook/install.json").is_file());
}

#[test]
fn a_v0_8_0_home_without_install_json_is_upgraded() {
    let old = release("v0.8.0", "herdr-app", true);
    let h = home_with(&old);
    let dry = h.install(&["--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", both(&dry));
    let plan = String::from_utf8_lossy(&dry.stdout).into_owned();
    // Each file is named by the first release that had it: chat.js changed in v0.8.0, the layout did not.
    assert!(plan.contains("mods/flockbook/chat.js: older Flockbook (v0.8.0), replaced"), "{plan}");
    assert!(plan.contains("layouts/flockbook.json: older Flockbook (v0.7.0), replaced"), "{plan}");
    let r = h.install(&[]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    for (rel, _) in &old {
        if repo().join("herdr-app").join(rel).is_file() {
            assert_eq!(fs::read(h.app.join(rel)).unwrap(), current(rel), "{rel}");
        }
    }
}

#[test]
fn an_older_file_the_person_changed_still_stops_everything() {
    let mut old = release("v0.7.0", "herdr-mod", false);
    for (rel, bytes) in old.iter_mut() {
        if rel == "mods/flockbook/chat.js" {
            bytes.extend_from_slice(b"\n// mine\n");
        }
    }
    let h = home_with(&old);
    let r = h.install(&[]);
    assert_eq!(r.status.code(), Some(1), "{}", both(&r));
    let e = String::from_utf8_lossy(&r.stderr).into_owned();
    assert!(e.contains("mods/flockbook/chat.js"), "{e}");
    assert!(!e.contains("layouts/flockbook.json"), "only the changed file is the person's: {e}");
    // Nothing was written: the layout is still v0.7.0's.
    assert_eq!(fs::read(h.app.join("layouts/flockbook.json")).unwrap(), old.iter().find(|(r, _)| r == "layouts/flockbook.json").unwrap().1);
}

#[test]
fn a_file_of_the_persons_own_beside_an_older_flockbook_still_stops() {
    let mut old = release("v0.7.0", "herdr-mod", false);
    old.push(("mods/flockbook/mine.js".into(), b"export const mine = 1;\n".to_vec()));
    let h = home_with(&old);
    let r = h.install(&[]);
    assert_eq!(r.status.code(), Some(1), "{}", both(&r));
    assert!(String::from_utf8_lossy(&r.stderr).contains("mods/flockbook/mine.js"));
}

// The sorting on its own, with a made-up table: files a newer version dropped, and paths that are never ours.
mod sorting {
    use modfeedkit::install::{known_files, sort_files};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::fs;

    fn sha(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn dropped_files_of_an_older_version_are_removed_only_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let put = |rel: &str, b: &[u8]| {
            fs::create_dir_all(home.join(rel).parent().unwrap()).unwrap();
            fs::write(home.join(rel), b).unwrap();
        };
        put("mods/flockbook/old.js", b"old\n");
        put("mods/flockbook/edited.js", b"edited by the person\n");
        put("themes/flockbook-old.json", b"{}\n");
        put("themes/theirs.json", b"{\"mine\":1}\n");
        put("layouts/flockbook.json", b"{\"v\":7}\n");
        // A path outside what Flockbook owns, even with a matching hash, is never touched.
        put("keys-backup/flockbook.json", b"x\n");
        let table = format!(
            "# comment\n{}  v0.7.0  mods/flockbook/old.js\n{}  v0.7.0  mods/flockbook/edited.js\n{}  v0.7.0  themes/flockbook-old.json\n{}  v0.7.0  layouts/flockbook.json\n{}  v0.7.0  keys-backup/flockbook.json\n{}  v0.7.0  mods/flockbook/../../escape.js\nnot a line\n",
            sha(b"old\n"),
            sha(b"edited by the first version\n"),
            sha(b"{}\n"),
            sha(b"{\"v\":7}\n"),
            sha(b"x\n"),
            sha(b"x\n"),
        );
        let known = known_files(table.as_bytes());
        let files: BTreeMap<&str, &[u8]> = [("layouts/flockbook.json", &b"{\"v\":8}\n"[..]), ("mods/flockbook/main.js", &b"new\n"[..])].into_iter().collect();
        let s = sort_files(home, &files, &BTreeMap::new(), &known);
        assert_eq!(s.older.into_iter().collect::<Vec<_>>(), vec![("layouts/flockbook.json".into(), "v0.7.0".into())]);
        assert_eq!(
            s.gone.into_iter().collect::<Vec<_>>(),
            vec![("mods/flockbook/old.js".into(), "v0.7.0".into()), ("themes/flockbook-old.json".into(), "v0.7.0".into())]
        );
        assert_eq!(s.clash.into_iter().collect::<Vec<_>>(), vec!["mods/flockbook/edited.js".to_string()]);
    }

    #[test]
    fn an_empty_or_bad_table_knows_nothing() {
        assert!(known_files(b"").is_empty());
        assert!(known_files(b"zz  v0.7.0  layouts/flockbook.json\n# x\n").is_empty());
    }
}
