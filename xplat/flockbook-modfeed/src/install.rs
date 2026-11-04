//! `flockbook-modfeed install`: the port of herdr-app/install.sh, with the mod built into the helper.
//! Plain text for a person, and an exit code (0 done, 1 stopped, 2 bad arguments); not JSON Lines.

use indexmap::IndexMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}
pub use embedded::{FILES, KEYS};

const RELEASES: &str = "https://github.com/minchenlee/flockbook/releases";
const HELP: &str = "\
Installs the Flockbook layout, mod, themes, keys and helper into a herdr-app home.
Usage: flockbook-modfeed install [--home <herdr-app home>] [--helper <flockbook-modfeed>] [--remote <folder>] [--dry-run]
 - The home is --home, else $HERDR_APP_HOME, else ~/.config/herdr-app. With no HOME, give --home.
 - A keys.json, layout or theme that is a symlink stays one; the file it points to is written.
 - It never overwrites what you have: if a layout, mod or theme with the same name is there and
   this did not put it there (or you changed it since), it stops and writes nothing.
 - keys.json is merged: your sections and keys stay; a key you already use keeps your command.
 - --helper copies flockbook-modfeed to <home>/flockbook/bin/. Without it, a helper already there
   stays; with none there, it says where to get one.
 - --remote copies the helpers for other machines (<folder>/<target>/flockbook-modfeed, checked
   against <folder>/SHA256SUMS) to <home>/flockbook/remote/.
 - After copying it runs `herdr-app check` on the mod. --dry-run prints the plan and writes nothing.
 - What it installed (with hashes) is kept in <home>/flockbook/install.json, so a later install
   can update its own files.
";

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_file(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| sha(&b))
}

fn stop(lines: &[String]) -> i32 {
    for l in lines {
        eprintln!("{l}");
    }
    1
}

/// Writes through a temp file beside `dest` and a rename, so a reader never sees half a file.
/// When `dest` is a symlink (a keys.json kept in a dotfiles folder), the file it points to is written,
/// in that file's own folder, and the link stays.
fn put(dest: &Path, bytes: &[u8], exe: bool) -> std::io::Result<()> {
    let real;
    let dest = if fs::symlink_metadata(dest).is_ok_and(|m| m.file_type().is_symlink()) {
        // A link to a file that is not there yet: create it where the link points.
        real = match fs::canonicalize(dest) {
            Ok(p) => p,
            Err(_) => dest.parent().unwrap_or(Path::new(".")).join(fs::read_link(dest)?),
        };
        real.as_path()
    } else {
        dest
    };
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".flockbook-new");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    if exe {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = exe;
    fs::rename(&tmp, dest)
}

fn rel_of(p: &Path, base: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// One helper for another machine: its target name and the bytes, already checked against SHA256SUMS.
struct Remote {
    target: String,
    bytes: Vec<u8>,
}

fn valid_target(t: &str) -> bool {
    !t.is_empty() && !t.starts_with('.') && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// Reads <dir>/SHA256SUMS (`<sum>  <target>/flockbook-modfeed` per line) and checks every helper it names.
fn read_remote(dir: &Path) -> Result<(Vec<Remote>, Vec<u8>), String> {
    let sums_path = dir.join("SHA256SUMS");
    let sums = fs::read(&sums_path).map_err(|e| format!("STOP: cannot read {}: {e}", sums_path.display()))?;
    let text = String::from_utf8_lossy(&sums);
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let (Some(sum), Some(path), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("STOP: {} has a line I do not understand: {line}", sums_path.display()));
        };
        let path = path.trim_start_matches('*');
        let target = path.strip_suffix("/flockbook-modfeed").unwrap_or("");
        if !valid_target(target) || sum.len() != 64 || !sum.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("STOP: {} has a line I do not understand: {line}", sums_path.display()));
        }
        if out.iter().any(|r: &Remote| r.target == target) {
            return Err(format!("STOP: {} names {target} twice", sums_path.display()));
        }
        let file = dir.join(target).join("flockbook-modfeed");
        let bytes = fs::read(&file).map_err(|e| format!("STOP: the helper for {target} is missing ({}: {e})", file.display()))?;
        if sha(&bytes) != sum.to_ascii_lowercase() {
            return Err(format!("STOP: the helper for {target} ({}) does not match SHA256SUMS", file.display()));
        }
        out.push(Remote { target: target.to_string(), bytes });
    }
    if out.is_empty() {
        return Err(format!("STOP: {} names no helpers", sums_path.display()));
    }
    Ok((out, sums))
}

type Keys = IndexMap<String, IndexMap<String, Value>>;

pub fn run(args: &[String]) -> i32 {
    let mut home: Option<String> = None;
    let mut helper: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = |what: &str| -> Result<String, i32> {
            args.get(i + 1).cloned().ok_or_else(|| {
                eprintln!("{a} needs {what}");
                2
            })
        };
        match a {
            "--home" => match value("a folder") {
                Ok(v) => home = Some(v),
                Err(c) => return c,
            },
            "--helper" => match value("a file") {
                Ok(v) => helper = Some(v),
                Err(c) => return c,
            },
            "--remote" => match value("a folder") {
                Ok(v) => remote = Some(v),
                Err(c) => return c,
            },
            "--dry-run" => {
                dry = true;
                i += 1;
                continue;
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return 0;
            }
            _ => {
                eprintln!("unknown argument: {a} (see --help)");
                return 2;
            }
        }
        i += 2;
    }
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let home = match home.or_else(|| nonempty("HERDR_APP_HOME")) {
        Some(h) => PathBuf::from(h),
        None => match nonempty("HOME").or_else(|| nonempty("USERPROFILE")) {
            Some(h) => Path::new(&h).join(".config").join("herdr-app"),
            None => return stop(&["STOP: HOME is not set, so there is no ~/.config/herdr-app. Name the folder with --home <herdr-app home>. Nothing was written.".into()]),
        },
    };
    if let Some(h) = &helper {
        if !Path::new(h).is_file() {
            return stop(&[format!("STOP: no helper at {h}")]);
        }
    }
    let marker_path = home.join("flockbook").join("install.json");
    let files: BTreeMap<&str, &[u8]> = FILES.iter().copied().collect();

    let mine: BTreeMap<String, String> = fs::read(&marker_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("files").and_then(|f| f.as_object()).cloned())
        .map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect())
        .unwrap_or_default();

    // A file is ours to replace when it is missing, already the same, or exactly what we put there.
    let mut clash = BTreeSet::new();
    for (rel, bytes) in &files {
        let dest = home.join(rel);
        if !dest.exists() {
            continue;
        }
        let have = if dest.is_file() { sha_file(&dest) } else { None };
        match have {
            None => {
                clash.insert(rel.to_string());
            }
            Some(h) if h != sha(bytes) && mine.get(*rel) != Some(&h) => {
                clash.insert(rel.to_string());
            }
            _ => {}
        }
    }
    // Files in the person's mods/flockbook that we never put there also mean the folder is theirs.
    let mod_dest = home.join("mods").join("flockbook");
    if mod_dest.is_dir() {
        let mut found = Vec::new();
        walk_files(&mod_dest, &mut found);
        for p in found {
            let rel = rel_of(&p, &home);
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // herdr-app writes jsconfig.json and *.d.ts at the mod's top level (see .gitignore).
            let generated = p.parent() == Some(mod_dest.as_path()) && (n == "jsconfig.json" || n.ends_with(".d.ts"));
            if !files.contains_key(rel.as_str()) && !mine.contains_key(&rel) && n != ".DS_Store" && !generated {
                clash.insert(rel);
            }
        }
    } else if mod_dest.exists() {
        clash.insert("mods/flockbook".to_string());
    }
    if !clash.is_empty() {
        let mut lines = vec![format!("STOP: these are already in {} and were not installed by this script (or you changed them):", home.display())];
        lines.extend(clash.iter().map(|r| format!("  {r}")));
        lines.push("Nothing was written. Move or rename them, then run this again.".into());
        return stop(&lines);
    }

    // Keys: add our sections and keys; a key the person already set to another command stays theirs.
    let ours: Keys = serde_json::from_slice(KEYS).expect("herdr-app/keys.json");
    let keys_path = home.join("keys.json");
    let mut keys: Keys = match fs::read(&keys_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Keys::new(),
        Err(e) => return stop(&[format!("STOP: cannot read {}: {e}. Nothing was written.", keys_path.display())]),
        Ok(b) => match serde_json::from_slice::<Value>(&b) {
            Err(_) => return stop(&[format!("STOP: {} is not valid JSON. Nothing was written.", keys_path.display())]),
            Ok(v) => match serde_json::from_value::<Keys>(v) {
                Ok(k) => k,
                Err(_) => {
                    return stop(&[format!("STOP: {} is not sections of key = command. Nothing was written.", keys_path.display())])
                }
            },
        },
    };
    let old_keys = keys.clone();
    let mut kept = Vec::new();
    for (section, binds) in &ours {
        let have = keys.entry(section.clone()).or_default();
        for (key, cmd) in binds {
            match have.get(key) {
                Some(theirs) if theirs != cmd => kept.push((section.clone(), key.clone(), theirs.clone(), cmd.clone())),
                _ => {
                    have.insert(key.clone(), cmd.clone());
                }
            }
        }
    }

    let remote = match remote {
        None => None,
        Some(dir) => match read_remote(Path::new(&dir)) {
            Ok(r) => Some(r),
            Err(e) => return stop(&[e, "Nothing was written.".into()]),
        },
    };

    let bin_dest = home.join("flockbook").join("bin").join(if cfg!(windows) { "flockbook-modfeed.exe" } else { "flockbook-modfeed" });
    let show = |v: &Value| v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
    println!("{}{}", if dry { "Dry run: would install into " } else { "Installing into " }, home.display());
    for rel in files.keys().filter(|r| !r.starts_with("mods/")) {
        println!("  {rel}");
    }
    println!("  mods/flockbook/... ({} files)", files.keys().filter(|r| r.starts_with("mods/")).count());
    println!("  keys.json ({})", if old_keys.is_empty() { "new" } else { "merged" });
    for (section, key, theirs, cmd) in &kept {
        println!("    kept your {section} {key} = {} (Flockbook uses it for {})", show(theirs), show(cmd));
    }
    if let Some(h) = &helper {
        println!("  flockbook/bin/flockbook-modfeed (from {h})");
    }
    if let Some((list, _)) = &remote {
        for r in list {
            println!("  flockbook/remote/{}/flockbook-modfeed", r.target);
        }
        println!("  flockbook/remote/SHA256SUMS");
    }
    if dry {
        println!("Dry run: nothing was written; herdr-app check did not run.");
        return 0;
    }

    let written = (|| -> std::io::Result<()> {
        for (rel, bytes) in &files {
            put(&home.join(rel), bytes, false)?;
        }
        // Files an earlier install put there that this version no longer has, if still unchanged.
        for (rel, h) in &mine {
            let dest = home.join(rel);
            let own = (rel.starts_with("mods/flockbook/") || rel.starts_with("themes/flockbook")) && !rel.split('/').any(|c| c == "..");
            if own && !files.contains_key(rel.as_str()) && dest.is_file() && sha_file(&dest).as_ref() == Some(h) {
                fs::remove_file(&dest)?;
            }
        }
        if keys != old_keys {
            put(&keys_path, (serde_json::to_string_pretty(&keys).unwrap() + "\n").as_bytes(), false)?;
        }
        if let Some(h) = &helper {
            put(&bin_dest, &fs::read(h)?, true)?;
        }
        if let Some((list, sums)) = &remote {
            let dir = home.join("flockbook").join("remote");
            for r in list {
                put(&dir.join(&r.target).join("flockbook-modfeed"), &r.bytes, true)?;
            }
            put(&dir.join("SHA256SUMS"), sums, false)?;
        }
        let marker: BTreeMap<&str, String> = files.keys().map(|rel| (*rel, sha_file(&home.join(rel)).unwrap_or_default())).collect();
        put(&marker_path, (serde_json::to_string_pretty(&serde_json::json!({ "files": marker })).unwrap() + "\n").as_bytes(), false)
    })();
    if let Err(e) = written {
        return stop(&[format!("STOP: writing into {} failed: {e}", home.display())]);
    }

    if !bin_dest.is_file() {
        let me = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "flockbook-modfeed".into());
        println!("The helper flockbook-modfeed is not installed; the chat stays empty without it.");
        println!("This program is one: run this again with --helper {me}");
        println!("or get it from {RELEASES} (Flockbook-herdr-mod-<version>.zip, bin/flockbook-modfeed).");
    }

    let _ = std::io::stdout().flush();
    let status = std::process::Command::new("herdr-app").arg("check").arg(&mod_dest).env("HERDR_APP_HOME", &home).status();
    match status {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Installed, but herdr-app is not on PATH, so herdr-app check did not run.");
            0
        }
        Err(e) => stop(&[format!("herdr-app check did not run: {e}. The files are installed.")]),
        Ok(s) if !s.success() => stop(&["herdr-app check found a problem (see above). The files are installed.".into()]),
        Ok(_) => {
            println!("Installed. In herdr-app run: layout.switch {{\"name\": \"flockbook\"}}");
            0
        }
    }
}
